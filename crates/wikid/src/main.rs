//! The `wikid` binary: one clap surface over two modes (DESIGN §6). Local
//! mode calls `wikid-core` directly against `--dir`/`$WIKID_DIR`; remote mode
//! speaks the HTTP API against `--server`/`$WIKID_SERVER` through the same
//! rendering paths. `wikid serve` hosts `wikid-server` and is the CLI's only
//! async entry point.

mod error;
mod remote;
mod render;

use std::io::Read;
use std::path::Path;

use clap::{CommandFactory, Parser, Subcommand};
use serde::Serialize;
use wikid_core::{
	Check, DoctorOptions, Document, EditResult, GlobResult, GrepOptions, GrepResult, HealthReport, LinkReport, Listing,
	MvResult, ReadLimit, RmResult, Vault, VaultStatus, WriteResult,
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

	/// Emit the result as one JSON object instead of human text
	#[arg(long, global = true)]
	json: bool,

	#[command(subcommand)]
	command: Option<Command>,
}

#[derive(Subcommand)]
enum Command {
	/// Run the daemon serving configured wikis
	Serve {
		/// Config file ($WIKID_CONFIG → ./wikid.toml → ~/.config/wikid/config.toml)
		#[arg(long, value_name = "PATH")]
		config: Option<String>,
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
	/// Read a page
	Cat {
		path: String,
		/// Print the whole file instead of the first 400 lines / 32 KiB
		#[arg(long)]
		full: bool,
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
	/// Surgically edit a page (literal string replace)
	Edit {
		path: String,
		/// Exact text to replace (must match exactly once unless --all)
		#[arg(long, value_name = "TEXT")]
		old: String,
		/// Replacement text
		#[arg(long, value_name = "TEXT")]
		new: String,
		/// Replace every occurrence instead of requiring a unique match
		#[arg(long)]
		all: bool,
	},
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
	/// Show outgoing links and backlinks for a page
	Links { path: String },
	/// Run structural health checks
	Doctor {
		/// Pages not modified in this many days are stale (default 90)
		#[arg(long, value_name = "N")]
		stale_days: Option<u64>,
		/// Comma-separated subset of checks to run (e.g. broken_links,orphan_pages)
		#[arg(long, value_name = "a,b,c")]
		checks: Option<String>,
	},
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
			println!("{}", out.text);
			std::process::exit(out.code);
		}
		Err(err) => {
			println!("{}", if json { err.json() } else { err.human() });
			std::process::exit(1);
		}
	}
}

fn run(cli: Cli) -> Result<Outcome, CliError> {
	// AXI checklist item 1: no arguments shows live data, never help text.
	let command = cli.command.unwrap_or(Command::Status);
	if let Command::Serve { config } = command {
		return run_serve(config.as_deref());
	}
	let dir = cli.dir.or_else(|| env_var("WIKID_DIR"));
	let server = cli.server.or_else(|| env_var("WIKID_SERVER"));
	if dir.is_some() && server.is_some() {
		// Flag-vs-flag conflicts are caught by clap itself; this covers the
		// env-supplied combinations with the same usage error and exit code 2.
		Cli::command()
			.error(
				clap::error::ErrorKind::ArgumentConflict,
				"--dir/$WIKID_DIR and --server/$WIKID_SERVER cannot both be set",
			)
			.exit();
	}
	let backend = if let Some(server) = server {
		let token = cli.token.or_else(|| env_var("WIKID_TOKEN"));
		let wiki = cli
			.wiki
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
	} else {
		return Err(CliError::no_target());
	};
	dispatch(&backend, command, cli.json)
}

/// `wikid serve` (DESIGN §6): discover the config (arg → `$WIKID_CONFIG` →
/// `./wikid.toml` → `~/.config/wikid/config.toml`), then run `wikid-server`
/// on a tokio runtime until stopped. The rest of the CLI stays sync.
fn run_serve(config: Option<&str>) -> Result<Outcome, CliError> {
	let path = wikid_server::config::discover(config.map(Path::new)).ok_or_else(CliError::no_config)?;
	let config = wikid_server::Config::load(&path).map_err(|err| CliError::new("config", format!("{err:#}"), None))?;
	tracing_subscriber::fmt()
		.with_env_filter(
			tracing_subscriber::EnvFilter::try_from_default_env()
				.unwrap_or_else(|_| tracing_subscriber::EnvFilter::new("info")),
		)
		.init();
	let runtime = tokio::runtime::Runtime::new()
		.map_err(|err| CliError::new("io", format!("failed to start async runtime: {err}"), None))?;
	runtime
		.block_on(wikid_server::serve(config))
		.map_err(|err| CliError::new("serve", format!("{err:#}"), None))?;
	Ok(Outcome::ok("wikid-server stopped".to_owned()))
}

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

	fn cat(&self, path: &str, full: bool) -> Result<Document, CliError> {
		match self {
			Self::Local(vault) => {
				let limit = if full { None } else { Some(ReadLimit::default()) };
				Ok(vault.cat(path, limit)?)
			}
			Self::Remote(remote) => remote.cat(path, full),
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

	fn edit(&self, path: &str, old: &str, new: &str, all: bool) -> Result<EditResult, CliError> {
		match self {
			Self::Local(vault) => Ok(vault.edit(path, old, new, all)?),
			Self::Remote(remote) => remote.edit(path, old, new, all),
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
			Self::Local(vault) => Ok(vault.links(path)?),
			Self::Remote(remote) => remote.links(path),
		}
	}

	fn doctor(&self, stale_days: Option<u64>, checks: Option<&[Check]>) -> Result<HealthReport, CliError> {
		match self {
			Self::Local(vault) => {
				let mut opts = DoctorOptions::default();
				if let Some(days) = stale_days {
					opts.stale_days = days;
				}
				opts.checks = checks.map(<[Check]>::to_vec);
				Ok(vault.doctor(&opts)?)
			}
			Self::Remote(remote) => remote.doctor(stale_days, checks),
		}
	}
}

fn dispatch(backend: &Backend, command: Command, json: bool) -> Result<Outcome, CliError> {
	match command {
		Command::Serve { .. } => unreachable!("handled in run()"),
		Command::Status => {
			let status = backend.status()?;
			Ok(Outcome::ok(emit(json, &status, || render::status(&status))))
		}
		Command::Ls { path } => {
			let listing = backend.ls(path.as_deref(), 1)?;
			Ok(Outcome::ok(emit(json, &listing, || render::listing(&listing, false))))
		}
		Command::Tree { path, depth } => {
			let listing = backend.ls(path.as_deref(), depth)?;
			Ok(Outcome::ok(emit(json, &listing, || render::listing(&listing, true))))
		}
		Command::Cat { path, full } => {
			let doc = backend.cat(&path, full)?;
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
		Command::Edit { path, old, new, all } => {
			let result = backend.edit(&path, &old, &new, all)?;
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
		Command::Doctor { stale_days, checks } => {
			let checks = checks.map(|list| parse_checks(&list)).transpose()?;
			let report = backend.doctor(stale_days, checks.as_deref())?;
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
