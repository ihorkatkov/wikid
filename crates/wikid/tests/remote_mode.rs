//! End-to-end remote mode (DESIGN §8): spawn the compiled binary's `serve` on
//! an ephemeral port with a temp config + token, run the CLI in remote mode
//! against it, and assert output parity with local mode on the same vault —
//! human and `--json` alike, including exit codes and error rendering.

use std::fs;
use std::net::TcpListener;
use std::path::Path;
use std::process::{Child, Command as ServeCommand, Stdio};
use std::time::{Duration, Instant};

use assert_cmd::Command;
use tempfile::TempDir;

const TOKEN: &str = "wkd_e2e_token";
const WIKI: &str = "main";

fn fixture_vault() -> TempDir {
	let dir = TempDir::new().expect("create temp vault");
	let root = dir.path();
	fs::create_dir_all(root.join("notes")).unwrap();
	fs::write(root.join("index.md"), "# Home\n\nSee [[alpha]] and [[missing]].\n").unwrap();
	fs::write(
		root.join("notes/alpha.md"),
		"# Alpha\n\nThe needle is here.\nAnother needle line.\n",
	)
	.unwrap();
	fs::write(root.join("notes/beta.md"), "# Beta\n\nplain text\n").unwrap();
	dir
}

/// Kills the spawned `wikid serve` child when the test ends (pass or panic).
struct ServerGuard {
	child: Child,
	/// Base URL of the daemon, e.g. `http://127.0.0.1:49312`.
	base: String,
	_config_dir: TempDir,
}

impl Drop for ServerGuard {
	fn drop(&mut self) {
		let _ = self.child.kill();
		let _ = self.child.wait();
	}
}

/// Picks a free port by binding then dropping; the serve child re-binds it
/// right after, before anything else in this process asks for a port.
fn free_port() -> u16 {
	TcpListener::bind("127.0.0.1:0")
		.expect("bind ephemeral port")
		.local_addr()
		.unwrap()
		.port()
}

fn spawn_server(vault: &Path) -> ServerGuard {
	let config_dir = TempDir::new().unwrap();
	let port = free_port();
	let config_path = config_dir.path().join("wikid.toml");
	// `{:?}` on the strings produces valid TOML basic strings (escaped quotes
	// and backslashes), so arbitrary temp paths survive the round trip.
	fs::write(
		&config_path,
		format!(
			"bind = \"127.0.0.1:{port}\"\n\n[wikis]\n{WIKI} = {:?}\n\n[tokens]\n{TOKEN:?} = \"e2e-test\"\n",
			vault.to_str().unwrap(),
		),
	)
	.unwrap();
	let child = ServeCommand::new(env!("CARGO_BIN_EXE_wikid"))
		.arg("serve")
		.arg("--config")
		.arg(&config_path)
		.stdout(Stdio::null())
		.stderr(Stdio::null())
		.spawn()
		.expect("spawn wikid serve");
	let guard = ServerGuard {
		child,
		base: format!("http://127.0.0.1:{port}"),
		_config_dir: config_dir,
	};
	wait_for_health(&guard.base);
	guard
}

fn spawn_server_with_config(config_path: &Path, base: String, config_dir: TempDir) -> ServerGuard {
	let child = ServeCommand::new(env!("CARGO_BIN_EXE_wikid"))
		.arg("serve")
		.arg("--config")
		.arg(config_path)
		.stdout(Stdio::null())
		.stderr(Stdio::null())
		.spawn()
		.expect("spawn wikid serve");
	let guard = ServerGuard {
		child,
		base,
		_config_dir: config_dir,
	};
	wait_for_health(&guard.base);
	guard
}

fn wait_for_health(base: &str) {
	let url = format!("{base}/health");
	let deadline = Instant::now() + Duration::from_secs(10);
	while ureq::get(&url).call().is_err() {
		assert!(Instant::now() < deadline, "wikid serve did not answer {url} within 10s");
		std::thread::sleep(Duration::from_millis(25));
	}
}

fn clear_env(cmd: &mut Command) {
	for var in ["WIKID_DIR", "WIKID_SERVER", "WIKID_TOKEN", "WIKID_WIKI", "WIKID_CONFIG"] {
		cmd.env_remove(var);
	}
}

/// Runs the CLI in local mode against the vault; returns (stdout, exit code).
fn local(vault: &Path, args: &[&str]) -> (String, i32) {
	let mut cmd = Command::cargo_bin("wikid").expect("binary builds");
	clear_env(&mut cmd);
	cmd.arg("--dir").arg(vault);
	run_cli(cmd, args)
}

/// Runs the CLI in remote mode against the daemon; returns (stdout, exit code).
fn remote(base: &str, args: &[&str]) -> (String, i32) {
	let mut cmd = Command::cargo_bin("wikid").expect("binary builds");
	clear_env(&mut cmd);
	cmd.args(["--server", base, "--token", TOKEN, "--wiki", WIKI]);
	run_cli(cmd, args)
}

fn run_cli(mut cmd: Command, args: &[&str]) -> (String, i32) {
	let output = cmd.args(args).output().expect("run wikid");
	(
		String::from_utf8(output.stdout).expect("stdout is UTF-8"),
		output.status.code().expect("exit code"),
	)
}

/// Asserts a command renders byte-identically in both modes (human and
/// `--json`) with the same exit code. Only safe for non-mutating commands —
/// both invocations hit the same vault.
fn assert_read_parity(vault: &Path, base: &str, args: &[&str]) {
	let (local_out, local_code) = local(vault, args);
	let (remote_out, remote_code) = remote(base, args);
	assert_eq!(remote_out, local_out, "human parity for {args:?}");
	assert_eq!(remote_code, local_code, "exit code parity for {args:?}");
	let mut json_args = args.to_vec();
	json_args.push("--json");
	let (local_json, _) = local(vault, &json_args);
	let (remote_json, _) = remote(base, &json_args);
	assert_eq!(remote_json, local_json, "--json parity for {args:?}");
}

#[test]
fn init_then_serve_then_remote_status_end_to_end() {
	let home = TempDir::new().unwrap();
	let vault = TempDir::new().unwrap();
	let mut init = Command::cargo_bin("wikid").expect("binary builds");
	clear_env(&mut init);
	init.env("HOME", home.path())
		.arg("init")
		.arg(vault.path())
		.assert()
		.success();
	let config_path = home.path().join(".config/wikid/config.toml");
	let token_stdout = {
		let mut token_cmd = Command::cargo_bin("wikid").expect("binary builds");
		clear_env(&mut token_cmd);
		let output = token_cmd
			.env("HOME", home.path())
			.args(["token", "show"])
			.output()
			.unwrap();
		assert!(output.status.success());
		String::from_utf8(output.stdout).unwrap()
	};
	let token = token_stdout.lines().next().unwrap().to_owned();
	let port = free_port();
	let mut config = wikid_server::Config::load(&config_path).unwrap();
	config.bind = format!("127.0.0.1:{port}");
	wikid_server::config::save(&config_path, &config).unwrap();
	let server = spawn_server_with_config(&config_path, format!("http://127.0.0.1:{port}"), home);
	let wiki = config.default_wiki.as_deref().unwrap();
	let mut remote_status = Command::cargo_bin("wikid").expect("binary builds");
	clear_env(&mut remote_status);
	remote_status
		.args([
			"--server",
			server.base.as_str(),
			"--token",
			token.as_str(),
			"--wiki",
			wiki,
			"status",
		])
		.assert()
		.success()
		.stdout(predicates::str::contains(
			vault.path().canonicalize().unwrap().display().to_string(),
		));
}

#[test]
fn remote_mode_matches_local_mode_end_to_end() {
	let vault = fixture_vault();
	let server = spawn_server(vault.path());
	let base = server.base.as_str();

	// Read commands: identical rendering, totals, hints, and exit codes.
	let read_commands: &[&[&str]] = &[
		&["status"],
		&["ls"],
		&["ls", "notes"],
		&["tree"],
		&["cat", "index.md"],
		&["cat", "index.md", "--full"],
		&["grep", "needle"],
		&["grep", "NEEDLE", "-i", "-l"],
		&["grep", "is here", "-C", "1", "--limit", "1"],
		&["grep", "zzz-not-there"], // zero matches: exit 1 in both modes
		&["glob", "**/*.md"],
		&["links", "index.md"],
		&["doctor"],
		&["doctor", "--stale-days", "0", "--checks", "stale_pages,broken_links"],
	];
	for args in read_commands {
		assert_read_parity(vault.path(), base, args);
	}

	// write: goes through the daemon onto disk; output matches a local write
	// of the very same file (delete in between so both report "created").
	let (remote_write, code) = remote(base, &["write", "drafts/e2e.md", "-m", "written remotely"]);
	assert_eq!(code, 0);
	assert_eq!(
		fs::read_to_string(vault.path().join("drafts/e2e.md")).unwrap(),
		"written remotely\n"
	);
	local(vault.path(), &["rm", "drafts/e2e.md", "--force"]);
	let (local_write, code) = local(vault.path(), &["write", "drafts/e2e.md", "-m", "written remotely"]);
	assert_eq!(code, 0);
	assert_eq!(remote_write, local_write, "write parity");

	// edit: a same-length swap back and forth keeps byte counts (and thus the
	// rendered output) identical across the two invocations.
	let (remote_edit, code) = remote(
		base,
		&["edit", "notes/alpha.md", "--old", "The needle", "--new", "The peedle"],
	);
	assert_eq!(code, 0);
	let (local_edit, code) = local(
		vault.path(),
		&["edit", "notes/alpha.md", "--old", "The peedle", "--new", "The needle"],
	);
	assert_eq!(code, 0);
	assert_eq!(remote_edit, local_edit, "edit parity");

	// mv: a remote move renders exactly like the same local move. Restore the
	// file between invocations so both runs move the same source to the same
	// destination (byte-identical output requires identical from/to).
	let (remote_mv, code) = remote(base, &["mv", "notes/alpha.md", "notes/alpha2.md"]);
	assert_eq!(code, 0);
	assert!(vault.path().join("notes/alpha2.md").exists(), "remote mv moved on disk");
	assert!(!vault.path().join("notes/alpha.md").exists());
	local(vault.path(), &["mv", "notes/alpha2.md", "notes/alpha.md"]);
	let (local_mv, code) = local(vault.path(), &["mv", "notes/alpha.md", "notes/alpha2.md"]);
	assert_eq!(code, 0);
	assert_eq!(remote_mv, local_mv, "mv parity");
	local(vault.path(), &["mv", "notes/alpha2.md", "notes/alpha.md"]);
	let (remote_mv_json, code) = remote(base, &["mv", "notes/alpha.md", "notes/alpha2.md", "--json"]);
	assert_eq!(code, 0);
	local(vault.path(), &["mv", "notes/alpha2.md", "notes/alpha.md"]);
	let (local_mv_json, code) = local(vault.path(), &["mv", "notes/alpha.md", "notes/alpha2.md", "--json"]);
	assert_eq!(code, 0);
	assert_eq!(remote_mv_json, local_mv_json, "mv --json parity");
	local(vault.path(), &["mv", "notes/alpha2.md", "notes/alpha.md"]);

	// mv onto an existing destination without --force: refused identically
	// (already_exists over the wire), and nothing moves.
	let (remote_clobber, remote_code) = remote(base, &["mv", "notes/alpha.md", "index.md"]);
	let (local_clobber, local_code) = local(vault.path(), &["mv", "notes/alpha.md", "index.md"]);
	assert_eq!(remote_code, 1);
	assert!(remote_clobber.starts_with("error[already_exists]:"), "{remote_clobber}");
	assert_eq!((remote_clobber, remote_code), (local_clobber, local_code));
	assert!(vault.path().join("notes/alpha.md").exists(), "refused mv must not move");

	// rm: the --force refusal is identical (gated client-side in both modes),
	// and the deletion renders identically.
	let (remote_refusal, remote_code) = remote(base, &["rm", "notes/beta.md"]);
	let (local_refusal, local_code) = local(vault.path(), &["rm", "notes/beta.md"]);
	assert_eq!(remote_code, 1);
	assert_eq!((remote_refusal, remote_code), (local_refusal, local_code));
	assert!(vault.path().join("notes/beta.md").exists());
	let (remote_rm, code) = remote(base, &["rm", "notes/beta.md", "--force"]);
	assert_eq!(code, 0);
	assert!(!vault.path().join("notes/beta.md").exists());
	local(vault.path(), &["write", "notes/beta.md", "-m", "temp"]);
	let (local_rm, code) = local(vault.path(), &["rm", "notes/beta.md", "--force"]);
	assert_eq!(code, 0);
	assert_eq!(remote_rm, local_rm, "rm parity");

	// Core errors travel the wire and render exactly like local mode.
	let (remote_err, remote_code) = remote(base, &["cat", "nope.md"]);
	let (local_err, local_code) = local(vault.path(), &["cat", "nope.md"]);
	assert_eq!((remote_err, remote_code), (local_err, local_code));
	let (remote_err, remote_code) = remote(base, &["cat", "nope.md", "--json"]);
	let (local_err, local_code) = local(vault.path(), &["cat", "nope.md", "--json"]);
	assert_eq!((remote_err, remote_code), (local_err, local_code));
	let (remote_err, _) = remote(base, &["edit", "notes/alpha.md", "--old", "needle", "--new", "pin"]);
	let (local_err, _) = local(
		vault.path(),
		&["edit", "notes/alpha.md", "--old", "needle", "--new", "pin"],
	);
	assert_eq!(remote_err, local_err, "ambiguous-edit error parity");
}

/// Remote targeting purely via `WIKID_SERVER`/`WIKID_TOKEN`/`WIKID_WIKI` env
/// vars — no flags — reaches the daemon and renders exactly like local mode.
#[test]
fn env_vars_alone_target_the_remote_daemon() {
	let vault = fixture_vault();
	let server = spawn_server(vault.path());

	let mut cmd = Command::cargo_bin("wikid").unwrap();
	clear_env(&mut cmd);
	let output = cmd
		.env("WIKID_SERVER", &server.base)
		.env("WIKID_TOKEN", TOKEN)
		.env("WIKID_WIKI", WIKI)
		.arg("status")
		.output()
		.unwrap();
	assert_eq!(output.status.code(), Some(0), "env-var targeting must reach the daemon");
	let remote_out = String::from_utf8(output.stdout).unwrap();
	let (local_out, _) = local(vault.path(), &["status"]);
	assert_eq!(remote_out, local_out, "env-var remote status matches local");
}

#[test]
fn remote_auth_and_wiki_errors_render_structured() {
	let vault = fixture_vault();
	let server = spawn_server(vault.path());
	let base = server.base.as_str();

	// Wrong token → the daemon's 401 body, rendered as a structured error.
	let mut cmd = Command::cargo_bin("wikid").unwrap();
	clear_env(&mut cmd);
	let output = cmd
		.args(["--server", base, "--token", "wrong", "--wiki", WIKI, "status"])
		.output()
		.unwrap();
	assert_eq!(output.status.code(), Some(1));
	let stdout = String::from_utf8(output.stdout).unwrap();
	assert!(
		stdout.starts_with("error[unauthorized]:"),
		"unexpected output: {stdout}"
	);
	assert!(stdout.contains("hint:"), "401 must carry the daemon's hint: {stdout}");

	// Missing token entirely → same 401 path.
	let mut cmd = Command::cargo_bin("wikid").unwrap();
	clear_env(&mut cmd);
	let output = cmd.args(["--server", base, "--wiki", WIKI, "status"]).output().unwrap();
	assert_eq!(output.status.code(), Some(1));
	let stdout = String::from_utf8(output.stdout).unwrap();
	assert!(stdout.starts_with("error[unauthorized]:"), "{stdout}");

	// Unknown wiki → 404 body listing the available names.
	let mut cmd = Command::cargo_bin("wikid").unwrap();
	clear_env(&mut cmd);
	let output = cmd
		.args(["--server", base, "--token", TOKEN, "--wiki", "nope", "status"])
		.output()
		.unwrap();
	assert_eq!(output.status.code(), Some(1));
	let stdout = String::from_utf8(output.stdout).unwrap();
	assert!(stdout.starts_with("error[unknown_wiki]:"), "{stdout}");
	assert!(stdout.contains(WIKI), "available wikis must be listed: {stdout}");
}
