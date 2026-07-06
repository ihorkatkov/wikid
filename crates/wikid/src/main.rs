//! The `wikid` binary: one clap surface over two modes (DESIGN §6). Local
//! mode calls `wikid-core` directly against `--dir`/`$WIKID_DIR`; remote mode
//! speaks the HTTP API against `--server`/`$WIKID_SERVER` through the same
//! rendering paths. `wikid serve` hosts `wikid-server` and is the CLI's only
//! async entry point.

mod error;
mod remote;
mod render;
mod update;

use std::io::{Read, Write};
use std::path::{Path, PathBuf};

use clap::{CommandFactory, Parser, Subcommand};
use serde::Serialize;
use wikid_core::{
	Check, DoctorOptions, DoctorProfile, Document, EditResult, GlobResult, GrepOptions, GrepResult, HashlinesResult,
	HealthReport, LineEdit, LinkReport, Listing, MvResult, ReadLimit, ReadRange, RmResult, TagReport, Vault,
	VaultStatus, WikidError, WriteResult,
};

use crate::error::CliError;
use crate::remote::Remote;

/// wikid — plain-Markdown wikis for humans and remote agents.
///
/// Point `wikid serve` at one or more wiki directories (Obsidian vaults
/// included) and every agent gets filesystem-feeling access over CLI and MCP.
#[derive(Parser)]
#[command(name = "wikid", version, about, arg_required_else_help = false)]
struct Cli {
	/// Local wiki directory (or $WIKID_DIR)
	#[arg(long, global = true, value_name = "PATH", conflicts_with = "server")]
	dir: Option<String>,

	/// Remote daemon URL (or $WIKID_SERVER)
	#[arg(long, global = true, value_name = "URL")]
	server: Option<String>,

	/// Bearer token for remote mode (or $WIKID_TOKEN)
	#[arg(long, global = true, value_name = "TOKEN")]
	token: Option<String>,

	/// Wiki name on the remote daemon (or $WIKID_WIKI)
	#[arg(long, global = true, value_name = "NAME")]
	wiki: Option<String>,

	/// Config file ($WIKID_CONFIG → ./wikid.toml → ~/.config/wikid/config.toml)
	#[arg(long, global = true, value_name = "PATH")]
	config: Option<String>,

	/// Emit the result as one JSON object instead of human text
	#[arg(long, global = true)]
	json: bool,

	#[command(subcommand)]
	command: Option<Command>,
}

#[derive(Subcommand)]
enum Command {
	/// Run the daemon serving configured wikis
	Serve,
	/// Initialize a blank LLM Wiki skeleton and register it in config
	Init { path: Option<String> },
	/// Show configured tokens (explicit secret-revealing commands)
	Token {
		#[command(subcommand)]
		command: TokenCommand,
	},
	/// Update the installed wikid binary from GitHub releases
	Update {
		/// Check whether an update is available without installing it
		#[arg(long)]
		check: bool,
		/// Reinstall even when the selected release is not newer
		#[arg(long)]
		force: bool,
		/// Install a specific release tag, e.g. v0.2.0
		#[arg(long, value_name = "TAG")]
		version: Option<String>,
	},
	/// Show page counts, recent activity, and health summary
	Status,
	/// List pages and directories
	Ls { path: Option<String> },
	/// List pages and directories recursively
	Tree {
		path: Option<String>,
		/// How many levels deep to list
		#[arg(long, value_name = "N", default_value_t = 3)]
		depth: usize,
	},
	/// Read a page; wikilinks may carry #Heading or #^block-id fragments
	Cat {
		path: String,
		/// Print the whole file instead of the first 400 lines / 32 KiB
		#[arg(long, conflicts_with = "lines")]
		full: bool,
		/// Read a 1-based inclusive line range, e.g. --lines 1200-1260
		#[arg(long, value_name = "START-END")]
		lines: Option<ReadRange>,
		/// Prefix each line with its number and hash (line:hash: text) for edit
		#[arg(long)]
		hashes: bool,
	},
	/// Search page content (regex)
	Grep {
		pattern: String,
		/// Case-insensitive matching
		#[arg(short = 'i', long)]
		ignore_case: bool,
		/// Print one line per matching file instead of per matching line
		#[arg(short = 'l', long)]
		files_only: bool,
		/// Lines of context around each match
		#[arg(short = 'C', long, value_name = "N", default_value_t = 0)]
		context: usize,
		/// Maximum number of matches returned
		#[arg(long, value_name = "N", default_value_t = 50)]
		limit: usize,
	},
	/// Find pages by path pattern
	Glob { pattern: String },
	/// Create or overwrite a page (content from stdin, or -m)
	Write {
		path: String,
		/// Inline content for one-liners (a trailing newline is added)
		#[arg(short = 'm', long, value_name = "TEXT")]
		message: Option<String>,
	},
	/// Replace a line by number and hash (read them with cat --hashes)
	Edit {
		path: String,
		/// 1-based line number to replace
		#[arg(long, value_name = "N")]
		line: usize,
		/// Hash of the line as last read (refused if the line changed since)
		#[arg(long, value_name = "HASH")]
		hash: String,
		/// Replacement text; may contain newlines to expand into several lines; use --new=-x for leading '-'
		#[arg(long, value_name = "TEXT")]
		new: String,
	},
	/// Replace multiple hash-addressed lines from a JSON array on stdin
	EditBatch { path: String },
	/// Rename or move a page
	Mv {
		from: String,
		to: String,
		/// Overwrite the destination if it exists
		#[arg(long)]
		force: bool,
	},
	/// Delete a page (requires --force)
	Rm {
		path: String,
		/// Confirm the deletion (there is no undo)
		#[arg(long)]
		force: bool,
	},
	/// Show links/backlinks; #Heading/#^block-id fragments kept; ![[...]] has embed=true
	Links { path: String },
	/// List inline and frontmatter tags, counting occurrences across the vault
	Tags,
	/// Run structural health checks
	Doctor {
		/// Pages not modified in this many days are stale (default 90)
		#[arg(long, value_name = "N")]
		stale_days: Option<u64>,
		/// Comma-separated subset of checks to run (e.g. broken_links,orphan_pages)
		#[arg(long, value_name = "a,b,c")]
		checks: Option<String>,
		/// Doctor policy profile: llm-wiki (default) or strict
		#[arg(long, value_name = "NAME", default_value = "llm-wiki")]
		profile: DoctorProfile,
	},
}

#[derive(Subcommand)]
enum TokenCommand {
	/// Print the token for an actor from the local config
	Show { actor: Option<String> },
}

/// A successful command's output and exit code (grep exits 1 on zero
/// matches, coreutils-faithful — AXI checklist item 5).
struct Outcome {
	text: String,
	code: i32,
}

impl Outcome {
	fn ok(text: String) -> Self {
		Self { text, code: 0 }
	}
}

fn main() {
	let cli = Cli::parse();
	let json = cli.json;
	// Errors go to stdout (structured, machine-parseable), exit code 1.
	match run(cli) {
		Ok(out) => {
			write_stdout(&out.text, out.code);
		}
		Err(err) => {
			write_stdout(&if json { err.json() } else { err.human() }, 1);
		}
	}
}

fn write_stdout(text: &str, code: i32) -> ! {
	let mut stdout = std::io::stdout().lock();
	if let Err(err) = writeln!(stdout, "{text}") {
		if err.kind() == std::io::ErrorKind::BrokenPipe {
			std::process::exit(0);
		}
		std::process::exit(1);
	}
	std::process::exit(code);
}

fn run(cli: Cli) -> Result<Outcome, CliError> {
	// AXI checklist item 1: no arguments shows live data, never help text.
	let command = cli.command.unwrap_or(Command::Status);
	let command = match command {
		Command::Serve => return run_serve(cli.config.as_deref(), cli.json),
		Command::Init { path } => return run_init(path.as_deref(), cli.config.as_deref(), cli.json),
		Command::Token { command } => return run_token(command, cli.config.as_deref(), cli.json),
		Command::Update { check, force, version } => return run_update(check, force, version.as_deref(), cli.json),
		other => other,
	};
	let explicit_dir = cli.dir;
	let explicit_server = cli.server;
	let explicit_token = cli.token;
	let explicit_wiki = cli.wiki;
	let env_dir = env_var("WIKID_DIR");
	let env_server = env_var("WIKID_SERVER");
	let has_explicit_local = explicit_dir.is_some();
	let has_explicit_remote = explicit_server.is_some() || explicit_token.is_some() || explicit_wiki.is_some();
	if !has_explicit_local && !has_explicit_remote && env_dir.is_some() && env_server.is_some() {
		// Flag-vs-flag conflicts are caught by clap itself; this covers env-only
		// local+remote targeting. Explicit flags win over opposite-mode env vars.
		Cli::command()
			.error(
				clap::error::ErrorKind::ArgumentConflict,
				"--dir/$WIKID_DIR and --server/$WIKID_SERVER cannot both be set",
			)
			.exit();
	}
	let dir = explicit_dir.or_else(|| (!has_explicit_remote).then_some(env_dir).flatten());
	let server = explicit_server.or_else(|| (!has_explicit_local).then_some(env_server).flatten());
	if has_explicit_remote && server.is_none() {
		return Err(CliError::no_target());
	}
	let backend = if let Some(server) = server {
		let token = explicit_token.or_else(|| env_var("WIKID_TOKEN"));
		let wiki = explicit_wiki
			.or_else(|| env_var("WIKID_WIKI"))
			.ok_or_else(CliError::no_wiki)?;
		Backend::Remote(Remote::new(&server, token, wiki))
	} else if let Some(dir) = dir {
		// A missing vault directory deserves better than the generic
		// not-found hint ("run ls…" — there is nothing to ls yet).
		let vault = Vault::open(&dir).map_err(|err| match err {
			wikid_core::WikidError::NotFound { path } => CliError::new(
				"not_found",
				format!("wiki directory not found: {path}"),
				Some("pass an existing directory via --dir or $WIKID_DIR".to_owned()),
			),
			other => CliError::from(other),
		})?;
		Backend::Local(vault)
	} else if let Some(resolved) = resolve_config_target(cli.config.as_deref())? {
		Backend::Local(Vault::open(&resolved.path)?)
	} else {
		return Err(CliError::no_target());
	};
	dispatch(&backend, command, cli.json)
}

/// `wikid serve` (DESIGN §6): discover the config (arg → `$WIKID_CONFIG` →
/// `./wikid.toml` → `~/.config/wikid/config.toml`), then run `wikid-server`
/// on a tokio runtime until stopped. The rest of the CLI stays sync.
fn run_serve(config_arg: Option<&str>, json: bool) -> Result<Outcome, CliError> {
	let requested = config_arg.map(Path::new);
	let cwd = std::env::current_dir().map_err(io_error)?;
	let (path, config, bootstrapped) = load_or_bootstrap_config(requested, &cwd)?;
	let startup = ServeStartup::from_config(&path, &config, bootstrapped);
	let startup_text = if json {
		serde_json::to_string(&startup).expect("startup serializes")
	} else {
		render_serve_startup(&startup)
	};
	println!("{startup_text}");
	std::io::stdout().flush().map_err(io_error)?;
	tracing_subscriber::fmt()
		.with_env_filter(
			tracing_subscriber::EnvFilter::try_from_default_env()
				.unwrap_or_else(|_| tracing_subscriber::EnvFilter::new("info")),
		)
		.with_writer(std::io::stderr)
		.init();
	let runtime = tokio::runtime::Runtime::new()
		.map_err(|err| CliError::new("io", format!("failed to start async runtime: {err}"), None))?;
	runtime
		.block_on(wikid_server::serve(config))
		.map_err(|err| CliError::new("serve", format!("{err:#}"), None))?;
	Ok(Outcome::ok("wikid-server stopped".to_owned()))
}

fn run_init(path: Option<&str>, config_arg: Option<&str>, json: bool) -> Result<Outcome, CliError> {
	let root = match path {
		Some(path) => PathBuf::from(path),
		None => std::env::current_dir().map_err(io_error)?,
	};
	std::fs::create_dir_all(&root).map_err(io_error)?;
	let root = root.canonicalize().map_err(io_error)?;
	let scaffold = create_skeleton(&root)?;
	let config_path = wikid_server::config::write_target(config_arg.map(Path::new)).ok_or_else(CliError::no_config)?;
	let (mut config, existed) = load_config_for_write(&config_path)?;
	let registration = register_wiki(&mut config, &root);
	ensure_admin_token(&mut config)?;
	wikid_server::config::save(&config_path, &config)
		.map_err(|err| CliError::new("config", format!("{err:#}"), None))?;
	let result = InitResult {
		path: root.display().to_string(),
		config_path: config_path.display().to_string(),
		wiki_name: registration.name,
		registered: registration.registered,
		config_created: !existed,
		created: scaffold.created,
		skipped: scaffold.skipped,
	};
	Ok(Outcome::ok(emit(json, &result, || render_init(&result))))
}

fn run_update(check: bool, force: bool, version: Option<&str>, json: bool) -> Result<Outcome, CliError> {
	let result = update::run(check, force, version)?;
	Ok(Outcome::ok(emit(json, &result, || update::render(&result))))
}

fn run_token(command: TokenCommand, config_arg: Option<&str>, json: bool) -> Result<Outcome, CliError> {
	match command {
		TokenCommand::Show { actor } => {
			let path = wikid_server::config::discover(config_arg.map(Path::new)).ok_or_else(CliError::no_config)?;
			let config =
				wikid_server::Config::load(&path).map_err(|err| CliError::new("config", format!("{err:#}"), None))?;
			let actor = actor.unwrap_or_else(|| "admin".to_owned());
			let mut matches: Vec<_> = config.tokens.iter().filter(|(_, name)| *name == &actor).collect();
			if matches.is_empty() {
				return Err(CliError::new(
					"token_not_found",
					format!("no token configured for actor {actor:?}"),
					Some(format!("inspect [tokens] in {}", path.display())),
				));
			}
			if matches.len() > 1 {
				return Err(CliError::new(
					"ambiguous_token",
					format!("multiple tokens configured for actor {actor:?}"),
					Some(format!("open {} and choose the token explicitly", path.display())),
				));
			}
			let (token, actor) = matches.pop().unwrap();
			let result = TokenShowResult {
				actor: actor.clone(),
				token: token.clone(),
				config_path: path.display().to_string(),
			};
			Ok(Outcome::ok(emit(json, &result, || render_token(&result))))
		}
	}
}

#[derive(Debug, Serialize)]
struct InitResult {
	path: String,
	config_path: String,
	wiki_name: String,
	registered: bool,
	config_created: bool,
	created: Vec<String>,
	skipped: Vec<String>,
}

#[derive(Debug, Serialize)]
struct TokenShowResult {
	actor: String,
	token: String,
	config_path: String,
}

#[derive(Debug, Serialize)]
struct ServeStartup {
	config_path: String,
	bind: String,
	bootstrapped: bool,
	wikis: Vec<WikiRegistration>,
	admin_token: String,
}

impl ServeStartup {
	fn from_config(path: &Path, config: &wikid_server::Config, bootstrapped: bool) -> Self {
		Self {
			config_path: path.display().to_string(),
			bind: config.bind.clone(),
			bootstrapped,
			wikis: config
				.wikis
				.iter()
				.map(|(name, path)| WikiRegistration {
					name: name.clone(),
					path: path.display().to_string(),
				})
				.collect(),
			admin_token: format!("admin token written to {} (not printed)", path.display()),
		}
	}
}

#[derive(Debug, Serialize)]
struct WikiRegistration {
	name: String,
	path: String,
}

struct ScaffoldResult {
	created: Vec<String>,
	skipped: Vec<String>,
}

struct RegisterResult {
	name: String,
	registered: bool,
}

struct ConfigTarget {
	path: PathBuf,
}

fn load_or_bootstrap_config(
	requested: Option<&Path>,
	cwd: &Path,
) -> Result<(PathBuf, wikid_server::Config, bool), CliError> {
	if let Some(path) = wikid_server::config::discover(requested)
		&& path.is_file()
	{
		let config =
			wikid_server::Config::load(&path).map_err(|err| CliError::new("config", format!("{err:#}"), None))?;
		return Ok((path, config, false));
	}
	let path = wikid_server::config::write_target(requested).ok_or_else(CliError::no_config)?;
	let mut config = wikid_server::Config::empty();
	let cwd = cwd.canonicalize().map_err(io_error)?;
	register_wiki(&mut config, &cwd);
	ensure_admin_token(&mut config)?;
	wikid_server::config::save(&path, &config).map_err(|err| CliError::new("config", format!("{err:#}"), None))?;
	Ok((path, config, true))
}

fn load_config_for_write(path: &Path) -> Result<(wikid_server::Config, bool), CliError> {
	if path.is_file() {
		let config =
			wikid_server::Config::load(path).map_err(|err| CliError::new("config", format!("{err:#}"), None))?;
		Ok((config, true))
	} else {
		Ok((wikid_server::Config::empty(), false))
	}
}

fn register_wiki(config: &mut wikid_server::Config, root: &Path) -> RegisterResult {
	let before = config.wikis.len();
	if let Some((name, _)) = config.wikis.iter().find(|(_, path)| canonical_eq(path, root)) {
		return RegisterResult {
			name: name.clone(),
			registered: false,
		};
	}
	let base = root.file_name().and_then(|name| name.to_str()).unwrap_or("wiki");
	let name = unique_wiki_name(config, base);
	config.wikis.insert(name.clone(), root.to_path_buf());
	if before == 0 && config.default_wiki.is_none() {
		config.default_wiki = Some(name.clone());
	}
	RegisterResult { name, registered: true }
}

fn unique_wiki_name(config: &wikid_server::Config, base: &str) -> String {
	if !config.wikis.contains_key(base) {
		return base.to_owned();
	}
	for n in 2.. {
		let candidate = format!("{base}-{n}");
		if !config.wikis.contains_key(&candidate) {
			return candidate;
		}
	}
	unreachable!()
}

fn canonical_eq(path: &Path, root: &Path) -> bool {
	path.canonicalize().map(|path| path == root).unwrap_or(false)
}

fn ensure_admin_token(config: &mut wikid_server::Config) -> Result<(), CliError> {
	if config.tokens.values().any(|actor| actor == "admin") {
		return Ok(());
	}
	config.tokens.insert(generate_token()?, "admin".to_owned());
	Ok(())
}

fn generate_token() -> Result<String, CliError> {
	let mut bytes = [0u8; 32];
	std::fs::File::open("/dev/urandom")
		.and_then(|mut file| file.read_exact(&mut bytes))
		.map_err(|err| {
			CliError::new(
				"token_generation",
				format!("failed to read random bytes from /dev/urandom: {err}"),
				Some("/dev/urandom is required for no-dependency token generation on this platform".to_owned()),
			)
		})?;
	let mut token = String::from("wkd_");
	for byte in bytes {
		token.push_str(&format!("{byte:02x}"));
	}
	Ok(token)
}

fn create_skeleton(root: &Path) -> Result<ScaffoldResult, CliError> {
	let mut created = Vec::new();
	let mut skipped = Vec::new();
	for dir in ["raw", "raw/assets", "concepts", "entities", "questions", "syntheses"] {
		let path = root.join(dir);
		if path.exists() {
			skipped.push(format!("{dir}/"));
		} else {
			std::fs::create_dir_all(&path).map_err(io_error)?;
			created.push(format!("{dir}/"));
		}
	}
	for (path, content) in INIT_FILES {
		let target = root.join(path);
		if target.exists() {
			skipped.push((*path).to_owned());
		} else {
			std::fs::write(&target, content).map_err(io_error)?;
			created.push((*path).to_owned());
		}
	}
	Ok(ScaffoldResult { created, skipped })
}

fn resolve_config_target(config_arg: Option<&str>) -> Result<Option<ConfigTarget>, CliError> {
	let Some(path) = wikid_server::config::discover(config_arg.map(Path::new)) else {
		return Ok(None);
	};
	let config = wikid_server::Config::load(&path).map_err(|err| CliError::new("config", format!("{err:#}"), None))?;
	let cwd = std::env::current_dir()
		.map_err(io_error)?
		.canonicalize()
		.map_err(io_error)?;
	if let Some((_, path)) = config
		.wikis
		.iter()
		.filter_map(|(name, path)| path.canonicalize().ok().map(|path| (name, path)))
		.filter(|(_, path)| cwd.starts_with(path))
		.max_by_key(|(_, path)| path.components().count())
	{
		return Ok(Some(ConfigTarget { path }));
	}
	if config.wikis.len() == 1 {
		return Ok(Some(ConfigTarget {
			path: config.wikis.values().next().unwrap().clone(),
		}));
	}
	if config.wikis.is_empty() {
		return Ok(None);
	}
	if let Some(default) = &config.default_wiki
		&& let Some(path) = config.wikis.get(default)
	{
		return Ok(Some(ConfigTarget { path: path.clone() }));
	}
	let names = config.wikis.keys().cloned().collect::<Vec<_>>().join(", ");
	Err(CliError::new(
		"ambiguous_wiki",
		format!("multiple wikis registered: {names}"),
		Some(format!("set default_wiki in {} or pass --dir/--wiki", path.display())),
	))
}

fn render_init(result: &InitResult) -> String {
	let mut lines = vec![format!("initialized wiki: {}", result.path)];
	if !result.created.is_empty() {
		lines.push(format!("created: {}", result.created.join(", ")));
	}
	if !result.skipped.is_empty() {
		lines.push(format!("skipped: {}", result.skipped.join(", ")));
	}
	let action = if result.registered {
		"registered"
	} else {
		"already registered"
	};
	lines.push(format!("{action}: {} -> {}", result.wiki_name, result.path));
	lines.push(format!("config: {}", result.config_path));
	lines.push("admin token: written to config (not printed)".to_owned());
	lines.push("hint: wikid status — inspect this wiki".to_owned());
	lines.push("hint: wikid serve — serve registered wikis".to_owned());
	lines.join("\n")
}

fn render_token(result: &TokenShowResult) -> String {
	format!(
		"{}\nhint: token for actor {:?} from {}",
		result.token, result.actor, result.config_path
	)
}

fn render_serve_startup(startup: &ServeStartup) -> String {
	let mut lines = Vec::new();
	if startup.bootstrapped {
		lines.push(format!("created config: {}", startup.config_path));
	} else {
		lines.push(format!("config: {}", startup.config_path));
	}
	lines.push(format!("serving: http://{}", startup.bind));
	for wiki in &startup.wikis {
		lines.push(format!("wiki: {} -> {}", wiki.name, wiki.path));
	}
	lines.push(startup.admin_token.clone());
	lines.push(format!(
		"hint: wikid token show admin --config {} — print the admin token",
		startup.config_path
	));
	lines.join("\n")
}

fn io_error(err: std::io::Error) -> CliError {
	CliError::new("io", err.to_string(), None)
}

const INIT_FILES: &[(&str, &str)] = &[
	("index.md", INDEX_TEMPLATE),
	("log.md", LOG_TEMPLATE),
	("AGENTS.md", AGENTS_TEMPLATE),
];

const INDEX_TEMPLATE: &str = r#"# Index

This is the content-oriented catalog for this LLM Wiki. The maintaining agent updates it on every ingest, query, or synthesis worth keeping.

## Sources

Raw inputs live in `raw/`. Add each processed source here with a one-line summary and link to any generated pages.

## Entities

Entity pages live in `entities/`.

## Concepts

Concept pages live in `concepts/`.

## Questions

Reusable questions and answered queries live in `questions/`.

## Syntheses

Durable analyses, comparisons, and briefs live in `syntheses/`.
"#;

const LOG_TEMPLATE: &str = r#"# Log

Append one entry per meaningful maintenance action. Keep the prefix parseable:

## [YYYY-MM-DD] ingest | <title>

## [YYYY-MM-DD] query | <title>

## [YYYY-MM-DD] lint | <title>
"#;

const AGENTS_TEMPLATE: &str = r#"# LLM Wiki Agent Instructions

This directory is a blank LLM Wiki: a plain-Markdown knowledge base maintained by an LLM agent.

## Architecture

- `raw/` contains immutable sources. Read these files, but do not rewrite them except by explicit human request.
- `raw/assets/` contains local images and attachments referenced by raw sources.
- `concepts/`, `entities/`, `questions/`, and `syntheses/` contain compiled wiki pages. The agent owns and maintains these pages.
- `index.md` is the content catalog. Update it whenever pages are created or materially changed.
- `log.md` is the chronological maintenance log. Append entries with `## [YYYY-MM-DD] <ingest|query|lint> | <title>`.

## Conventions

- Use `[[wikilinks]]` for internal links.
- Prefer short pages with clear headings.
- Preserve raw evidence and separate it from synthesis.
- When answering a reusable question, consider filing the answer under `questions/` or `syntheses/`.
- When ingesting a source, update relevant concept/entity pages, then update `index.md` and `log.md`.
- When linting, look for broken links, orphan pages, stale claims, contradictions, and missing pages.

## wikid CLI

- `wikid status` shows the selected wiki's overview.
- `wikid grep <pattern>` searches wiki content.
- `wikid cat <path>` reads a page.
- `wikid links <path>` shows outgoing links and backlinks.
- `wikid doctor` checks structural health.
- `wikid serve` exposes registered wikis over HTTP.
- `wikid edit-batch <path>` reads a JSON array of hash-guarded line edits from stdin for multiple safe replacements in one file.

Paths are wiki-root-relative, even when the shell cwd is inside a subdirectory of the wiki. In remote mode, `status` shows `root (server): ...`; that path belongs to the machine running `wikid serve`, not the client shell.
"#;

/// The targeted wiki: a local directory or a remote daemon. Both expose the
/// same operations returning the shared core structs, so `dispatch` renders
/// identically in either mode (DESIGN §6).
enum Backend {
	Local(Vault),
	Remote(Remote),
}

impl Backend {
	fn status(&self) -> Result<VaultStatus, CliError> {
		match self {
			Self::Local(vault) => Ok(vault.status()?),
			Self::Remote(remote) => remote.status(),
		}
	}

	fn ls(&self, path: Option<&str>, depth: usize) -> Result<Listing, CliError> {
		match self {
			Self::Local(vault) => Ok(vault.ls(path, depth)?),
			Self::Remote(remote) => remote.ls(path, depth),
		}
	}

	fn cat(&self, path: &str, full: bool, lines: Option<ReadRange>) -> Result<Document, CliError> {
		match self {
			Self::Local(vault) => {
				let limit = if full || lines.is_some() {
					None
				} else {
					Some(ReadLimit::default())
				};
				with_extension_hint(vault, path, vault.cat_with_range(path, limit, lines))
			}
			Self::Remote(remote) => remote.cat(path, full, lines),
		}
	}

	fn cat_hashes(&self, path: &str, full: bool, lines: Option<ReadRange>) -> Result<HashlinesResult, CliError> {
		match self {
			Self::Local(vault) => {
				let limit = if full || lines.is_some() {
					None
				} else {
					Some(ReadLimit::default())
				};
				with_extension_hint(vault, path, vault.cat_hashes_with_range(path, limit, lines))
			}
			Self::Remote(remote) => remote.cat_hashes(path, full, lines),
		}
	}

	fn grep(&self, pattern: &str, opts: &GrepOptions) -> Result<GrepResult, CliError> {
		match self {
			Self::Local(vault) => Ok(vault.grep(pattern, opts)?),
			Self::Remote(remote) => remote.grep(pattern, opts),
		}
	}

	fn glob(&self, pattern: &str) -> Result<GlobResult, CliError> {
		match self {
			Self::Local(vault) => Ok(vault.glob(pattern)?),
			Self::Remote(remote) => remote.glob(pattern),
		}
	}

	fn write(&self, path: &str, content: &str) -> Result<WriteResult, CliError> {
		match self {
			Self::Local(vault) => Ok(vault.write(path, content)?),
			Self::Remote(remote) => remote.write(path, content),
		}
	}

	fn edit(&self, path: &str, edits: &[LineEdit]) -> Result<EditResult, CliError> {
		match self {
			Self::Local(vault) => Ok(vault.edit(path, edits)?),
			Self::Remote(remote) => remote.edit(path, edits),
		}
	}

	fn mv(&self, from: &str, to: &str, force: bool) -> Result<MvResult, CliError> {
		match self {
			Self::Local(vault) => Ok(vault.mv(from, to, force)?),
			Self::Remote(remote) => remote.mv(from, to, force),
		}
	}

	fn rm(&self, path: &str) -> Result<RmResult, CliError> {
		match self {
			Self::Local(vault) => Ok(vault.rm(path)?),
			Self::Remote(remote) => remote.rm(path),
		}
	}

	fn links(&self, path: &str) -> Result<LinkReport, CliError> {
		match self {
			Self::Local(vault) => with_extension_hint(vault, path, vault.links(path)),
			Self::Remote(remote) => remote.links(path),
		}
	}

	fn tags(&self) -> Result<TagReport, CliError> {
		match self {
			Self::Local(vault) => Ok(vault.tags()?),
			Self::Remote(remote) => remote.tags(),
		}
	}

	fn doctor(
		&self,
		stale_days: Option<u64>,
		checks: Option<&[Check]>,
		profile: DoctorProfile,
	) -> Result<HealthReport, CliError> {
		match self {
			Self::Local(vault) => {
				let mut opts = DoctorOptions::default();
				if let Some(days) = stale_days {
					opts.stale_days = days;
				}
				opts.checks = checks.map(<[Check]>::to_vec);
				opts.profile = profile;
				Ok(vault.doctor(&opts)?)
			}
			Self::Remote(remote) => remote.doctor(stale_days, checks, profile),
		}
	}
}

fn with_extension_hint<T>(vault: &Vault, requested: &str, result: Result<T, WikidError>) -> Result<T, CliError> {
	match result {
		Ok(value) => Ok(value),
		Err(WikidError::NotFound { path }) => Err(not_found_with_extension_hint(vault, requested, path)),
		Err(err) => Err(err.into()),
	}
}

fn not_found_with_extension_hint(vault: &Vault, requested: &str, path: String) -> CliError {
	let hint =
		md_extension_hint(vault, requested).unwrap_or_else(|| "run ls or glob to discover valid paths".to_string());
	CliError::new("not_found", format!("not found: {path}"), Some(hint))
}

fn md_extension_hint(vault: &Vault, requested: &str) -> Option<String> {
	if requested.ends_with(".md") {
		return None;
	}
	let candidate = format!("{requested}.md");
	let full_path = vault.root().join(&candidate);
	full_path.is_file().then(|| format!("did you mean {candidate}?"))
}

fn dispatch(backend: &Backend, command: Command, json: bool) -> Result<Outcome, CliError> {
	match command {
		Command::Serve | Command::Init { .. } | Command::Token { .. } | Command::Update { .. } => {
			unreachable!("handled in run()")
		}
		Command::Status => {
			let status = backend.status()?;
			Ok(Outcome::ok(emit(json, &status, || {
				render::status(&status, matches!(backend, Backend::Remote(_)))
			})))
		}
		Command::Ls { path } => {
			let listing = backend.ls(path.as_deref(), 1)?;
			Ok(Outcome::ok(emit(json, &listing, || render::listing(&listing, false))))
		}
		Command::Tree { path, depth } => {
			let listing = backend.ls(path.as_deref(), depth)?;
			Ok(Outcome::ok(emit(json, &listing, || render::listing(&listing, true))))
		}
		Command::Cat {
			path,
			full,
			lines,
			hashes,
		} => {
			if hashes {
				let result = backend.cat_hashes(&path, full, lines)?;
				return Ok(Outcome::ok(emit(json, &result, || render::hashlines(&result))));
			}
			let doc = backend.cat(&path, full, lines)?;
			Ok(Outcome::ok(emit(json, &doc, || render::document(&doc))))
		}
		Command::Grep {
			pattern,
			ignore_case,
			files_only,
			context,
			limit,
		} => {
			let opts = GrepOptions {
				ignore_case,
				files_only,
				context,
				limit,
			};
			let result = backend.grep(&pattern, &opts)?;
			let code = if result.total_matches == 0 { 1 } else { 0 };
			Ok(Outcome {
				text: emit(json, &result, || render::grep(&result, &pattern, files_only)),
				code,
			})
		}
		Command::Glob { pattern } => {
			let result = backend.glob(&pattern)?;
			Ok(Outcome::ok(emit(json, &result, || render::glob(&result, &pattern))))
		}
		Command::Write { path, message } => {
			let content = match message {
				Some(text) if text.ends_with('\n') => text,
				Some(text) => format!("{text}\n"),
				None => read_stdin()?,
			};
			let result = backend.write(&path, &content)?;
			Ok(Outcome::ok(emit(json, &result, || render::write(&result))))
		}
		Command::Edit { path, line, hash, new } => {
			let edits = [LineEdit {
				line,
				expected_hash: hash,
				new_text: new,
			}];
			let result = backend.edit(&path, &edits)?;
			Ok(Outcome::ok(emit(json, &result, || render::edit(&result))))
		}
		Command::EditBatch { path } => {
			let edits = read_edit_batch()?;
			let result = backend.edit(&path, &edits)?;
			Ok(Outcome::ok(emit(json, &result, || render::edit(&result))))
		}
		Command::Mv { from, to, force } => {
			let result = backend.mv(&from, &to, force)?;
			Ok(Outcome::ok(emit(json, &result, || render::mv(&result))))
		}
		Command::Rm { path, force } => {
			// AXI checklist item 6: the refusal is a structured error, never a
			// question. Gated here so local and remote refuse identically.
			if !force {
				return Err(CliError::force_required(&path));
			}
			let result = backend.rm(&path)?;
			Ok(Outcome::ok(emit(json, &result, || render::rm(&result))))
		}
		Command::Links { path } => {
			let report = backend.links(&path)?;
			Ok(Outcome::ok(emit(json, &report, || render::links(&report))))
		}
		Command::Tags => {
			let report = backend.tags()?;
			Ok(Outcome::ok(emit(json, &report, || render::tags(&report))))
		}
		Command::Doctor {
			stale_days,
			checks,
			profile,
		} => {
			let checks = checks.map(|list| parse_checks(&list)).transpose()?;
			let report = backend.doctor(stale_days, checks.as_deref(), profile)?;
			Ok(Outcome::ok(emit(json, &report, || render::doctor(&report))))
		}
	}
}

/// `--json` emits the core result struct directly; human mode renders it.
fn emit<T: Serialize>(json: bool, value: &T, human: impl FnOnce() -> String) -> String {
	if json {
		serde_json::to_string(value).expect("core result structs always serialize")
	} else {
		human()
	}
}

fn env_var(name: &str) -> Option<String> {
	std::env::var(name).ok().filter(|value| !value.is_empty())
}

fn read_stdin() -> Result<String, CliError> {
	let mut content = String::new();
	std::io::stdin()
		.read_to_string(&mut content)
		.map_err(|e| CliError::new("io", format!("failed to read content from stdin: {e}"), None))?;
	Ok(content)
}

fn read_edit_batch() -> Result<Vec<LineEdit>, CliError> {
	let input = read_stdin()?;
	serde_json::from_str::<Vec<LineEdit>>(&input).map_err(|err| {
		CliError::new(
			"bad_edit",
			format!("edit-batch stdin must be a JSON array of line edits: {err}"),
			Some("example: [{\"line\":1,\"expected_hash\":\"abc123\",\"new_text\":\"replacement\"}]".to_owned()),
		)
	})
}

/// Parses the `--checks a,b,c` filter; unknown names surface the core
/// `bad_pattern` error listing valid checks.
fn parse_checks(list: &str) -> Result<Vec<Check>, CliError> {
	list.split(',')
		.map(str::trim)
		.filter(|name| !name.is_empty())
		.map(|name| name.parse::<Check>().map_err(CliError::from))
		.collect()
}

#[cfg(test)]
mod tests {
	use super::*;

	#[test]
	fn cli_definition_is_consistent() {
		Cli::command().debug_assert();
	}

	#[test]
	fn parse_checks_accepts_names_and_rejects_unknowns() {
		let checks = parse_checks("broken_links, orphan_pages").unwrap();
		assert_eq!(checks, vec![Check::BrokenLinks, Check::OrphanPages]);
		assert!(parse_checks("").unwrap().is_empty());
		let err = parse_checks("nonsense").unwrap_err();
		assert_eq!(err.code, "bad_pattern");
	}
}
