//! The axum router (DESIGN §7): a thin HTTP view over `wikid-core`. Success
//! bodies are the core result structs serialized directly; every route except
//! `GET /health` requires a configured bearer token.

use std::collections::BTreeMap;
use std::sync::Arc;

use anyhow::Context as _;
use axum::extract::rejection::{JsonRejection, QueryRejection};
use axum::extract::{Path, Query, Request, State};
use axum::http::header;
use axum::middleware::{self, Next};
use axum::response::{IntoResponse, Response};
use axum::routing::{get, post, put};
use axum::{Json, Router};
use serde::{Deserialize, Serialize};
use wikid_core::{
	Check, DoctorOptions, Document, EditResult, GlobResult, GrepOptions, GrepResult, HealthReport, LinkReport, Listing,
	MvResult, ReadLimit, RmResult, Vault, VaultStatus, WriteResult,
};

use crate::config::Config;
use crate::error::ApiError;

/// Shared server state: the opened vaults and the token → actor map.
#[derive(Debug)]
pub struct AppState {
	wikis: BTreeMap<String, Vault>,
	tokens: BTreeMap<String, String>,
}

impl AppState {
	/// Builds state from already-opened vaults (tests, embedders).
	pub fn new(wikis: BTreeMap<String, Vault>, tokens: BTreeMap<String, String>) -> Self {
		Self { wikis, tokens }
	}

	/// Opens every configured wiki, failing fast when a directory is missing
	/// (DESIGN §7 startup validation).
	pub fn from_config(config: &Config) -> anyhow::Result<Self> {
		let mut wikis = BTreeMap::new();
		for (name, path) in &config.wikis {
			let vault = Vault::open(path).with_context(|| format!("wiki {name:?} at {}", path.display()))?;
			wikis.insert(name.clone(), vault);
		}
		Ok(Self::new(wikis, config.tokens.clone()))
	}

	/// True when no tokens are configured (auth-less mode; loopback only).
	pub fn auth_less(&self) -> bool {
		self.tokens.is_empty()
	}

	fn wiki(&self, name: &str) -> Result<&Vault, ApiError> {
		self.wikis
			.get(name)
			.ok_or_else(|| ApiError::unknown_wiki(name, self.wikis.keys()))
	}
}

/// Builds the full router (DESIGN §7). Everything except `GET /health` sits
/// behind the bearer-token middleware. Tests drive this with
/// `tower::ServiceExt::oneshot`; only [`crate::serve`] binds a port.
pub fn app(state: AppState) -> Router {
	let state = Arc::new(state);
	let authed = Router::new()
		.route("/v1/wikis", get(list_wikis))
		.route("/v1/wikis/{wiki}/status", get(wiki_status))
		.route("/v1/wikis/{wiki}/ls", get(ls))
		.route("/v1/wikis/{wiki}/cat", get(cat))
		.route("/v1/wikis/{wiki}/grep", get(grep))
		.route("/v1/wikis/{wiki}/glob", get(glob))
		.route("/v1/wikis/{wiki}/links", get(links))
		.route("/v1/wikis/{wiki}/doctor", get(doctor))
		.route("/v1/wikis/{wiki}/pages", put(write_page).delete(rm_page))
		.route("/v1/wikis/{wiki}/edit", post(edit))
		.route("/v1/wikis/{wiki}/mv", post(mv))
		.route_layer(middleware::from_fn_with_state(state.clone(), auth))
		.with_state(state);
	Router::new()
		.route("/health", get(health))
		.merge(authed)
		.fallback(route_not_found)
		.method_not_allowed_fallback(method_not_allowed)
}

async fn health() -> Json<serde_json::Value> {
	Json(serde_json::json!({"status": "ok"}))
}

async fn route_not_found() -> ApiError {
	ApiError::route_not_found()
}

async fn method_not_allowed() -> ApiError {
	ApiError::method_not_allowed()
}

/// Bearer-token gate + request log line (actor name from the token).
async fn auth(State(state): State<Arc<AppState>>, request: Request, next: Next) -> Response {
	let Some(actor) = authenticate(&state, &request) else {
		return ApiError::unauthorized().into_response();
	};
	let method = request.method().clone();
	let path = request.uri().path().to_owned();
	let response = next.run(request).await;
	tracing::info!(%actor, %method, path, status = response.status().as_u16(), "request");
	response
}

/// An empty token map means auth-less mode — `serve` refuses that off
/// loopback, so it only ever applies to local traffic.
fn authenticate(state: &AppState, request: &Request) -> Option<String> {
	if state.auth_less() {
		return Some("anonymous".to_owned());
	}
	let value = request.headers().get(header::AUTHORIZATION)?.to_str().ok()?;
	let token = value.strip_prefix("Bearer ")?;
	// Compare against every configured token without early exit so timing
	// does not leak how much of a guessed token matched.
	let mut actor = None;
	for (candidate, name) in &state.tokens {
		if constant_time_eq(candidate.as_bytes(), token.as_bytes()) {
			actor = Some(name.clone());
		}
	}
	actor
}

/// Constant-time byte comparison: XOR-folds the full length instead of
/// returning at the first mismatch. Length differences still short-circuit —
/// standard for bearer tokens, where length is not the secret.
fn constant_time_eq(a: &[u8], b: &[u8]) -> bool {
	a.len() == b.len() && a.iter().zip(b).fold(0u8, |acc, (x, y)| acc | (x ^ y)) == 0
}

/// One wiki in the daemon's `GET /v1/wikis` listing.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct WikiSummary {
	/// The configured wiki name.
	pub name: String,
	/// Markdown pages visible in the wiki.
	pub pages: usize,
}

/// Result of `GET /v1/wikis`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct WikiList {
	/// Every wiki this daemon serves, sorted by name.
	pub wikis: Vec<WikiSummary>,
}

async fn list_wikis(State(state): State<Arc<AppState>>) -> Result<Json<WikiList>, ApiError> {
	let mut wikis = Vec::with_capacity(state.wikis.len());
	for (name, vault) in &state.wikis {
		// Depth 0 lists nothing but still aggregates subtree totals.
		let pages = vault.ls(None, 0)?.total_pages;
		wikis.push(WikiSummary {
			name: name.clone(),
			pages,
		});
	}
	Ok(Json(WikiList { wikis }))
}

async fn wiki_status(
	State(state): State<Arc<AppState>>,
	Path(wiki): Path<String>,
) -> Result<Json<VaultStatus>, ApiError> {
	Ok(Json(state.wiki(&wiki)?.status()?))
}

#[derive(Deserialize)]
struct LsQuery {
	path: Option<String>,
	depth: Option<usize>,
}

async fn ls(
	State(state): State<Arc<AppState>>,
	Path(wiki): Path<String>,
	query: Result<Query<LsQuery>, QueryRejection>,
) -> Result<Json<Listing>, ApiError> {
	let Query(q) = query?;
	Ok(Json(state.wiki(&wiki)?.ls(q.path.as_deref(), q.depth.unwrap_or(1))?))
}

#[derive(Deserialize)]
struct CatQuery {
	path: String,
	#[serde(default)]
	full: bool,
}

async fn cat(
	State(state): State<Arc<AppState>>,
	Path(wiki): Path<String>,
	query: Result<Query<CatQuery>, QueryRejection>,
) -> Result<Json<Document>, ApiError> {
	let Query(q) = query?;
	let limit = if q.full { None } else { Some(ReadLimit::default()) };
	Ok(Json(state.wiki(&wiki)?.cat(&q.path, limit)?))
}

#[derive(Deserialize)]
struct GrepQuery {
	pattern: String,
	#[serde(default)]
	ignore_case: bool,
	#[serde(default)]
	files_only: bool,
	#[serde(default)]
	context: usize,
	limit: Option<usize>,
}

async fn grep(
	State(state): State<Arc<AppState>>,
	Path(wiki): Path<String>,
	query: Result<Query<GrepQuery>, QueryRejection>,
) -> Result<Json<GrepResult>, ApiError> {
	let Query(q) = query?;
	let mut opts = GrepOptions {
		ignore_case: q.ignore_case,
		files_only: q.files_only,
		context: q.context,
		..Default::default()
	};
	if let Some(limit) = q.limit {
		opts.limit = limit;
	}
	Ok(Json(state.wiki(&wiki)?.grep(&q.pattern, &opts)?))
}

#[derive(Deserialize)]
struct GlobQuery {
	pattern: String,
}

async fn glob(
	State(state): State<Arc<AppState>>,
	Path(wiki): Path<String>,
	query: Result<Query<GlobQuery>, QueryRejection>,
) -> Result<Json<GlobResult>, ApiError> {
	let Query(q) = query?;
	Ok(Json(state.wiki(&wiki)?.glob(&q.pattern)?))
}

#[derive(Deserialize)]
struct LinksQuery {
	path: String,
}

async fn links(
	State(state): State<Arc<AppState>>,
	Path(wiki): Path<String>,
	query: Result<Query<LinksQuery>, QueryRejection>,
) -> Result<Json<LinkReport>, ApiError> {
	let Query(q) = query?;
	Ok(Json(state.wiki(&wiki)?.links(&q.path)?))
}

#[derive(Deserialize)]
struct DoctorQuery {
	stale_days: Option<u64>,
	checks: Option<String>,
}

async fn doctor(
	State(state): State<Arc<AppState>>,
	Path(wiki): Path<String>,
	query: Result<Query<DoctorQuery>, QueryRejection>,
) -> Result<Json<HealthReport>, ApiError> {
	let Query(q) = query?;
	let mut opts = DoctorOptions::default();
	if let Some(days) = q.stale_days {
		opts.stale_days = days;
	}
	if let Some(list) = q.checks {
		let checks = list
			.split(',')
			.map(|name| name.trim().parse::<Check>())
			.collect::<Result<Vec<_>, _>>()?;
		opts.checks = Some(checks);
	}
	Ok(Json(state.wiki(&wiki)?.doctor(&opts)?))
}

#[derive(Deserialize)]
struct WriteBody {
	path: String,
	content: String,
}

async fn write_page(
	State(state): State<Arc<AppState>>,
	Path(wiki): Path<String>,
	body: Result<Json<WriteBody>, JsonRejection>,
) -> Result<Json<WriteResult>, ApiError> {
	let Json(b) = body?;
	Ok(Json(state.wiki(&wiki)?.write(&b.path, &b.content)?))
}

#[derive(Deserialize)]
struct EditBody {
	path: String,
	old: String,
	new: String,
	#[serde(default)]
	all: bool,
}

async fn edit(
	State(state): State<Arc<AppState>>,
	Path(wiki): Path<String>,
	body: Result<Json<EditBody>, JsonRejection>,
) -> Result<Json<EditResult>, ApiError> {
	let Json(b) = body?;
	Ok(Json(state.wiki(&wiki)?.edit(&b.path, &b.old, &b.new, b.all)?))
}

#[derive(Deserialize)]
struct MvBody {
	from: String,
	to: String,
	#[serde(default)]
	force: bool,
}

async fn mv(
	State(state): State<Arc<AppState>>,
	Path(wiki): Path<String>,
	body: Result<Json<MvBody>, JsonRejection>,
) -> Result<Json<MvResult>, ApiError> {
	let Json(b) = body?;
	Ok(Json(state.wiki(&wiki)?.mv(&b.from, &b.to, b.force)?))
}

#[derive(Deserialize)]
struct RmQuery {
	path: String,
	force: Option<bool>,
}

async fn rm_page(
	State(state): State<Arc<AppState>>,
	Path(wiki): Path<String>,
	query: Result<Query<RmQuery>, QueryRejection>,
) -> Result<Json<RmResult>, ApiError> {
	let Query(q) = query?;
	let vault = state.wiki(&wiki)?;
	// The confirmation gate lives here, not in core (DESIGN §3 rm).
	if q.force != Some(true) {
		return Err(ApiError::force_required(&q.path));
	}
	Ok(Json(vault.rm(&q.path)?))
}

#[cfg(test)]
mod tests {
	use std::path::PathBuf;

	use axum::body::Body;
	use axum::http::{Method, StatusCode};
	use http_body_util::BodyExt as _;
	use serde_json::{Value, json};
	use tempfile::TempDir;
	use tower::ServiceExt as _;

	use super::*;

	const TOKEN: &str = "wkd_test_token";

	/// A minimal vault: two pages (one wikilink between them), one binary
	/// attachment.
	fn vault_dir() -> TempDir {
		let dir = tempfile::tempdir().unwrap();
		let write = |rel: &str, content: &[u8]| {
			let path = dir.path().join(rel);
			std::fs::create_dir_all(path.parent().unwrap()).unwrap();
			std::fs::write(path, content).unwrap();
		};
		write("index.md", b"# Home\n\nSee [[alpha]].\n");
		write(
			"projects/alpha.md",
			b"# Alpha\n\nalpha status: green\nsecond alpha line\n",
		);
		write("attachments/logo.png", b"\x89PNG\r\n\x00\x00binary");
		dir
	}

	fn test_app(dir: &TempDir) -> Router {
		let config = Config {
			bind: "127.0.0.1:0".to_owned(),
			default_wiki: None,
			wikis: BTreeMap::from([("main".to_owned(), dir.path().to_path_buf())]),
			tokens: BTreeMap::from([(TOKEN.to_owned(), "tester".to_owned())]),
		};
		app(AppState::from_config(&config).unwrap())
	}

	fn get_req(uri: &str) -> Request {
		Request::builder()
			.uri(uri)
			.header(header::AUTHORIZATION, format!("Bearer {TOKEN}"))
			.body(Body::empty())
			.unwrap()
	}

	fn json_req(method: Method, uri: &str, body: Value) -> Request {
		Request::builder()
			.method(method)
			.uri(uri)
			.header(header::AUTHORIZATION, format!("Bearer {TOKEN}"))
			.header(header::CONTENT_TYPE, "application/json")
			.body(Body::from(body.to_string()))
			.unwrap()
	}

	async fn call(app: &Router, request: Request) -> (StatusCode, Value) {
		let response = app.clone().oneshot(request).await.unwrap();
		let status = response.status();
		let bytes = response.into_body().collect().await.unwrap().to_bytes();
		let body = serde_json::from_slice(&bytes).unwrap_or_else(|e| {
			panic!(
				"non-JSON body for {status} ({e}): {:?}",
				String::from_utf8_lossy(&bytes)
			)
		});
		(status, body)
	}

	fn error_code(body: &Value) -> &str {
		body["error"]["code"].as_str().expect("error.code")
	}

	// --- auth ---

	#[tokio::test]
	async fn health_needs_no_token() {
		let dir = vault_dir();
		let app = test_app(&dir);
		let request = Request::builder().uri("/health").body(Body::empty()).unwrap();
		let (status, body) = call(&app, request).await;
		assert_eq!(status, StatusCode::OK);
		assert_eq!(body, json!({"status": "ok"}));
	}

	#[tokio::test]
	async fn missing_token_is_401() {
		let dir = vault_dir();
		let app = test_app(&dir);
		let request = Request::builder().uri("/v1/wikis").body(Body::empty()).unwrap();
		let (status, body) = call(&app, request).await;
		assert_eq!(status, StatusCode::UNAUTHORIZED);
		assert_eq!(error_code(&body), "unauthorized");
		assert!(body["error"]["hint"].is_string());
	}

	#[tokio::test]
	async fn bad_token_is_401() {
		let dir = vault_dir();
		let app = test_app(&dir);
		for value in ["Bearer wrong-token", TOKEN] {
			let request = Request::builder()
				.uri("/v1/wikis/main/status")
				.header(header::AUTHORIZATION, value)
				.body(Body::empty())
				.unwrap();
			let (status, body) = call(&app, request).await;
			assert_eq!(status, StatusCode::UNAUTHORIZED, "accepted {value:?}");
			assert_eq!(error_code(&body), "unauthorized");
		}
	}

	#[tokio::test]
	async fn empty_token_map_serves_without_auth() {
		let dir = vault_dir();
		let wikis = BTreeMap::from([("main".to_owned(), Vault::open(dir.path()).unwrap())]);
		let app = app(AppState::new(wikis, BTreeMap::new()));
		let request = Request::builder().uri("/v1/wikis").body(Body::empty()).unwrap();
		let (status, body) = call(&app, request).await;
		assert_eq!(status, StatusCode::OK);
		assert_eq!(body["wikis"][0]["name"], "main");
	}

	// --- unknown wiki / unknown route ---

	#[tokio::test]
	async fn unknown_wiki_is_404_listing_available() {
		let dir = vault_dir();
		let app = test_app(&dir);
		let (status, body) = call(&app, get_req("/v1/wikis/nope/status")).await;
		assert_eq!(status, StatusCode::NOT_FOUND);
		assert_eq!(error_code(&body), "unknown_wiki");
		assert!(
			body["error"]["message"].as_str().unwrap().contains("main"),
			"available names missing: {body}"
		);
	}

	#[tokio::test]
	async fn unknown_route_is_structured_404() {
		let dir = vault_dir();
		let app = test_app(&dir);
		let (status, body) = call(&app, get_req("/v1/nope")).await;
		assert_eq!(status, StatusCode::NOT_FOUND);
		assert_eq!(error_code(&body), "not_found");
	}

	#[tokio::test]
	async fn wrong_method_is_structured_405() {
		let dir = vault_dir();
		let app = test_app(&dir);
		let (status, body) = call(&app, get_req("/v1/wikis/main/edit")).await;
		assert_eq!(status, StatusCode::METHOD_NOT_ALLOWED);
		assert_eq!(error_code(&body), "method_not_allowed");
	}

	// --- happy paths, one per route ---

	#[tokio::test]
	async fn list_wikis_reports_names_and_page_counts() {
		let dir = vault_dir();
		let app = test_app(&dir);
		let (status, body) = call(&app, get_req("/v1/wikis")).await;
		assert_eq!(status, StatusCode::OK);
		assert_eq!(body, json!({"wikis": [{"name": "main", "pages": 2}]}));
	}

	#[tokio::test]
	async fn status_returns_the_core_struct() {
		let dir = vault_dir();
		let app = test_app(&dir);
		let (status, body) = call(&app, get_req("/v1/wikis/main/status")).await;
		assert_eq!(status, StatusCode::OK);
		let parsed: VaultStatus = serde_json::from_value(body).unwrap();
		assert_eq!(parsed.total_pages, 2);
		assert_eq!(parsed.total_files, 1);
	}

	#[tokio::test]
	async fn ls_lists_entries_with_depth() {
		let dir = vault_dir();
		let app = test_app(&dir);
		let (status, body) = call(&app, get_req("/v1/wikis/main/ls")).await;
		assert_eq!(status, StatusCode::OK);
		let listing: Listing = serde_json::from_value(body).unwrap();
		assert_eq!(listing.total_pages, 2);
		assert!(listing.entries.iter().any(|e| e.path == "index.md"));
		assert!(listing.entries.iter().all(|e| e.path != "projects/alpha.md"));

		let (status, body) = call(&app, get_req("/v1/wikis/main/ls?path=projects&depth=2")).await;
		assert_eq!(status, StatusCode::OK);
		let listing: Listing = serde_json::from_value(body).unwrap();
		assert!(listing.entries.iter().any(|e| e.path == "projects/alpha.md"));
	}

	#[tokio::test]
	async fn cat_reads_a_page_with_and_without_full() {
		let dir = vault_dir();
		let app = test_app(&dir);
		let (status, body) = call(&app, get_req("/v1/wikis/main/cat?path=index.md")).await;
		assert_eq!(status, StatusCode::OK);
		let doc: Document = serde_json::from_value(body).unwrap();
		assert!(doc.content.contains("[[alpha]]"));
		assert!(!doc.truncated);

		let (status, body) = call(&app, get_req("/v1/wikis/main/cat?path=index.md&full=true")).await;
		assert_eq!(status, StatusCode::OK);
		let doc: Document = serde_json::from_value(body).unwrap();
		assert_eq!(doc.total_lines, 3);
	}

	#[tokio::test]
	async fn grep_finds_matches_with_options() {
		let dir = vault_dir();
		let app = test_app(&dir);
		let (status, body) = call(&app, get_req("/v1/wikis/main/grep?pattern=alpha")).await;
		assert_eq!(status, StatusCode::OK);
		let result: GrepResult = serde_json::from_value(body).unwrap();
		assert_eq!(result.total_matches, 3);
		// Stem match ranks alpha.md first.
		assert_eq!(result.matches[0].path, "projects/alpha.md");

		let uri = "/v1/wikis/main/grep?pattern=ALPHA&ignore_case=true&files_only=true&limit=1";
		let (status, body) = call(&app, get_req(uri)).await;
		assert_eq!(status, StatusCode::OK);
		let result: GrepResult = serde_json::from_value(body).unwrap();
		assert_eq!(result.matches.len(), 1);
		assert!(result.truncated);
	}

	#[tokio::test]
	async fn glob_matches_paths() {
		let dir = vault_dir();
		let app = test_app(&dir);
		let (status, body) = call(&app, get_req("/v1/wikis/main/glob?pattern=**/*.md")).await;
		assert_eq!(status, StatusCode::OK);
		let result: GlobResult = serde_json::from_value(body).unwrap();
		assert_eq!(result.total, 2);
	}

	#[tokio::test]
	async fn links_reports_outgoing_and_backlinks() {
		let dir = vault_dir();
		let app = test_app(&dir);
		let (status, body) = call(&app, get_req("/v1/wikis/main/links?path=projects/alpha.md")).await;
		assert_eq!(status, StatusCode::OK);
		let report: LinkReport = serde_json::from_value(body).unwrap();
		assert!(report.outgoing.is_empty());
		assert_eq!(report.backlinks, vec!["index.md".to_owned()]);

		let (_, body) = call(&app, get_req("/v1/wikis/main/links?path=index.md")).await;
		let report: LinkReport = serde_json::from_value(body).unwrap();
		assert_eq!(report.outgoing[0].resolved.as_deref(), Some("projects/alpha.md"));
	}

	#[tokio::test]
	async fn doctor_runs_all_or_selected_checks() {
		let dir = vault_dir();
		let app = test_app(&dir);
		let (status, body) = call(&app, get_req("/v1/wikis/main/doctor")).await;
		assert_eq!(status, StatusCode::OK);
		let report: HealthReport = serde_json::from_value(body).unwrap();
		assert_eq!(report.counts.len(), 8);

		let uri = "/v1/wikis/main/doctor?checks=broken_links,stale_pages&stale_days=1";
		let (status, body) = call(&app, get_req(uri)).await;
		assert_eq!(status, StatusCode::OK);
		let report: HealthReport = serde_json::from_value(body).unwrap();
		assert_eq!(report.counts.len(), 2);
		// stale_days=1 flags nothing: the fixture was written moments ago.
		assert_eq!(report.counts["stale_pages"], 0);

		let (status, body) = call(&app, get_req("/v1/wikis/main/doctor?checks=bogus")).await;
		assert_eq!(status, StatusCode::BAD_REQUEST);
		assert_eq!(error_code(&body), "bad_pattern");
	}

	#[tokio::test]
	async fn write_creates_a_page() {
		let dir = vault_dir();
		let app = test_app(&dir);
		let body = json!({"path": "notes/new.md", "content": "# New\n"});
		let (status, body) = call(&app, json_req(Method::PUT, "/v1/wikis/main/pages", body)).await;
		assert_eq!(status, StatusCode::OK);
		let result: WriteResult = serde_json::from_value(body).unwrap();
		assert!(result.created);
		assert_eq!(result.bytes, 6);
		assert_eq!(
			std::fs::read_to_string(dir.path().join("notes/new.md")).unwrap(),
			"# New\n"
		);
	}

	#[tokio::test]
	async fn edit_replaces_text() {
		let dir = vault_dir();
		let app = test_app(&dir);
		let body = json!({"path": "projects/alpha.md", "old": "status: green", "new": "status: blue"});
		let (status, body) = call(&app, json_req(Method::POST, "/v1/wikis/main/edit", body)).await;
		assert_eq!(status, StatusCode::OK);
		let result: EditResult = serde_json::from_value(body).unwrap();
		assert_eq!(result.replacements, 1);
		let content = std::fs::read_to_string(dir.path().join("projects/alpha.md")).unwrap();
		assert!(content.contains("status: blue"));
	}

	#[tokio::test]
	async fn mv_renames_a_page() {
		let dir = vault_dir();
		let app = test_app(&dir);
		let body = json!({"from": "projects/alpha.md", "to": "archive/alpha.md", "force": false});
		let (status, body) = call(&app, json_req(Method::POST, "/v1/wikis/main/mv", body)).await;
		assert_eq!(status, StatusCode::OK);
		let result: MvResult = serde_json::from_value(body).unwrap();
		assert_eq!(result.to, "archive/alpha.md");
		assert!(dir.path().join("archive/alpha.md").exists());
		assert!(!dir.path().join("projects/alpha.md").exists());
	}

	#[tokio::test]
	async fn delete_with_force_removes_the_page() {
		let dir = vault_dir();
		let app = test_app(&dir);
		let request = Request::builder()
			.method(Method::DELETE)
			.uri("/v1/wikis/main/pages?path=projects/alpha.md&force=true")
			.header(header::AUTHORIZATION, format!("Bearer {TOKEN}"))
			.body(Body::empty())
			.unwrap();
		let (status, body) = call(&app, request).await;
		assert_eq!(status, StatusCode::OK);
		let result: RmResult = serde_json::from_value(body).unwrap();
		assert_eq!(result.path, "projects/alpha.md");
		assert!(!dir.path().join("projects/alpha.md").exists());
	}

	#[test]
	fn constant_time_eq_matches_equality() {
		assert!(constant_time_eq(b"wkd_token", b"wkd_token"));
		assert!(!constant_time_eq(b"wkd_token", b"wkd_tokeN"));
		assert!(!constant_time_eq(b"wkd_token", b"wkd_toke"));
		assert!(!constant_time_eq(b"", b"x"));
		assert!(constant_time_eq(b"", b""));
	}

	// --- refusals and error mapping through the wire ---

	#[tokio::test]
	async fn delete_without_force_is_400() {
		let dir = vault_dir();
		let app = test_app(&dir);
		for uri in [
			"/v1/wikis/main/pages?path=index.md",
			"/v1/wikis/main/pages?path=index.md&force=false",
		] {
			let request = Request::builder()
				.method(Method::DELETE)
				.uri(uri)
				.header(header::AUTHORIZATION, format!("Bearer {TOKEN}"))
				.body(Body::empty())
				.unwrap();
			let (status, body) = call(&app, request).await;
			assert_eq!(status, StatusCode::BAD_REQUEST, "deleted via {uri}");
			// The CLI's rm refusal, with wire wording (DESIGN §7).
			assert_eq!(error_code(&body), "force_required");
			assert_eq!(
				body["error"]["message"].as_str().unwrap(),
				"rm is destructive: refusing to delete index.md without force=true"
			);
			assert!(body["error"]["hint"].as_str().unwrap().contains("force=true"));
		}
		assert!(dir.path().join("index.md").exists());
	}

	#[tokio::test]
	async fn path_escape_is_400() {
		let dir = vault_dir();
		let app = test_app(&dir);
		let (status, body) = call(&app, get_req("/v1/wikis/main/cat?path=../outside.md")).await;
		assert_eq!(status, StatusCode::BAD_REQUEST);
		assert_eq!(error_code(&body), "invalid_path");

		let body_json = json!({"path": "../outside.md", "content": "x"});
		let (status, body) = call(&app, json_req(Method::PUT, "/v1/wikis/main/pages", body_json)).await;
		assert_eq!(status, StatusCode::BAD_REQUEST);
		assert_eq!(error_code(&body), "invalid_path");
	}

	#[tokio::test]
	async fn core_errors_keep_their_design_statuses() {
		let dir = vault_dir();
		let app = test_app(&dir);
		// NotFound → 404
		let (status, body) = call(&app, get_req("/v1/wikis/main/cat?path=nope.md")).await;
		assert_eq!(status, StatusCode::NOT_FOUND);
		assert_eq!(error_code(&body), "not_found");
		// NotUtf8 → 415
		let (status, body) = call(&app, get_req("/v1/wikis/main/cat?path=attachments/logo.png")).await;
		assert_eq!(status, StatusCode::UNSUPPORTED_MEDIA_TYPE);
		assert_eq!(error_code(&body), "not_utf8");
		// BadPattern → 400
		let (status, body) = call(&app, get_req("/v1/wikis/main/grep?pattern=(")).await;
		assert_eq!(status, StatusCode::BAD_REQUEST);
		assert_eq!(error_code(&body), "bad_pattern");
		// Ambiguous → 409, and the hint comes from core verbatim.
		let edit = json!({"path": "projects/alpha.md", "old": "alpha", "new": "beta"});
		let (status, body) = call(&app, json_req(Method::POST, "/v1/wikis/main/edit", edit)).await;
		assert_eq!(status, StatusCode::CONFLICT);
		assert_eq!(error_code(&body), "ambiguous");
		assert!(body["error"]["hint"].as_str().unwrap().contains('2'));
		// NoMatch → 404
		let edit = json!({"path": "projects/alpha.md", "old": "zzzz qqqq", "new": "x"});
		let (status, body) = call(&app, json_req(Method::POST, "/v1/wikis/main/edit", edit)).await;
		assert_eq!(status, StatusCode::NOT_FOUND);
		assert_eq!(error_code(&body), "no_match");
		// AlreadyExists → 409
		let mv = json!({"from": "index.md", "to": "projects/alpha.md"});
		let (status, body) = call(&app, json_req(Method::POST, "/v1/wikis/main/mv", mv)).await;
		assert_eq!(status, StatusCode::CONFLICT);
		assert_eq!(error_code(&body), "already_exists");
	}

	#[tokio::test]
	async fn malformed_query_and_body_are_structured_400s() {
		let dir = vault_dir();
		let app = test_app(&dir);
		let (status, body) = call(&app, get_req("/v1/wikis/main/ls?depth=abc")).await;
		assert_eq!(status, StatusCode::BAD_REQUEST);
		assert_eq!(error_code(&body), "usage");
		// cat without its required path parameter.
		let (status, body) = call(&app, get_req("/v1/wikis/main/cat")).await;
		assert_eq!(status, StatusCode::BAD_REQUEST);
		assert_eq!(error_code(&body), "usage");
		// Non-JSON body on a JSON route.
		let request = Request::builder()
			.method(Method::POST)
			.uri("/v1/wikis/main/edit")
			.header(header::AUTHORIZATION, format!("Bearer {TOKEN}"))
			.header(header::CONTENT_TYPE, "application/json")
			.body(Body::from("not json"))
			.unwrap();
		let (status, body) = call(&app, request).await;
		assert_eq!(status, StatusCode::BAD_REQUEST);
		assert_eq!(error_code(&body), "usage");
	}

	#[tokio::test]
	async fn from_config_fails_fast_on_missing_wiki_dir() {
		let config = Config {
			bind: "127.0.0.1:0".to_owned(),
			default_wiki: None,
			wikis: BTreeMap::from([("ghost".to_owned(), PathBuf::from("/nonexistent/wiki"))]),
			tokens: BTreeMap::new(),
		};
		let err = AppState::from_config(&config).unwrap_err();
		assert!(format!("{err:#}").contains("ghost"), "got: {err:#}");
	}
}
