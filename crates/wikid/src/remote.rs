//! Remote mode (DESIGN §6): a sync `ureq` client mapping each CLI command 1:1
//! onto the HTTP API (DESIGN §7) and deserializing the shared core structs, so
//! results flow through the exact same rendering paths as local mode. Daemon
//! error bodies (`{"error":{code,message,hint}}`) map back onto [`CliError`]
//! verbatim — remote failures render as the same `error[<code>]` lines.

use serde::Deserialize;
use serde::de::DeserializeOwned;
use serde_json::json;
use wikid_core::{
	Check, Document, EditResult, GlobResult, GrepOptions, GrepResult, HashlinesResult, HealthReport, LineEdit,
	LinkReport, Listing, MvResult, RmResult, VaultStatus, WriteResult,
};

use crate::error::CliError;

/// A connection to one wiki on a remote daemon.
pub struct Remote {
	base: String,
	token: Option<String>,
	wiki: String,
	agent: ureq::Agent,
}

impl Remote {
	/// `token` is optional: auth-less loopback daemons accept bare requests.
	pub fn new(server: &str, token: Option<String>, wiki: String) -> Self {
		Self {
			base: server.trim_end_matches('/').to_owned(),
			token,
			wiki,
			agent: ureq::agent(),
		}
	}

	fn url(&self, op: &str) -> String {
		format!("{}/v1/wikis/{}/{op}", self.base, self.wiki)
	}

	fn prepare(&self, request: ureq::Request) -> ureq::Request {
		match &self.token {
			Some(token) => request.set("Authorization", &format!("Bearer {token}")),
			None => request,
		}
	}

	fn get<T: DeserializeOwned>(&self, op: &str, query: &[(&str, String)]) -> Result<T, CliError> {
		let mut request = self.prepare(self.agent.get(&self.url(op)));
		for (key, value) in query {
			request = request.query(key, value);
		}
		parse(request.call())
	}

	fn send_json<T: DeserializeOwned>(&self, method: &str, op: &str, body: serde_json::Value) -> Result<T, CliError> {
		let request = self.prepare(self.agent.request(method, &self.url(op)));
		parse(request.send_json(body))
	}

	pub fn status(&self) -> Result<VaultStatus, CliError> {
		self.get("status", &[])
	}

	pub fn ls(&self, path: Option<&str>, depth: usize) -> Result<Listing, CliError> {
		let mut query = vec![("depth", depth.to_string())];
		if let Some(path) = path {
			query.push(("path", path.to_owned()));
		}
		self.get("ls", &query)
	}

	pub fn cat(&self, path: &str, full: bool) -> Result<Document, CliError> {
		self.get("cat", &[("path", path.to_owned()), ("full", full.to_string())])
	}

	pub fn cat_hashes(&self, path: &str, full: bool) -> Result<HashlinesResult, CliError> {
		self.get(
			"cat",
			&[
				("path", path.to_owned()),
				("full", full.to_string()),
				("hashes", "true".to_owned()),
			],
		)
	}

	pub fn grep(&self, pattern: &str, opts: &GrepOptions) -> Result<GrepResult, CliError> {
		self.get(
			"grep",
			&[
				("pattern", pattern.to_owned()),
				("ignore_case", opts.ignore_case.to_string()),
				("files_only", opts.files_only.to_string()),
				("context", opts.context.to_string()),
				("limit", opts.limit.to_string()),
			],
		)
	}

	pub fn glob(&self, pattern: &str) -> Result<GlobResult, CliError> {
		self.get("glob", &[("pattern", pattern.to_owned())])
	}

	pub fn links(&self, path: &str) -> Result<LinkReport, CliError> {
		self.get("links", &[("path", path.to_owned())])
	}

	pub fn doctor(&self, stale_days: Option<u64>, checks: Option<&[Check]>) -> Result<HealthReport, CliError> {
		let mut query = Vec::new();
		if let Some(days) = stale_days {
			query.push(("stale_days", days.to_string()));
		}
		if let Some(checks) = checks {
			let names: Vec<&str> = checks.iter().map(|check| check.name()).collect();
			query.push(("checks", names.join(",")));
		}
		self.get("doctor", &query)
	}

	pub fn write(&self, path: &str, content: &str) -> Result<WriteResult, CliError> {
		self.send_json("PUT", "pages", json!({"path": path, "content": content}))
	}

	pub fn edit(&self, path: &str, edits: &[LineEdit]) -> Result<EditResult, CliError> {
		self.send_json("POST", "edit", json!({"path": path, "edits": edits}))
	}

	pub fn mv(&self, from: &str, to: &str, force: bool) -> Result<MvResult, CliError> {
		self.send_json("POST", "mv", json!({"from": from, "to": to, "force": force}))
	}

	/// The `--force` gate lives in the CLI (shared with local mode), so the
	/// wire call is always the already-confirmed form.
	pub fn rm(&self, path: &str) -> Result<RmResult, CliError> {
		let request = self
			.prepare(self.agent.delete(&self.url("pages")))
			.query("path", path)
			.query("force", "true");
		parse(request.call())
	}
}

fn parse<T: DeserializeOwned>(result: Result<ureq::Response, ureq::Error>) -> Result<T, CliError> {
	match result {
		Ok(response) => response
			.into_json::<T>()
			.map_err(|err| CliError::new("transport", format!("invalid response body from server: {err}"), None)),
		Err(ureq::Error::Status(status, response)) => Err(error_from_body(status, response)),
		Err(ureq::Error::Transport(err)) => Err(CliError::new(
			"transport",
			format!("cannot reach server: {err}"),
			Some("check --server/$WIKID_SERVER and that `wikid serve` is running there".to_owned()),
		)),
	}
}

/// Maps the daemon's structured error body back onto the same `error[<code>]`
/// rendering local mode uses; a body that isn't ours becomes a plain `http`
/// error carrying the status.
fn error_from_body(status: u16, response: ureq::Response) -> CliError {
	#[derive(Deserialize)]
	struct Body {
		error: Detail,
	}
	#[derive(Deserialize)]
	struct Detail {
		code: String,
		message: String,
		hint: Option<String>,
	}
	match response.into_json::<Body>() {
		Ok(body) => CliError::new(&body.error.code, body.error.message, body.error.hint),
		Err(_) => CliError::new(
			"http",
			format!("server returned HTTP {status} with an unrecognized body"),
			None,
		),
	}
}

#[cfg(test)]
mod tests {
	use super::*;

	#[test]
	fn urls_join_base_wiki_and_op_and_trim_trailing_slash() {
		let remote = Remote::new("http://localhost:7448/", None, "main".to_owned());
		assert_eq!(remote.url("status"), "http://localhost:7448/v1/wikis/main/status");
		let remote = Remote::new("http://localhost:7448", Some("t".to_owned()), "notes".to_owned());
		assert_eq!(remote.url("ls"), "http://localhost:7448/v1/wikis/notes/ls");
	}

	#[test]
	fn structured_error_bodies_keep_code_message_and_hint() {
		let body = r#"{"error":{"code":"not_found","message":"not found: a.md","hint":"wikid ls"}}"#;
		let response = ureq::Response::new(404, "Not Found", body).unwrap();
		let err = error_from_body(404, response);
		assert_eq!(err.code, "not_found");
		assert_eq!(err.message, "not found: a.md");
		assert_eq!(err.hint.as_deref(), Some("wikid ls"));
	}

	#[test]
	fn unrecognized_error_bodies_become_http_errors_with_the_status() {
		let response = ureq::Response::new(502, "Bad Gateway", "<html>oops</html>").unwrap();
		let err = error_from_body(502, response);
		assert_eq!(err.code, "http");
		assert!(err.message.contains("502"), "{}", err.message);
	}
}
