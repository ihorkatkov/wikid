//! Deterministic boxd-level worktree creation for wikid.
//!
//! `boxd-worktree create <branch>` forks a warm golden VM, creates a new git
//! branch from the golden's checked-out commit, restarts `wikid serve`, and
//! prints AXI-shaped human output or one JSON object with `--json`.

use std::ffi::OsStr;
use std::process::{Command, ExitStatus};

use clap::{Parser, Subcommand};
use serde::Serialize;

const DEFAULT_GOLDEN: &str = "wikid-golden";
const DEFAULT_REPO_PATH: &str = "/home/boxd/wikid";
const DEFAULT_WIKI_PATH: &str = "/home/boxd/wikis/wiki";
const DEFAULT_PORT: u16 = 7448;

#[derive(Parser)]
#[command(name = "boxd-worktree", version, about = "Create fast boxd forks as wikid worktrees")]
struct Cli {
	/// Emit one JSON object instead of human text
	#[arg(long, global = true)]
	json: bool,

	#[command(subcommand)]
	command: Action,
}

#[derive(Subcommand)]
enum Action {
	/// Fork the golden VM and create a new branch from its current commit
	Create {
		/// New branch name to create in the fork
		branch: String,

		/// Golden VM to fork
		#[arg(long, default_value = DEFAULT_GOLDEN)]
		golden: String,

		/// New VM name; defaults to `wikid-<sanitized-branch>`
		#[arg(long)]
		name: Option<String>,

		/// Repo path inside the VM
		#[arg(long, default_value = DEFAULT_REPO_PATH)]
		repo_path: String,

		/// Wiki directory served by wikid
		#[arg(long, default_value = DEFAULT_WIKI_PATH)]
		wiki_path: String,

		/// HTTP port served by wikid
		#[arg(long, default_value_t = DEFAULT_PORT)]
		port: u16,
	},
}

#[derive(Serialize)]
struct CreateOutput {
	status: &'static str,
	vm: String,
	branch: String,
	golden: String,
	url: String,
	ssh: String,
	repo_path: String,
	wiki_path: String,
	port: u16,
	commit: String,
	next: Vec<String>,
}

#[derive(Serialize)]
struct ErrorOutput {
	status: &'static str,
	error: String,
	next: Vec<String>,
}

fn main() {
	let cli = Cli::parse();
	let result = match cli.command {
		Action::Create {
			branch,
			golden,
			name,
			repo_path,
			wiki_path,
			port,
		} => create(branch, golden, name, repo_path, wiki_path, port),
	};

	match result {
		Ok(output) => print_create(output, cli.json),
		Err(err) => {
			print_error(&err, cli.json);
			std::process::exit(1);
		}
	}
}

fn create(
	branch: String,
	golden: String,
	name: Option<String>,
	repo_path: String,
	wiki_path: String,
	port: u16,
) -> Result<CreateOutput, String> {
	validate_branch(&branch)?;
	let vm = name.unwrap_or_else(|| format!("wikid-{}", sanitize_name(&branch)));
	validate_vm_name(&vm)?;

	if boxd_info(&vm).is_ok() {
		return Err(format!("VM '{vm}' already exists; refusing to overwrite"));
	}
	boxd_info(&golden).map_err(|err| format!("golden VM '{golden}' is not available: {err}"))?;

	run(
		"boxd",
		[
			"fork",
			golden.as_str(),
			"--name",
			vm.as_str(),
			"--auto-suspend-timeout",
			"0",
		],
	)?;
	let setup_result = configure_fork(&vm, &branch, &repo_path, &wiki_path, port);
	if let Err(err) = setup_result {
		let _ = run("boxd", ["destroy", vm.as_str(), "-y"]);
		return Err(format!("fork setup failed and VM '{vm}' was destroyed: {err}"));
	}

	let commit = boxd_exec(&vm, format!("cd {} && git rev-parse --short HEAD", sh(&repo_path)))?
		.trim()
		.to_owned();
	Ok(CreateOutput {
		status: "ok",
		vm: vm.clone(),
		branch,
		golden,
		url: format!("https://{vm}.boxd.sh"),
		ssh: format!("ssh {vm}.boxd"),
		repo_path,
		wiki_path,
		port,
		commit,
		next: vec![
			format!("ssh {vm}.boxd"),
			"edit, validate, commit, and push from the fork".to_owned(),
		],
	})
}

fn configure_fork(vm: &str, branch: &str, repo_path: &str, wiki_path: &str, port: u16) -> Result<(), String> {
	let script = format!(
		"set -eu\n\
		 cd {repo}\n\
		 git fetch --prune origin\n\
		 if git show-ref --verify --quiet refs/heads/{branch_ref}; then\n\
		   echo 'branch already exists in fork: {branch}' >&2\n\
		   exit 11\n\
		 fi\n\
		 if git ls-remote --exit-code --heads origin {branch_q} >/dev/null 2>&1; then\n\
		   echo 'remote branch already exists: {branch}' >&2\n\
		   exit 12\n\
		 fi\n\
		 git checkout -b {branch_q}\n\
		 mkdir -p {wiki_parent}\n\
		 if [ ! -f {wiki}/index.md ]; then\n\
		   /home/boxd/.cargo/bin/wikid init {wiki}\n\
		 fi\n\
		 pkill -x wikid >/dev/null 2>&1 || true\n\
		 nohup /home/boxd/.cargo/bin/wikid --dir {wiki} serve >/tmp/wikid.log 2>&1 &\n\
		 for i in $(seq 1 20); do\n\
		   curl -fsS http://127.0.0.1:{port}/health >/dev/null 2>&1 && break\n\
		   sleep 0.25\n\
		 done\n\
		 curl -fsS http://127.0.0.1:{port}/health >/dev/null",
		repo = sh(repo_path),
		branch_ref = branch.replace('"', ""),
		branch = branch,
		branch_q = sh(branch),
		wiki = sh(wiki_path),
		wiki_parent = sh(parent_path(wiki_path)),
		port = port,
	);
	boxd_exec(vm, format!("bash -lc {}", sh(&script)))?;
	run("boxd", ["proxy", "set-port", "--vm", vm, "--port", &port.to_string()])?;
	Ok(())
}

fn boxd_info(vm: &str) -> Result<String, String> {
	run("boxd", ["info", vm, "--json"])
}

fn boxd_exec(vm: &str, command: String) -> Result<String, String> {
	run("boxd", ["exec", vm, "--", command.as_str()])
}

fn run<I, S>(program: &str, args: I) -> Result<String, String>
where
	I: IntoIterator<Item = S>,
	S: AsRef<OsStr>,
{
	let output = Command::new(program)
		.args(args)
		.output()
		.map_err(|err| format!("failed to run {program}: {err}"))?;
	command_result(program, output.status, &output.stdout, &output.stderr)
}

fn command_result(program: &str, status: ExitStatus, stdout: &[u8], stderr: &[u8]) -> Result<String, String> {
	let out = String::from_utf8_lossy(stdout).trim().to_owned();
	let err = String::from_utf8_lossy(stderr).trim().to_owned();
	if status.success() {
		Ok(out)
	} else if err.is_empty() {
		Err(format!("{program} exited with {status}: {out}"))
	} else {
		Err(format!("{program} exited with {status}: {err}"))
	}
}

fn validate_branch(branch: &str) -> Result<(), String> {
	let chars_ok = branch
		.bytes()
		.all(|b| b.is_ascii_alphanumeric() || matches!(b, b'/' | b'.' | b'_' | b'-'));
	if branch.is_empty()
		|| !chars_ok
		|| branch.starts_with('-')
		|| branch.starts_with('/')
		|| branch.contains("..")
		|| branch.contains("//")
		|| branch.ends_with('/')
		|| branch.ends_with('.')
		|| branch.ends_with(".lock")
	{
		return Err(format!("invalid branch name: {branch}"));
	}
	Ok(())
}

fn validate_vm_name(name: &str) -> Result<(), String> {
	let valid = !name.is_empty()
		&& name.len() <= 63
		&& name
			.bytes()
			.all(|b| b.is_ascii_lowercase() || b.is_ascii_digit() || b == b'-')
		&& !name.starts_with('-')
		&& !name.ends_with('-');
	if valid {
		Ok(())
	} else {
		Err(format!("invalid VM name: {name}"))
	}
}

fn sanitize_name(branch: &str) -> String {
	let mut out = String::with_capacity(branch.len());
	let mut last_dash = false;
	for ch in branch.chars() {
		let lower = ch.to_ascii_lowercase();
		if lower.is_ascii_alphanumeric() {
			out.push(lower);
			last_dash = false;
		} else if !last_dash {
			out.push('-');
			last_dash = true;
		}
	}
	let trimmed = out.trim_matches('-');
	if trimmed.is_empty() {
		"worktree".to_owned()
	} else {
		trimmed.chars().take(57).collect()
	}
}

fn parent_path(path: &str) -> &str {
	path.rsplit_once('/')
		.map_or("/home/boxd", |(parent, _)| if parent.is_empty() { "/" } else { parent })
}

fn sh(value: &str) -> String {
	format!("'{}'", value.replace('\'', "'\\''"))
}

fn print_create(output: CreateOutput, json: bool) {
	if json {
		println!("{}", serde_json::to_string(&output).expect("serialize create output"));
		return;
	}
	println!("created: {}", output.vm);
	println!("branch: {} @ {}", output.branch, output.commit);
	println!("url: {}", output.url);
	println!("ssh: {}", output.ssh);
	println!("repo: {}", output.repo_path);
	println!("serve: {} on port {}", output.wiki_path, output.port);
	println!("next: {}", output.next.join("; "));
}

fn print_error(error: &str, json: bool) {
	if json {
		let output = ErrorOutput {
			status: "error",
			error: error.to_owned(),
			next: vec!["fix the reported condition and retry; existing VMs are never overwritten".to_owned()],
		};
		println!("{}", serde_json::to_string(&output).expect("serialize error output"));
		return;
	}
	println!("error: {error}");
	println!("next: fix the reported condition and retry; existing VMs are never overwritten");
}

#[cfg(test)]
mod tests {
	use super::{sanitize_name, validate_branch, validate_vm_name};

	#[test]
	fn sanitizes_branch_for_vm_name() {
		assert_eq!(sanitize_name("neo/123_Add CLI"), "neo-123-add-cli");
		assert_eq!(sanitize_name("---"), "worktree");
	}

	#[test]
	fn validates_branch_names() {
		assert!(validate_branch("neo/123-add_cli.v1").is_ok());
		assert!(validate_branch("bad branch").is_err());
		assert!(validate_branch("bad;branch").is_err());
		assert!(validate_branch("bad..branch").is_err());
	}

	#[test]
	fn validates_vm_names() {
		assert!(validate_vm_name("wikid-neo-123").is_ok());
		assert!(validate_vm_name("Wikid-neo-123").is_err());
		assert!(validate_vm_name("-wikid").is_err());
	}
}
