//! The `wikid` binary: one clap surface over two modes (DESIGN §6). Local
//! mode calls `wikid-core` directly against `--dir`/`$WIKID_DIR`; remote mode
//! speaks the HTTP API against `--server`/`$WIKID_SERVER` through the same
//! rendering paths. `wikid serve` hosts `wikid-server` and is the CLI's only
//! async entry point.

mod error;
mod remote;
mod render;
mod skills;
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
#[command(
	name = "wikid",
	version,
	about,
	arg_required_else_help = false,
	before_help = "Start here (for AI agents):\n  wikid skills get core\n\n  Usage guides ship inside this binary — always version-matched. They\n  cover the read→edit hash protocol, wikilink resolution, tags, doctor,\n  and remote mode. Prefer them over guessing from the flag list below.\n  `wikid skills` lists all guides (core, llm-wiki).\n"
)]
struct Cli {
	/// Local wiki directory (or $WIKID_DIR)
	#[arg(long, global = true, value_name = "PATH", conflicts_with = "server")]
	dir: Option<String>,

	/// Remote daemon URL (or $WIKID_SERVER)
	#[arg(long, global = true, value_name = "URL")]
	server: Option<String>,

	/// Bearer token for direct remote mode or a named profile override (or $WIKID_TOKEN)
	#[arg(long, global = true, value_name = "TOKEN")]
	token: Option<String>,

	/// Configured local or remote target name
	#[arg(
		long,
		global = true,
		value_name = "NAME",
		conflicts_with_all = ["dir", "server", "wiki"]
	)]
	target: Option<String>,

	/// Daemon wiki with --server; without --server, a legacy alias for --target
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
	/// List and print embedded agent usage guides
	#[command(
		display_order = 0,
		long_about = "List and print embedded agent usage guides. Guides are embedded in the binary; wiki target flags are ignored."
	)]
	Skills {
		#[command(subcommand)]
		command: Option<SkillsCommand>,
	},
	/// Run the daemon serving configured wikis
	#[command(display_order = 10)]
	Serve,
	/// Initialize a blank LLM Wiki skeleton and register it in config
	#[command(display_order = 20)]
	Init { path: Option<String> },
	/// Inspect configured local wikis and remote servers without revealing tokens
	#[command(
		display_order = 30,
		long_about = "Inspect the discovered config's local wiki targets and remote server profiles. Token values are never printed; target-selection flags are ignored."
	)]
	Config {
		#[command(subcommand)]
		command: ConfigCommand,
	},
	/// Show configured tokens (explicit secret-revealing commands)
	#[command(display_order = 30)]
	Token {
		#[command(subcommand)]
		command: TokenCommand,
	},
	/// Update the installed wikid binary from GitHub releases
	#[command(display_order = 40)]
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
	#[command(display_order = 50)]
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
	/// Read a page or wikilink fragment; #Heading returns its section, #^block-id its line
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
	/// List inline and frontmatter tags; implied nested-tag ancestors are marked
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
enum SkillsCommand {
	/// List embedded usage guides
	List,
	/// Print a usage guide
	#[command(long_about = "Print a usage guide. Guides are embedded in the binary; wiki target flags are ignored.")]
	Get {
		#[arg(value_name = "NAME", help = skills::skill_name_help())]
		name: String,
		/// Append reference documents after SKILL.md
		#[arg(long)]
		full: bool,
	},
	/// Materialize embedded usage guides and print their path
	Path { name: Option<String> },
	/// Report embedded guides, materialized cache, and Claude skill wiring
	Status,
}

#[derive(Subcommand)]
enum ConfigCommand {
	/// List local wiki targets and remote server profiles
	List,
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
	let write_result = (|| -> std::io::Result<()> {
		stdout.write_all(text.as_bytes())?;
		if !text.ends_with('\n') {
			stdout.write_all(b"\n")?;
		}
		Ok(())
	})();
	if let Err(err) = write_result {
		if err.kind() == std::io::ErrorKind::BrokenPipe {
			std::process::exit(0);
		}
		std::process::exit(1);
	}
	std::process::exit(code);
}

fn run(cli: Cli) -> Result<Outcome, CliError> {
	// Bare `wikid` is the orientation dashboard; explicit `status` stays focused.
	let overview = cli.command.is_none();
	let command = cli.command.unwrap_or(Command::Status);
	let command = match command {
		Command::Skills { command } => return run_skills(command, cli.json),
		Command::Serve => return run_serve(cli.config.as_deref(), cli.json),
		Command::Init { path } => {
			return run_init(path.as_deref(), cli.dir.as_deref(), cli.config.as_deref(), cli.json);
		}
		Command::Config { command } => return run_config(command, cli.config.as_deref(), cli.json),
		Command::Token { command } => return run_token(command, cli.config.as_deref(), cli.json),
		Command::Update { check, force, version } => return run_update(check, force, version.as_deref(), cli.json),
		other => other,
	};
	let config_arg = cli.config;
	let explicit_dir = cli.dir;
	let explicit_server = cli.server;
	let explicit_token = cli.token;
	let explicit_target = cli.target;
	let explicit_wiki = cli.wiki;
	let env_dir = env_var("WIKID_DIR");
	let env_server = env_var("WIKID_SERVER");
	let env_token = env_var("WIKID_TOKEN");
	let env_wiki = env_var("WIKID_WIKI");
	let has_explicit_target =
		explicit_dir.is_some() || explicit_server.is_some() || explicit_target.is_some() || explicit_wiki.is_some();
	if !has_explicit_target && explicit_token.is_none() && env_dir.is_some() && env_server.is_some() {
		// Flag-vs-flag conflicts are caught by clap itself; this covers env-only
		// local+remote targeting. Explicit flags win over opposite-mode env vars.
		Cli::command()
			.error(
				clap::error::ErrorKind::ArgumentConflict,
				"--dir/$WIKID_DIR and --server/$WIKID_SERVER cannot both be set",
			)
			.exit();
	}
	let selected = if let Some(dir) = explicit_dir {
		let name = target_name_from_path(Path::new(&dir));
		open_local_target(
			Path::new(&dir),
			name,
			false,
			false,
			format!("--dir {}", shell_arg(&dir)),
		)?
	} else if let Some(server) = explicit_server {
		let wiki = explicit_wiki.or(env_wiki).ok_or_else(CliError::no_wiki)?;
		direct_remote_target(
			server,
			explicit_token.clone().or(env_token),
			wiki,
			explicit_token.is_some(),
		)
	} else if let Some(name) = explicit_target.or(explicit_wiki) {
		match resolve_config_target(config_arg.as_deref(), Some(&name), explicit_token, env_token)? {
			Some(target) => target.into_selected(config_arg.as_deref())?,
			None => {
				return Err(CliError::new(
					"no_target",
					format!("no configured target found for {name:?}"),
					Some("run wikid config list or use --server with --wiki for a direct daemon wiki".to_owned()),
				));
			}
		}
	} else if let Some(server) = env_server {
		let wiki = env_wiki.ok_or_else(CliError::no_wiki)?;
		direct_remote_target(
			server,
			explicit_token.clone().or(env_token),
			wiki,
			explicit_token.is_some(),
		)
	} else if explicit_token.is_some() {
		return Err(CliError::no_target());
	} else if let Some(dir) = env_dir {
		let name = target_name_from_path(Path::new(&dir));
		open_local_target(
			Path::new(&dir),
			name,
			false,
			false,
			format!("--dir {}", shell_arg(&dir)),
		)?
	} else if let Some(target) = resolve_config_target(config_arg.as_deref(), None, None, env_token)? {
		target.into_selected(config_arg.as_deref())?
	} else {
		return Err(CliError::no_target());
	};
	if overview {
		run_overview(&selected, config_arg.as_deref(), cli.json)
	} else {
		dispatch(&selected.backend, &selected.context, command, cli.json)
	}
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

fn run_init(
	path: Option<&str>,
	explicit_dir: Option<&str>,
	config_arg: Option<&str>,
	json: bool,
) -> Result<Outcome, CliError> {
	let root = resolve_init_root(path, explicit_dir)?;
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

fn resolve_init_root(path: Option<&str>, explicit_dir: Option<&str>) -> Result<PathBuf, CliError> {
	if let Some(path) = path {
		return Ok(PathBuf::from(path));
	}
	if let Some(dir) = explicit_dir {
		return Ok(PathBuf::from(dir));
	}
	if let Some(dir) = env_var("WIKID_DIR") {
		return Ok(PathBuf::from(dir));
	}
	std::env::current_dir().map_err(io_error)
}

fn run_update(check: bool, force: bool, version: Option<&str>, json: bool) -> Result<Outcome, CliError> {
	let result = update::run(check, force, version)?;
	Ok(Outcome::ok(emit(json, &result, || update::render(&result))))
}

fn run_skills(command: Option<SkillsCommand>, json: bool) -> Result<Outcome, CliError> {
	match command.unwrap_or(SkillsCommand::List) {
		SkillsCommand::List => {
			let result = skills::list()?;
			Ok(Outcome::ok(emit(json, &result, || skills::render_list(&result))))
		}
		SkillsCommand::Get { name, full } => {
			let result = skills::get(&name, full)?;
			Ok(Outcome::ok(emit(json, &result, || result.content.clone())))
		}
		SkillsCommand::Path { name } => {
			let result = skills::materialize(name.as_deref())?;
			Ok(Outcome::ok(emit(json, &result, || result.path.clone())))
		}
		SkillsCommand::Status => {
			let result = skills::status()?;
			Ok(Outcome::ok(emit(json, &result, || skills::render_status(&result))))
		}
	}
}

fn run_config(command: ConfigCommand, config_arg: Option<&str>, json: bool) -> Result<Outcome, CliError> {
	match command {
		ConfigCommand::List => {
			let path = wikid_server::config::discover(config_arg.map(Path::new)).ok_or_else(CliError::no_config)?;
			let config =
				wikid_server::Config::load(&path).map_err(|err| CliError::new("config", format!("{err:#}"), None))?;
			warn_insecure_config(&path, &config);
			let mut targets = config
				.wikis
				.iter()
				.map(|(name, path)| ConfigListTarget::Local {
					name: name.clone(),
					path: path.display().to_string(),
				})
				.chain(config.remotes.iter().map(|(name, remote)| ConfigListTarget::Remote {
					name: name.clone(),
					server: remote.server.clone(),
					wiki: remote.wiki.clone().unwrap_or_else(|| name.clone()),
				}))
				.collect::<Vec<_>>();
			targets.sort_by(|left, right| left.name().cmp(right.name()));
			let result = ConfigListResult {
				config_path: path.display().to_string(),
				bind: config.bind,
				default_wiki: config.default_wiki,
				targets,
			};
			Ok(Outcome::ok(emit(json, &result, || render_config_list(&result))))
		}
	}
}

fn run_token(command: TokenCommand, config_arg: Option<&str>, json: bool) -> Result<Outcome, CliError> {
	match command {
		TokenCommand::Show { actor } => {
			let path = wikid_server::config::discover(config_arg.map(Path::new)).ok_or_else(CliError::no_config)?;
			let config =
				wikid_server::Config::load(&path).map_err(|err| CliError::new("config", format!("{err:#}"), None))?;
			warn_insecure_config(&path, &config);
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
struct ConfigListResult {
	config_path: String,
	bind: String,
	#[serde(skip_serializing_if = "Option::is_none")]
	default_wiki: Option<String>,
	targets: Vec<ConfigListTarget>,
}

#[derive(Debug, Clone, Serialize)]
#[serde(tag = "kind", rename_all = "lowercase")]
enum ConfigListTarget {
	Local { name: String, path: String },
	Remote { name: String, server: String, wiki: String },
}

impl ConfigListTarget {
	fn name(&self) -> &str {
		match self {
			Self::Local { name, .. } | Self::Remote { name, .. } => name,
		}
	}
}

#[derive(Debug, Clone, Serialize)]
#[serde(tag = "kind", rename_all = "lowercase")]
enum OverviewTarget {
	Local {
		name: String,
		path: String,
		active: bool,
		default: bool,
		configured: bool,
	},
	Remote {
		name: String,
		server: String,
		wiki: String,
		active: bool,
		default: bool,
		configured: bool,
	},
}

impl OverviewTarget {
	fn name(&self) -> &str {
		match self {
			Self::Local { name, .. } | Self::Remote { name, .. } => name,
		}
	}

	fn is_active(&self) -> bool {
		match self {
			Self::Local { active, .. } | Self::Remote { active, .. } => *active,
		}
	}

	fn is_default(&self) -> bool {
		match self {
			Self::Local { default, .. } | Self::Remote { default, .. } => *default,
		}
	}

	fn set_active(&mut self, value: bool) {
		match self {
			Self::Local { active, .. } | Self::Remote { active, .. } => *active = value,
		}
	}

	fn matches_context(&self, context: &TargetContext) -> bool {
		match (self, &context.kind) {
			(Self::Local { path, .. }, TargetKind::Local { path: active_path }) => Path::new(path)
				.canonicalize()
				.map(|path| path.display().to_string() == *active_path)
				.unwrap_or(path == active_path),
			(
				Self::Remote { server, wiki, .. },
				TargetKind::Remote {
					server: active_server,
					wiki: active_wiki,
				},
			) => server.trim_end_matches('/') == active_server.trim_end_matches('/') && wiki == active_wiki,
			_ => false,
		}
	}
}

#[derive(Debug, Serialize)]
struct OverviewResult {
	targets: Vec<OverviewTarget>,
	active_target: String,
	status: VaultStatus,
}

struct OverviewTargets {
	targets: Vec<OverviewTarget>,
	matched_config: Option<MatchedConfigTarget>,
}

struct MatchedConfigTarget {
	name: String,
	is_default: bool,
}

#[derive(Debug, Clone)]
enum TargetKind {
	Local { path: String },
	Remote { server: String, wiki: String },
}

#[derive(Debug, Clone)]
struct TargetContext {
	name: String,
	kind: TargetKind,
	configured: bool,
	is_default: bool,
	hint_prefix: String,
}

impl TargetContext {
	fn with_target_hints(&self, text: String) -> String {
		text.lines()
			.map(|line| {
				let Some(command) = line.strip_prefix("hint: wikid ") else {
					return line.to_owned();
				};
				if command.starts_with("skills ")
					|| command.starts_with("--target ")
					|| command.starts_with("--config ")
					|| command.starts_with("--server ")
					|| command.starts_with("--dir ")
					|| self.hint_prefix.is_empty()
				{
					line.to_owned()
				} else {
					format!("hint: wikid {} {command}", self.hint_prefix)
				}
			})
			.collect::<Vec<_>>()
			.join("\n")
	}

	fn focused_status(&self, status: &VaultStatus) -> String {
		let mode = match (&self.kind, self.configured) {
			(TargetKind::Local { .. }, _) => "local",
			(TargetKind::Remote { .. }, true) => "remote",
			(TargetKind::Remote { .. }, false) => "direct remote",
		};
		let default = if self.is_default { ", default" } else { "" };
		let mut lines = vec![format!("target: {} ({mode}{default})", self.name)];
		match &self.kind {
			TargetKind::Local { .. } => {
				lines.push(format!("root: {}", status.root));
				lines.push(format!("wikid: {}", env!("CARGO_PKG_VERSION")));
			}
			TargetKind::Remote { server, wiki } => {
				lines.push(format!("wiki: {wiki}"));
				lines.push(format!("server: {server}"));
				lines.push(format!(
					"wikid: client {}  server {}",
					env!("CARGO_PKG_VERSION"),
					status.version
				));
			}
		}
		lines.push(render::status_body(status));
		lines.push("hint: wikid grep <pattern> — search this wiki".to_owned());
		lines.push("hint: wikid doctor — inspect structural issues".to_owned());
		lines.join("\n")
	}

	fn synthetic_overview_target(&self) -> OverviewTarget {
		match &self.kind {
			TargetKind::Local { path } => OverviewTarget::Local {
				name: self.name.clone(),
				path: path.clone(),
				active: true,
				default: self.is_default,
				configured: self.configured,
			},
			TargetKind::Remote { server, wiki } => OverviewTarget::Remote {
				name: self.name.clone(),
				server: server.clone(),
				wiki: wiki.clone(),
				active: true,
				default: self.is_default,
				configured: self.configured,
			},
		}
	}
}

struct SelectedTarget {
	backend: Backend,
	context: TargetContext,
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

enum ConfigTarget {
	Local {
		name: String,
		path: PathBuf,
		is_default: bool,
	},
	Remote {
		name: String,
		server: String,
		token: Option<String>,
		wiki: String,
		is_default: bool,
		token_override: bool,
	},
}

impl ConfigTarget {
	fn into_selected(self, config_arg: Option<&str>) -> Result<SelectedTarget, CliError> {
		match self {
			Self::Local { name, path, is_default } => {
				let hint_prefix = config_target_prefix(config_arg, &name, false);
				open_local_target(&path, name, true, is_default, hint_prefix)
			}
			Self::Remote {
				name,
				server,
				token,
				wiki,
				is_default,
				token_override,
			} => {
				let hint_prefix = config_target_prefix(config_arg, &name, token_override);
				Ok(SelectedTarget {
					backend: Backend::Remote(Remote::new(&server, token, wiki.clone())),
					context: TargetContext {
						name,
						kind: TargetKind::Remote { server, wiki },
						configured: true,
						is_default,
						hint_prefix,
					},
				})
			}
		}
	}
}

fn config_target_prefix(config_arg: Option<&str>, target: &str, token_override: bool) -> String {
	let mut parts = Vec::new();
	if let Some(config) = config_arg {
		parts.push(format!("--config {}", shell_arg(config)));
	}
	parts.push(format!("--target {}", shell_arg(target)));
	if token_override {
		parts.push("--token <TOKEN>".to_owned());
	}
	parts.join(" ")
}

fn open_local_vault(dir: &Path) -> Result<Vault, CliError> {
	// A missing vault directory deserves better than the generic not-found
	// hint ("run ls…" — there is nothing to ls yet).
	Vault::open(dir).map_err(|err| match err {
		wikid_core::WikidError::NotFound { path } => CliError::new(
			"not_found",
			format!("wiki directory not found: {path}"),
			Some("pass an existing directory via --dir or $WIKID_DIR".to_owned()),
		),
		other => CliError::from(other),
	})
}

fn open_local_target(
	dir: &Path,
	name: String,
	configured: bool,
	is_default: bool,
	hint_prefix: String,
) -> Result<SelectedTarget, CliError> {
	let vault = open_local_vault(dir)?;
	let path = vault.root().display().to_string();
	Ok(SelectedTarget {
		backend: Backend::Local(vault),
		context: TargetContext {
			name,
			kind: TargetKind::Local { path },
			configured,
			is_default,
			hint_prefix,
		},
	})
}

fn direct_remote_target(server: String, token: Option<String>, wiki: String, token_override: bool) -> SelectedTarget {
	let mut hint_prefix = format!("--server {} --wiki {}", shell_arg(&server), shell_arg(&wiki));
	if token_override {
		hint_prefix.push_str(" --token <TOKEN>");
	}
	SelectedTarget {
		backend: Backend::Remote(Remote::new(&server, token, wiki.clone())),
		context: TargetContext {
			name: wiki.clone(),
			kind: TargetKind::Remote { server, wiki },
			configured: false,
			is_default: false,
			hint_prefix,
		},
	}
}

fn target_name_from_path(path: &Path) -> String {
	path.file_name()
		.and_then(|name| name.to_str())
		.filter(|name| !name.is_empty())
		.unwrap_or("wiki")
		.to_owned()
}

fn shell_arg(value: &str) -> String {
	if !value.is_empty()
		&& value
			.chars()
			.all(|ch| ch.is_ascii_alphanumeric() || matches!(ch, '_' | '-' | '.' | '/' | ':' | '@'))
	{
		value.to_owned()
	} else {
		format!("'{}'", value.replace('\'', "'\"'\"'"))
	}
}

fn run_overview(selected: &SelectedTarget, config_arg: Option<&str>, json: bool) -> Result<Outcome, CliError> {
	let status = selected.backend.status()?;
	let overview_targets = overview_targets(config_arg, &selected.context)?;
	let mut active = selected.context.clone();
	if !active.configured
		&& let Some(matched) = overview_targets.matched_config
	{
		let token_override = active.hint_prefix.contains("--token <TOKEN>");
		active.name = matched.name;
		active.configured = true;
		active.is_default = matched.is_default;
		active.hint_prefix = config_target_prefix(config_arg, &active.name, token_override);
	}
	let result = OverviewResult {
		targets: overview_targets.targets,
		active_target: active.name.clone(),
		status,
	};
	Ok(Outcome::ok(emit(json, &result, || {
		render_overview(&result, &active, config_arg)
	})))
}

fn overview_targets(config_arg: Option<&str>, active: &TargetContext) -> Result<OverviewTargets, CliError> {
	let mut targets = Vec::new();
	if let Some(path) = wikid_server::config::discover(config_arg.map(Path::new)) {
		let config =
			wikid_server::Config::load(&path).map_err(|err| CliError::new("config", format!("{err:#}"), None))?;
		if !active.configured {
			warn_insecure_config(&path, &config);
		}
		for (name, path) in config.wikis {
			targets.push(OverviewTarget::Local {
				active: false,
				default: config.default_wiki.as_deref() == Some(&name),
				configured: true,
				name,
				path: path.display().to_string(),
			});
		}
		for (name, remote) in config.remotes {
			targets.push(OverviewTarget::Remote {
				active: false,
				default: config.default_wiki.as_deref() == Some(&name),
				configured: true,
				wiki: remote.wiki.unwrap_or_else(|| name.clone()),
				name,
				server: remote.server,
			});
		}
	}
	let matching = targets
		.iter()
		.enumerate()
		.filter(|(_, target)| {
			if active.configured {
				target.name() == active.name && target.matches_context(active)
			} else {
				target.matches_context(active)
			}
		})
		.map(|(index, _)| index)
		.collect::<Vec<_>>();
	let matched_config = if matching.len() == 1 {
		let target = &mut targets[matching[0]];
		target.set_active(true);
		Some(MatchedConfigTarget {
			name: target.name().to_owned(),
			is_default: target.is_default(),
		})
	} else {
		targets.push(active.synthetic_overview_target());
		None
	};
	targets.sort_by(|left, right| {
		right
			.is_active()
			.cmp(&left.is_active())
			.then_with(|| left.name().cmp(right.name()))
	});
	Ok(OverviewTargets {
		targets,
		matched_config,
	})
}

fn render_overview(result: &OverviewResult, active: &TargetContext, config_arg: Option<&str>) -> String {
	let name_width = result
		.targets
		.iter()
		.map(|target| target.name().len())
		.max()
		.unwrap_or(1);
	let mut lines = vec!["targets:".to_owned()];
	for target in &result.targets {
		match target {
			OverviewTarget::Local {
				name,
				path,
				active,
				default,
				..
			} => {
				let marker = if *active { '*' } else { ' ' };
				let tags = overview_tags(*active, *default);
				lines.push(format!("{marker} {name:name_width$}  local   {path}{tags}"));
			}
			OverviewTarget::Remote {
				name,
				server,
				wiki,
				active,
				default,
				..
			} => {
				let marker = if *active { '*' } else { ' ' };
				let tags = overview_tags(*active, *default);
				lines.push(format!(
					"{marker} {name:name_width$}  remote  {server}  wiki={wiki}{tags}"
				));
			}
		}
	}
	lines.push(String::new());
	lines.push(format!("active: {}", active.name));
	match &active.kind {
		TargetKind::Local { .. } => lines.push(format!("wikid: {}", env!("CARGO_PKG_VERSION"))),
		TargetKind::Remote { .. } => lines.push(format!(
			"wikid: client {}  server {}",
			env!("CARGO_PKG_VERSION"),
			result.status.version
		)),
	}
	lines.push(render::status_body(&result.status));
	lines.push(String::new());
	lines.push("hint: wikid skills get core — start here: load the agent usage guide".to_owned());
	let switch = match config_arg {
		Some(config) => format!("wikid --config {} --target <name>", shell_arg(config)),
		None => "wikid --target <name>".to_owned(),
	};
	lines.push(format!("hint: {switch} — inspect another target"));
	lines.push("hint: wikid grep <pattern> — search the active wiki".to_owned());
	lines.push("hint: wikid doctor — inspect structural issues".to_owned());
	active.with_target_hints(lines.join("\n"))
}

fn overview_tags(active: bool, default: bool) -> &'static str {
	match (active, default) {
		(true, true) => "  [active, default]",
		(true, false) => "  [active]",
		(false, true) => "  [default]",
		(false, false) => "",
	}
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
		warn_insecure_config(&path, &config);
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

fn resolve_config_target(
	config_arg: Option<&str>,
	requested_name: Option<&str>,
	token_override: Option<String>,
	token_fallback: Option<String>,
) -> Result<Option<ConfigTarget>, CliError> {
	let Some(path) = wikid_server::config::discover(config_arg.map(Path::new)) else {
		return Ok(None);
	};
	let config = wikid_server::Config::load(&path).map_err(|err| CliError::new("config", format!("{err:#}"), None))?;
	warn_insecure_config(&path, &config);
	if let Some(name) = requested_name {
		return named_config_target(&config, name, token_override, token_fallback, &path).map(Some);
	}
	let cwd = std::env::current_dir()
		.map_err(io_error)?
		.canonicalize()
		.map_err(io_error)?;
	if let Some((name, local_path)) = config
		.wikis
		.iter()
		.filter_map(|(name, path)| path.canonicalize().ok().map(|path| (name, path)))
		.filter(|(_, path)| cwd.starts_with(path))
		.max_by_key(|(_, path)| path.components().count())
	{
		return Ok(Some(ConfigTarget::Local {
			name: name.clone(),
			path: local_path,
			is_default: config.default_wiki.as_deref() == Some(name),
		}));
	}
	let target_count = config.wikis.len() + config.remotes.len();
	if target_count == 0 {
		return Ok(None);
	}
	if target_count == 1 {
		if let Some((name, local_path)) = config.wikis.iter().next() {
			return Ok(Some(ConfigTarget::Local {
				name: name.clone(),
				path: local_path.clone(),
				is_default: config.default_wiki.as_deref() == Some(name),
			}));
		}
		let (name, remote) = config.remotes.iter().next().unwrap();
		return Ok(Some(remote_config_target(
			name,
			remote,
			token_override,
			token_fallback,
			config.default_wiki.as_deref() == Some(name),
		)));
	}
	if let Some(default) = &config.default_wiki
		&& (config.wikis.contains_key(default) || config.remotes.contains_key(default))
	{
		return named_config_target(&config, default, token_override, token_fallback, &path).map(Some);
	}
	let names = config_target_names(&config).join(", ");
	Err(CliError::new(
		"ambiguous_wiki",
		format!("multiple local/remote targets registered: {names}"),
		Some(format!(
			"set default_wiki in {} or run wikid --target <name>",
			path.display()
		)),
	))
}

fn named_config_target(
	config: &wikid_server::Config,
	name: &str,
	token_override: Option<String>,
	token_fallback: Option<String>,
	config_path: &Path,
) -> Result<ConfigTarget, CliError> {
	let is_default = config.default_wiki.as_deref() == Some(name);
	match (config.wikis.get(name), config.remotes.get(name)) {
		(Some(_), Some(_)) => Err(CliError::new(
			"ambiguous_wiki",
			format!("target name {name:?} is configured as both local and remote"),
			Some(format!(
				"rename one of the duplicate targets in {}",
				config_path.display()
			)),
		)),
		(Some(path), None) => Ok(ConfigTarget::Local {
			name: name.to_owned(),
			path: path.clone(),
			is_default,
		}),
		(None, Some(remote)) => Ok(remote_config_target(
			name,
			remote,
			token_override,
			token_fallback,
			is_default,
		)),
		(None, None) => Err(CliError::new(
			"unknown_wiki",
			format!(
				"unknown configured target {name:?}; available: {}",
				config_target_names(config).join(", ")
			),
			Some(format!(
				"run wikid config list --config {}; use --server with --wiki for a direct daemon wiki",
				config_path.display()
			)),
		)),
	}
}

fn remote_config_target(
	name: &str,
	remote: &wikid_server::config::RemoteProfile,
	token_override: Option<String>,
	token_fallback: Option<String>,
	is_default: bool,
) -> ConfigTarget {
	let has_token_override = token_override.is_some();
	ConfigTarget::Remote {
		name: name.to_owned(),
		server: remote.server.clone(),
		token: token_override.or_else(|| remote.token.clone()).or(token_fallback),
		wiki: remote.wiki.clone().unwrap_or_else(|| name.to_owned()),
		is_default,
		token_override: has_token_override,
	}
}

fn config_target_names(config: &wikid_server::Config) -> Vec<String> {
	let mut names = config
		.wikis
		.keys()
		.chain(config.remotes.keys())
		.cloned()
		.collect::<Vec<_>>();
	names.sort();
	names.dedup();
	names
}

#[cfg(unix)]
fn warn_insecure_config(path: &Path, config: &wikid_server::Config) {
	use std::os::unix::fs::PermissionsExt as _;
	let has_secrets = !config.tokens.is_empty() || config.remotes.values().any(|remote| remote.token.is_some());
	let is_exposed = std::fs::metadata(path)
		.map(|metadata| metadata.permissions().mode() & 0o077 != 0)
		.unwrap_or(false);
	if has_secrets && is_exposed {
		eprintln!(
			"warning: config {} contains tokens and is accessible by group/other users; run chmod 600 {}",
			path.display(),
			path.display()
		);
	}
}

#[cfg(not(unix))]
fn warn_insecure_config(_path: &Path, _config: &wikid_server::Config) {}

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

fn render_config_list(result: &ConfigListResult) -> String {
	let mut lines = vec![
		format!("config: {}", result.config_path),
		format!("bind: {}", result.bind),
		format!("default: {}", result.default_wiki.as_deref().unwrap_or("(none)")),
	];
	let mut local_count = 0;
	let mut remote_count = 0;
	for target in &result.targets {
		match target {
			ConfigListTarget::Local { name, path } => {
				local_count += 1;
				lines.push(format!("local  {name}  {path}"));
			}
			ConfigListTarget::Remote { name, server, wiki } => {
				remote_count += 1;
				lines.push(format!("remote  {name}  {server}  wiki={wiki}"));
			}
		}
	}
	let target_label = if result.targets.len() == 1 { "target" } else { "targets" };
	lines.push(format!(
		"total: {} {target_label} ({local_count} local, {remote_count} remote)",
		result.targets.len()
	));
	lines.push("hint: wikid --target <name> status — inspect one configured target".to_owned());
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

Run `wikid skills get core` before using the CLI; it is the version-matched usage guide for reading, editing, linking, tags, doctor, and remote mode.
"#;

/// The targeted wiki: a local directory or a remote daemon. Both expose the
/// same operations and shared core structs; the client human view adds target
/// provenance while JSON keeps the operation wire shapes (DESIGN §6).
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

fn dispatch(backend: &Backend, target: &TargetContext, command: Command, json: bool) -> Result<Outcome, CliError> {
	let result: Result<Outcome, CliError> = match command {
		Command::Skills { .. }
		| Command::Serve
		| Command::Init { .. }
		| Command::Config { .. }
		| Command::Token { .. }
		| Command::Update { .. } => unreachable!("handled in run()"),
		Command::Status => {
			let status = backend.status()?;
			Ok(Outcome::ok(emit(json, &status, || target.focused_status(&status))))
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
				text: emit(json, &result, || {
					render::grep(&result, &pattern, files_only, ignore_case)
				}),
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
	};
	let mut outcome = result?;
	if !json {
		outcome.text = target.with_target_hints(outcome.text);
	}
	Ok(outcome)
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

	#[test]
	fn core_skill_mentions_every_clap_subcommand() {
		let core = skills::find("core").unwrap();
		let content = skills::content(core, true);
		for subcommand in Cli::command().get_subcommands() {
			let name = subcommand.get_name();
			if name == "help" {
				continue;
			}
			assert!(content.contains(name), "core skill must mention subcommand {name}");
		}
	}
}
