//! Local-mode CLI integration tests (DESIGN §8): the AXI conformance
//! checklist item by item, `--json` validity for every command, exit codes,
//! and happy + error paths for the mutating commands, all through the real
//! binary against temp vaults.

use std::fs;
use std::path::{Path, PathBuf};

use assert_cmd::Command;
use predicates::prelude::*;
use tempfile::TempDir;

/// Builds a fixture vault: linked pages (incl. a broken link), a nested dir,
/// a page big enough to truncate, a binary attachment, and a hidden
/// `.obsidian/` dir that must stay invisible.
fn fixture_vault() -> TempDir {
	let dir = TempDir::new().expect("create temp vault");
	let root = dir.path();
	fs::create_dir_all(root.join("notes")).unwrap();
	fs::create_dir_all(root.join("assets")).unwrap();
	fs::create_dir_all(root.join(".obsidian")).unwrap();
	fs::write(root.join("index.md"), "# Home\n\nSee [[alpha]] and [[missing]].\n").unwrap();
	fs::write(
		root.join("notes/alpha.md"),
		"# Alpha\n\nThe needle is here.\nAnother needle line.\n",
	)
	.unwrap();
	fs::write(root.join("notes/beta.md"), "# Beta\n\nplain text\n").unwrap();
	fs::write(root.join(".obsidian/app.json"), "{}").unwrap();
	fs::write(root.join("assets/blob.bin"), b"\xff\xfe\x00binary").unwrap();
	let big: String = (1..=500).map(|i| format!("line {i}\n")).collect();
	fs::write(root.join("big.md"), big).unwrap();
	dir
}

/// A `wikid` invocation with hermetic env and explicit `--dir` targeting.
fn wikid(vault: &Path) -> Command {
	let mut cmd = Command::cargo_bin("wikid").expect("binary builds");
	clear_env(&mut cmd);
	cmd.arg("--dir").arg(vault);
	cmd
}

/// A `wikid` invocation with hermetic env and no targeting at all.
fn wikid_untargeted() -> Command {
	let mut cmd = Command::cargo_bin("wikid").expect("binary builds");
	clear_env(&mut cmd);
	cmd
}

fn clear_env(cmd: &mut Command) {
	for var in ["WIKID_DIR", "WIKID_SERVER", "WIKID_TOKEN", "WIKID_WIKI", "WIKID_CONFIG"] {
		cmd.env_remove(var);
	}
}

fn stdout_of(cmd: &mut Command) -> String {
	let output = cmd.output().expect("run wikid");
	String::from_utf8(output.stdout).expect("stdout is UTF-8")
}

/// Runs the command and parses stdout as one JSON object.
fn json_of(cmd: &mut Command) -> serde_json::Value {
	let stdout = stdout_of(cmd);
	serde_json::from_str(&stdout).unwrap_or_else(|e| panic!("stdout is not valid JSON ({e}): {stdout}"))
}

// --- AXI conformance checklist (DESIGN §6), one test per item ---

#[test]
fn axi_1_no_args_is_live_status_not_help() {
	let vault = fixture_vault();
	let mut cmd = wikid_untargeted();
	cmd.env("WIKID_DIR", vault.path());
	cmd.assert()
		.success()
		.stdout(predicate::str::contains("vault:"))
		.stdout(predicate::str::contains("pages: 4"))
		.stdout(predicate::str::contains("Usage").not());
}

#[test]
fn axi_2_list_items_are_compact_and_totals_always_present() {
	let vault = fixture_vault();
	let out = stdout_of(wikid(vault.path()).arg("ls"));
	assert!(out.contains("total: 2 dirs, 1 files, 4 pages"), "totals missing: {out}");
	for line in out
		.lines()
		.filter(|l| !l.starts_with("total:") && !l.starts_with("hint:"))
	{
		let fields = line.split("  ").count();
		assert!(fields <= 4, "list line has {fields} fields (> 4): {line}");
	}
}

#[test]
fn axi_3_cat_truncates_by_default_with_size_hint_and_full_override() {
	let vault = fixture_vault();
	let truncated = stdout_of(wikid(vault.path()).args(["cat", "big.md"]));
	assert!(
		truncated.contains("… truncated (500 lines / 4392 bytes total) — use --full"),
		"missing truncation marker: {truncated}"
	);
	assert!(!truncated.contains("line 500"), "truncated output leaked the tail");
	let full = stdout_of(wikid(vault.path()).args(["cat", "big.md", "--full"]));
	assert!(full.contains("line 500"), "--full did not return the whole file");
	assert!(!full.contains("truncated"), "--full still shows a truncation marker");
}

#[test]
fn axi_4_zero_results_are_explicit() {
	let vault = fixture_vault();
	wikid(vault.path())
		.args(["grep", "zzz-not-there"])
		.assert()
		.stdout(predicate::str::contains("no matches for \"zzz-not-there\" in 4 files"));
	wikid(vault.path())
		.args(["glob", "nothing/**"])
		.assert()
		.success()
		.stdout(predicate::str::contains("no matches for \"nothing/**\""));
}

#[test]
fn axi_5_structured_errors_on_stdout_and_exit_codes_0_1_2() {
	let vault = fixture_vault();
	// Exit 0: a successful read.
	wikid(vault.path()).args(["cat", "index.md"]).assert().success();
	// Exit 1: structured error on stdout, nothing on stderr.
	wikid(vault.path())
		.args(["cat", "nope.md"])
		.assert()
		.code(1)
		.stdout(predicate::str::starts_with("error[not_found]: not found: nope.md"))
		.stdout(predicate::str::contains("hint:"))
		.stderr(predicate::str::is_empty());
	// Exit 1: grep with zero matches (coreutils-faithful).
	wikid(vault.path()).args(["grep", "zzz-not-there"]).assert().code(1);
	wikid(vault.path()).args(["grep", "needle"]).assert().code(0);
	// Exit 2: usage errors via clap.
	wikid(vault.path()).args(["ls", "--bogus"]).assert().code(2);
	wikid(vault.path()).arg("unknown-command").assert().code(2);
}

#[test]
fn axi_6_rm_requires_force_and_never_prompts() {
	let vault = fixture_vault();
	let target = vault.path().join("notes/beta.md");
	wikid(vault.path())
		.args(["rm", "notes/beta.md"])
		.assert()
		.code(1)
		.stdout(predicate::str::starts_with("error[force_required]:"))
		.stdout(predicate::str::contains("--force"));
	assert!(target.exists(), "rm without --force must not delete");
	wikid(vault.path())
		.args(["rm", "notes/beta.md", "--force"])
		.assert()
		.success()
		.stdout(predicate::str::contains("removed notes/beta.md"));
	assert!(!target.exists(), "rm --force must delete");
}

#[test]
fn axi_7_human_output_ends_with_hints_json_has_none() {
	let vault = fixture_vault();
	// Every read command's human output must trail with hint: lines, and its
	// --json output must carry none.
	let commands: &[&[&str]] = &[
		&["status"],
		&["ls"],
		&["tree"],
		&["cat", "index.md"],
		&["grep", "needle"],
		&["glob", "**/*.md"],
		&["links", "index.md"],
		&["doctor"],
	];
	for args in commands {
		let human = stdout_of(wikid(vault.path()).args(*args));
		let hint_lines: Vec<&str> = human.lines().filter(|l| l.starts_with("hint:")).collect();
		assert!(
			(1..=2).contains(&hint_lines.len()),
			"{args:?}: expected 1-2 hint lines, got {}: {human}",
			hint_lines.len()
		);
		assert!(
			human.trim_end().lines().last().unwrap().starts_with("hint:"),
			"{args:?}: hints must trail the output: {human}"
		);
		let json = stdout_of(wikid(vault.path()).args(*args).arg("--json"));
		assert!(!json.contains("hint"), "{args:?}: --json must not carry hints: {json}");
	}
	// Hints name a concrete next command (AXI-7), e.g. grep points at cat.
	let grep = stdout_of(wikid(vault.path()).args(["grep", "needle"]));
	assert!(
		grep.contains("hint: wikid cat"),
		"grep hint names the next step: {grep}"
	);
}

// --- `--json` emits the serialized core struct for every command ---

#[test]
fn json_output_parses_for_every_command() {
	let vault = fixture_vault();
	let status = json_of(wikid(vault.path()).args(["status", "--json"]));
	assert_eq!(status["total_pages"], 4);
	let ls = json_of(wikid(vault.path()).args(["ls", "--json"]));
	assert!(ls["entries"].is_array());
	assert_eq!(ls["total_pages"], 4);
	let tree = json_of(wikid(vault.path()).args(["tree", "--json"]));
	assert!(
		tree["entries"].as_array().unwrap().len() > ls["entries"].as_array().unwrap().len(),
		"tree (depth 3) must list deeper than ls (depth 1)"
	);
	let cat = json_of(wikid(vault.path()).args(["cat", "index.md", "--json"]));
	assert_eq!(cat["path"], "index.md");
	assert!(cat["content"].as_str().unwrap().contains("[[alpha]]"));
	let grep = json_of(wikid(vault.path()).args(["grep", "needle", "--json"]));
	assert_eq!(grep["total_matches"], 2);
	assert_eq!(grep["matched_files"], 1, "both needles live in notes/alpha.md");
	assert_eq!(grep["total_files"], 4);
	let glob = json_of(wikid(vault.path()).args(["glob", "**/*.md", "--json"]));
	assert_eq!(glob["total"], 4);
	let write = json_of(wikid(vault.path()).args(["write", "new.md", "-m", "fresh", "--json"]));
	assert_eq!(write["created"], true);
	let edit = json_of(wikid(vault.path()).args(["edit", "new.md", "--old", "fresh", "--new", "stale", "--json"]));
	assert_eq!(edit["replacements"], 1);
	let mv = json_of(wikid(vault.path()).args(["mv", "new.md", "old.md", "--json"]));
	assert_eq!(mv["from"], "new.md");
	assert_eq!(mv["to"], "old.md");
	let rm = json_of(wikid(vault.path()).args(["rm", "old.md", "--force", "--json"]));
	assert_eq!(rm["path"], "old.md");
	let links = json_of(wikid(vault.path()).args(["links", "index.md", "--json"]));
	assert!(links["outgoing"].is_array());
	assert!(links["backlinks"].is_array());
	let doctor = json_of(wikid(vault.path()).args(["doctor", "--json"]));
	assert!(doctor["issues"].is_array());
	assert!(doctor["summary"].is_string());
}

#[test]
fn json_grep_zero_matches_still_exits_1_with_parseable_body() {
	let vault = fixture_vault();
	let output = wikid(vault.path())
		.args(["grep", "zzz-not-there", "--json"])
		.output()
		.unwrap();
	assert_eq!(output.status.code(), Some(1));
	let value: serde_json::Value = serde_json::from_slice(&output.stdout).expect("valid JSON");
	assert_eq!(value["total_matches"], 0);
}

#[test]
fn json_errors_use_the_error_object_shape() {
	let vault = fixture_vault();
	let output = wikid(vault.path()).args(["cat", "nope.md", "--json"]).output().unwrap();
	assert_eq!(output.status.code(), Some(1));
	let value: serde_json::Value = serde_json::from_slice(&output.stdout).expect("valid JSON");
	assert_eq!(value["error"]["code"], "not_found");
	assert!(value["error"]["message"].is_string());
	assert!(value["error"]["hint"].is_string());
}

// --- targeting: local dir, remote selection, serve config discovery ---

#[test]
fn no_target_is_a_structured_error() {
	wikid_untargeted()
		.arg("status")
		.assert()
		.code(1)
		.stdout(predicate::str::starts_with("error[no_target]:"))
		.stdout(predicate::str::contains("--dir"))
		.stdout(predicate::str::contains("--server"));
}

#[test]
fn dir_and_server_together_is_a_usage_error() {
	let vault = fixture_vault();
	// Flag + flag: caught by clap.
	wikid(vault.path())
		.args(["--server", "http://localhost:7448", "status"])
		.assert()
		.code(2);
	// Env + flag: same usage error and exit code.
	let mut cmd = wikid_untargeted();
	cmd.env("WIKID_DIR", vault.path())
		.args(["--server", "http://localhost:7448", "status"])
		.assert()
		.code(2);
}

#[test]
fn remote_mode_without_a_wiki_is_a_structured_error() {
	wikid_untargeted()
		.args(["--server", "http://localhost:7448", "--token", "t", "status"])
		.assert()
		.code(1)
		.stdout(predicate::str::starts_with("error[no_wiki]:"))
		.stdout(predicate::str::contains("--wiki"));
	let mut env_remote = wikid_untargeted();
	env_remote
		.env("WIKID_SERVER", "http://localhost:7448")
		.arg("status")
		.assert()
		.code(1)
		.stdout(predicate::str::starts_with("error[no_wiki]:"));
}

#[test]
fn remote_mode_reports_unreachable_servers_as_transport_errors() {
	// Port 1 refuses connections immediately — no daemon runs there.
	wikid_untargeted()
		.args([
			"--server",
			"http://127.0.0.1:1",
			"--token",
			"t",
			"--wiki",
			"main",
			"status",
		])
		.assert()
		.code(1)
		.stdout(predicate::str::starts_with("error[transport]:"))
		.stdout(predicate::str::contains("hint:"));
}

#[test]
fn serve_without_a_discoverable_config_is_a_structured_error() {
	let empty_home = TempDir::new().unwrap();
	let cwd = TempDir::new().unwrap();
	let mut cmd = Command::cargo_bin("wikid").unwrap();
	clear_env(&mut cmd);
	cmd.env("HOME", empty_home.path())
		.current_dir(cwd.path())
		.arg("serve")
		.assert()
		.code(1)
		.stdout(predicate::str::starts_with("error[no_config]:"))
		.stdout(predicate::str::contains("wikid.toml"));
}

#[test]
fn serve_with_a_missing_config_file_fails_loudly() {
	let mut cmd = wikid_untargeted();
	cmd.args(["serve", "--config", "/nonexistent/wikid.toml"])
		.assert()
		.code(1)
		.stdout(predicate::str::starts_with("error[config]:"))
		.stdout(predicate::str::contains("/nonexistent/wikid.toml"));
}

// --- read commands: flags and ignore rules ---

#[test]
fn ls_hides_dotfiles_and_tree_recurses() {
	let vault = fixture_vault();
	let ls = stdout_of(wikid(vault.path()).args(["ls", "--json"]));
	assert!(!ls.contains(".obsidian"), "hidden dirs must be invisible: {ls}");
	let shallow = stdout_of(wikid(vault.path()).arg("ls"));
	assert!(!shallow.contains("notes/alpha.md"), "ls is depth 1: {shallow}");
	let tree = stdout_of(wikid(vault.path()).arg("tree"));
	assert!(tree.contains("notes/alpha.md"), "tree recurses: {tree}");
	let depth1 = stdout_of(wikid(vault.path()).args(["tree", "--depth", "1"]));
	assert!(!depth1.contains("notes/alpha.md"), "--depth limits tree: {depth1}");
}

#[test]
fn ls_of_missing_path_is_not_found() {
	let vault = fixture_vault();
	wikid(vault.path())
		.args(["ls", "ghost"])
		.assert()
		.code(1)
		.stdout(predicate::str::starts_with("error[not_found]:"));
}

#[test]
fn cat_of_binary_attachment_is_not_utf8() {
	let vault = fixture_vault();
	wikid(vault.path())
		.args(["cat", "assets/blob.bin"])
		.assert()
		.code(1)
		.stdout(predicate::str::starts_with("error[not_utf8]:"));
}

#[test]
fn grep_supports_ignore_case_files_only_context_and_limit() {
	let vault = fixture_vault();
	// -i: pattern in the wrong case still matches.
	wikid(vault.path()).args(["grep", "NEEDLE"]).assert().code(1);
	wikid(vault.path())
		.args(["grep", "NEEDLE", "-i"])
		.assert()
		.success()
		.stdout(predicate::str::contains("total: 2 matches"));
	// -l: one line per file, no line text.
	let files = stdout_of(wikid(vault.path()).args(["grep", "needle", "-l"]));
	assert_eq!(
		files.lines().filter(|l| l.contains("alpha.md")).count(),
		1,
		"-l collapses to one line per file: {files}"
	);
	// -C: context lines rendered with `-` separators around the match.
	let context = stdout_of(wikid(vault.path()).args(["grep", "is here", "-C", "1"]));
	assert!(context.contains("notes/alpha.md:2- "), "context before: {context}");
	assert!(context.contains("notes/alpha.md:3: The needle is here."), "{context}");
	assert!(context.contains("notes/alpha.md:4- "), "context after: {context}");
	// --limit: truncation is explicit with the size hint.
	wikid(vault.path())
		.args(["grep", "needle", "--limit", "1"])
		.assert()
		.success()
		.stdout(predicate::str::contains(
			"total: 2 matches in 1 file (4 searched) (showing first 1) — use --limit <n>",
		));
}

#[test]
fn grep_bad_regex_is_a_structured_error() {
	let vault = fixture_vault();
	wikid(vault.path())
		.args(["grep", "(unclosed"])
		.assert()
		.code(1)
		.stdout(predicate::str::starts_with("error[bad_pattern]:"));
}

#[test]
fn links_reports_outgoing_and_backlinks() {
	let vault = fixture_vault();
	let out = stdout_of(wikid(vault.path()).args(["links", "index.md"]));
	assert!(out.contains("[[alpha]] → notes/alpha.md"), "resolved link: {out}");
	assert!(out.contains("[[missing]] → (unresolved)"), "broken link: {out}");
	let back = stdout_of(wikid(vault.path()).args(["links", "notes/alpha.md"]));
	assert!(back.contains("index.md"), "backlink from index: {back}");
}

#[test]
fn doctor_filters_checks_and_rejects_unknown_names() {
	let vault = fixture_vault();
	let filtered = json_of(wikid(vault.path()).args(["doctor", "--checks", "broken_links", "--json"]));
	assert_eq!(filtered["counts"]["broken_links"], 1, "index.md links to [[missing]]");
	assert!(
		filtered["counts"].get("orphan_pages").is_none(),
		"disabled checks must not report: {filtered}"
	);
	wikid(vault.path())
		.args(["doctor", "--checks", "bogus_check"])
		.assert()
		.code(1)
		.stdout(predicate::str::starts_with("error[bad_pattern]:"))
		.stdout(predicate::str::contains("broken_links"));
	// --stale-days 0 makes every page stale.
	let stale = json_of(wikid(vault.path()).args(["doctor", "--stale-days", "0", "--checks", "stale_pages", "--json"]));
	assert_eq!(stale["counts"]["stale_pages"], 4, "{stale}");
}

// --- mutating commands: happy + error paths through the real binary ---

#[test]
fn write_from_stdin_creates_then_overwrites() {
	let vault = fixture_vault();
	wikid(vault.path())
		.args(["write", "drafts/todo.md"])
		.write_stdin("# Todo\n\n- ship it\n")
		.assert()
		.success()
		.stdout(predicate::str::contains("wrote drafts/todo.md (created,"));
	assert_eq!(
		fs::read_to_string(vault.path().join("drafts/todo.md")).unwrap(),
		"# Todo\n\n- ship it\n"
	);
	wikid(vault.path())
		.args(["write", "drafts/todo.md"])
		.write_stdin("replaced\n")
		.assert()
		.success()
		.stdout(predicate::str::contains("wrote drafts/todo.md (updated,"));
	assert_eq!(
		fs::read_to_string(vault.path().join("drafts/todo.md")).unwrap(),
		"replaced\n"
	);
}

#[test]
fn write_message_flag_adds_a_trailing_newline() {
	let vault = fixture_vault();
	wikid(vault.path())
		.args(["write", "one-liner.md", "-m", "just this"])
		.assert()
		.success();
	assert_eq!(
		fs::read_to_string(vault.path().join("one-liner.md")).unwrap(),
		"just this\n"
	);
}

#[test]
fn write_rejects_escaping_and_hidden_paths() {
	let vault = fixture_vault();
	for bad in ["../escape.md", ".obsidian/sneaky.md"] {
		wikid(vault.path())
			.args(["write", bad, "-m", "nope"])
			.assert()
			.code(1)
			.stdout(predicate::str::starts_with("error[invalid_path]:"));
	}
	assert!(!vault.path().parent().unwrap().join("escape.md").exists());
}

#[test]
fn edit_replaces_unique_match_and_reports_ambiguity_and_misses() {
	let vault = fixture_vault();
	wikid(vault.path())
		.args(["edit", "notes/alpha.md", "--old", "is here", "--new", "was here"])
		.assert()
		.success()
		.stdout(predicate::str::contains("edited notes/alpha.md: 1 replacement"));
	let content = fs::read_to_string(vault.path().join("notes/alpha.md")).unwrap();
	assert!(content.contains("The needle was here."));
	// Two occurrences of "needle" without --all → ambiguous, file untouched.
	wikid(vault.path())
		.args(["edit", "notes/alpha.md", "--old", "needle", "--new", "pin"])
		.assert()
		.code(1)
		.stdout(predicate::str::starts_with("error[ambiguous]:"))
		.stdout(predicate::str::contains("2 matches"));
	assert!(
		fs::read_to_string(vault.path().join("notes/alpha.md"))
			.unwrap()
			.contains("needle")
	);
	// Zero occurrences → no_match with the nearest-line hint.
	wikid(vault.path())
		.args(["edit", "notes/alpha.md", "--old", "The needle is gone", "--new", "x"])
		.assert()
		.code(1)
		.stdout(predicate::str::starts_with("error[no_match]:"))
		.stdout(predicate::str::contains("hint:"));
}

#[test]
fn edit_all_replaces_every_occurrence() {
	let vault = fixture_vault();
	wikid(vault.path())
		.args(["edit", "notes/alpha.md", "--old", "needle", "--new", "pin", "--all"])
		.assert()
		.success()
		.stdout(predicate::str::contains("2 replacements"));
	let content = fs::read_to_string(vault.path().join("notes/alpha.md")).unwrap();
	assert!(!content.contains("needle"));
	assert_eq!(content.matches("pin").count(), 2);
}

#[test]
fn mv_moves_and_refuses_to_clobber_without_force() {
	let vault = fixture_vault();
	wikid(vault.path())
		.args(["mv", "notes/beta.md", "archive/beta.md"])
		.assert()
		.success()
		.stdout(predicate::str::contains("moved notes/beta.md → archive/beta.md"));
	assert!(vault.path().join("archive/beta.md").exists());
	assert!(!vault.path().join("notes/beta.md").exists());
	// Destination exists → refused without --force.
	wikid(vault.path())
		.args(["mv", "archive/beta.md", "index.md"])
		.assert()
		.code(1)
		.stdout(predicate::str::starts_with("error[already_exists]:"));
	assert!(vault.path().join("archive/beta.md").exists());
	// --force overwrites.
	wikid(vault.path())
		.args(["mv", "archive/beta.md", "index.md", "--force"])
		.assert()
		.success();
	assert!(
		fs::read_to_string(vault.path().join("index.md"))
			.unwrap()
			.contains("# Beta")
	);
	// Missing source → not_found.
	wikid(vault.path())
		.args(["mv", "ghost.md", "somewhere.md"])
		.assert()
		.code(1)
		.stdout(predicate::str::starts_with("error[not_found]:"));
}

#[test]
fn rm_of_missing_file_is_not_found() {
	let vault = fixture_vault();
	wikid(vault.path())
		.args(["rm", "ghost.md", "--force"])
		.assert()
		.code(1)
		.stdout(predicate::str::starts_with("error[not_found]:"));
}

/// `--dir` at a nonexistent path is a dedicated structured error that points
/// back at the targeting flags, not the generic "run ls" not-found hint.
#[test]
fn dir_at_missing_path_is_a_targeting_error() {
	wikid(Path::new("/nonexistent/wikid-test-vault"))
		.arg("status")
		.assert()
		.code(1)
		.stdout(predicate::str::starts_with(
			"error[not_found]: wiki directory not found: /nonexistent/wikid-test-vault",
		))
		.stdout(predicate::str::contains("--dir"))
		.stderr(predicate::str::is_empty());
}

/// `--dir` accepts relative paths too (resolved from the process cwd).
#[test]
fn dir_flag_accepts_relative_paths() {
	let vault = fixture_vault();
	let name = vault.path().file_name().unwrap().to_str().unwrap();
	let parent: PathBuf = vault.path().parent().unwrap().to_path_buf();
	let mut cmd = Command::cargo_bin("wikid").unwrap();
	clear_env(&mut cmd);
	cmd.current_dir(&parent)
		.args(["--dir", name, "status"])
		.assert()
		.success()
		.stdout(predicate::str::contains("pages: 4"));
}
