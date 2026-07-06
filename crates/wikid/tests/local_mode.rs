//! Local-mode CLI integration tests (DESIGN §8): the AXI conformance
//! checklist item by item, `--json` validity for every command, exit codes,
//! and happy + error paths for the mutating commands, all through the real
//! binary against temp vaults.

use std::fs;
use std::io::{BufRead, BufReader, Read};
use std::net::TcpListener;
use std::path::{Path, PathBuf};
use std::process::{Command as StdCommand, Stdio};

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
	fs::write(
		root.join("index.md"),
		"---\ntags: [Home]\n---\n\n# Home\n\nSee [[alpha]] and [[missing]]. #project\n",
	)
	.unwrap();
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

fn free_port() -> u16 {
	TcpListener::bind("127.0.0.1:0").unwrap().local_addr().unwrap().port()
}

fn write_config(path: &Path, bind: &str, wikis: &[(&str, &Path)], default_wiki: Option<&str>, token: Option<&str>) {
	let mut text = format!("bind = {bind:?}\n");
	if let Some(default_wiki) = default_wiki {
		text.push_str(&format!("default_wiki = {default_wiki:?}\n"));
	}
	text.push_str("\n[wikis]\n");
	for (name, path) in wikis {
		text.push_str(&format!("{name} = {:?}\n", path.display().to_string()));
	}
	text.push_str("\n[tokens]\n");
	if let Some(token) = token {
		text.push_str(&format!("{token:?} = \"admin\"\n"));
	}
	fs::write(path, text).unwrap();
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
		.stdout(predicate::str::contains("wiki:"))
		.stdout(predicate::str::contains("root:"))
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
fn cat_lines_reads_windows_and_hashes_use_absolute_line_numbers() {
	let vault = fixture_vault();
	let out = stdout_of(wikid(vault.path()).args(["cat", "big.md", "--lines", "498-999"]));
	assert!(
		out.contains("line 498\nline 499\nline 500"),
		"window missing tail: {out}"
	);
	assert!(out.contains("lines 498-500 of 500"), "window metadata missing: {out}");
	assert!(
		!out.contains("truncated"),
		"window should not be marked truncated: {out}"
	);

	let json = json_of(wikid(vault.path()).args(["cat", "big.md", "--lines", "498-500", "--hashes", "--json"]));
	assert_eq!(json["range_start"], 498);
	assert_eq!(json["range_end"], 500);
	assert_eq!(json["lines"][0]["line"], 498);
	assert_eq!(json["lines"][0]["text"], "line 498");

	wikid(vault.path())
		.args(["cat", "big.md", "--full", "--lines", "1-2"])
		.assert()
		.failure();
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
		&["tags"],
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
	let hashes = json_of(wikid(vault.path()).args(["cat", "new.md", "--hashes", "--json"]));
	assert_eq!(hashes["lines"][0]["line"], 1);
	assert_eq!(hashes["lines"][0]["text"], "fresh");
	let hash = hashes["lines"][0]["hash"].as_str().unwrap().to_owned();
	let edit = json_of(wikid(vault.path()).args([
		"edit", "new.md", "--line", "1", "--hash", &hash, "--new", "stale", "--json",
	]));
	assert_eq!(edit["replacements"], 1);
	let hashes = json_of(wikid(vault.path()).args(["cat", "new.md", "--hashes", "--json"]));
	let hash = hashes["lines"][0]["hash"].as_str().unwrap().to_owned();
	let mut batch = wikid(vault.path());
	let batch_output = batch
		.args(["edit-batch", "new.md", "--json"])
		.write_stdin(format!(
			"[{{\"line\":1,\"expected_hash\":{hash:?},\"new_text\":\"batched\"}}]"
		))
		.output()
		.unwrap();
	assert_eq!(batch_output.status.code(), Some(0));
	let batch: serde_json::Value = serde_json::from_slice(&batch_output.stdout).unwrap();
	assert_eq!(batch["replacements"], 1);
	let mv = json_of(wikid(vault.path()).args(["mv", "new.md", "old.md", "--json"]));
	assert_eq!(mv["from"], "new.md");
	assert_eq!(mv["to"], "old.md");
	let rm = json_of(wikid(vault.path()).args(["rm", "old.md", "--force", "--json"]));
	assert_eq!(rm["path"], "old.md");
	let links = json_of(wikid(vault.path()).args(["links", "index.md", "--json"]));
	assert!(links["outgoing"].is_array());
	assert!(links["backlinks"].is_array());
	let tags = json_of(wikid(vault.path()).args(["tags", "--json"]));
	assert_eq!(tags["tags"][0]["tag"], "Home");
	assert_eq!(tags["tags"][1]["tag"], "project");
	assert!(tags["tags"][1].get("implied").is_none());
	let doctor = json_of(wikid(vault.path()).args(["doctor", "--json"]));
	assert!(doctor["issues"].is_array());
	assert!(doctor["summary"].is_string());
}

#[test]
fn tags_json_and_human_output_mark_implied_ancestors() {
	let vault = fixture_vault();
	fs::write(
		vault.path().join("tagged.md"),
		"#project/wikid #area/research #project\n",
	)
	.unwrap();

	let json = json_of(wikid(vault.path()).args(["tags", "--json"]));
	let tags = json["tags"].as_array().unwrap();
	let area = tags.iter().find(|tag| tag["tag"] == "area").unwrap();
	assert_eq!(area["implied"], true);
	let project = tags.iter().find(|tag| tag["tag"] == "project").unwrap();
	assert!(project.get("implied").is_none());

	let human = stdout_of(wikid(vault.path()).args(["tags"]));
	assert!(human.contains("#area (implied)"), "{human}");
	assert!(human.contains("#project  "), "{human}");
}

#[test]
fn doctor_shows_sanitized_yaml_details_for_malformed_frontmatter() {
	let vault = TempDir::new().unwrap();
	fs::write(vault.path().join("bad.md"), "---\ntitle: [unterminated\n---\n# Bad\n").unwrap();
	let out = stdout_of(wikid(vault.path()).args(["doctor", "--checks", "malformed_frontmatter"]));
	assert!(out.contains("invalid YAML frontmatter (line"), "{out}");
	assert!(!out.contains("column"), "{out}");
	let json = json_of(wikid(vault.path()).args(["doctor", "--checks", "malformed_frontmatter", "--json"]));
	let detail = json["issues"][0]["detail"].as_str().unwrap();
	assert!(detail.starts_with("invalid YAML frontmatter (line"), "{detail}");
	assert!(!detail.contains("column"), "{detail}");
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

#[test]
fn cat_fragments_extract_sections_blocks_and_json_errors() {
	let vault = fixture_vault();
	fs::write(
		vault.path().join("Alpha.md"),
		"# Alpha\n\n## Section One\nbody\n### Nested\ndetail\n## Section Two\nother ^block-id\n",
	)
	.unwrap();

	let section = stdout_of(wikid(vault.path()).args(["cat", "Alpha#Section One"]));
	assert!(
		section.contains("## Section One\nbody\n### Nested\ndetail"),
		"{section}"
	);
	assert!(!section.contains("## Section Two"), "{section}");

	let hashes = json_of(wikid(vault.path()).args(["cat", "Alpha.md#Alpha#Nested", "--hashes", "--json"]));
	assert_eq!(hashes["lines"][0]["line"], 5);
	assert_eq!(hashes["lines"][0]["text"], "### Nested");

	let block = stdout_of(wikid(vault.path()).args(["cat", "Alpha.md#^block-id"]));
	assert!(block.contains("other ^block-id"), "{block}");
	assert!(!block.contains("## Section Two"), "{block}");

	let missing = wikid(vault.path())
		.args(["cat", "Alpha.md#Missing", "--json"])
		.output()
		.unwrap();
	assert_eq!(missing.status.code(), Some(1));
	let value: serde_json::Value = serde_json::from_slice(&missing.stdout).expect("valid JSON");
	assert_eq!(value["error"]["code"], "fragment_not_found");
	assert!(value["error"]["message"].as_str().unwrap().contains("Alpha.md"));
	assert!(value["error"]["message"].as_str().unwrap().contains("Missing"));
}

#[test]
fn read_path_not_found_hints_when_md_extension_exists() {
	let vault = fixture_vault();
	fs::write(vault.path().join("Home.md"), "# Home\n").unwrap();
	wikid(vault.path())
		.args(["cat", "Home"])
		.assert()
		.code(1)
		.stdout(predicate::str::contains("hint: did you mean Home.md?"));
	let json = json_of(wikid(vault.path()).args(["links", "Home", "--json"]));
	assert_eq!(json["error"]["hint"], "did you mean Home.md?");
}

// --- targeting: local dir, remote selection, serve config discovery ---

#[test]
fn no_target_is_a_friendly_zero_state() {
	let home = TempDir::new().unwrap();
	wikid_untargeted()
		.env("HOME", home.path())
		.arg("status")
		.assert()
		.code(1)
		.stdout(predicate::str::starts_with("error[no_target]: wikid: no wiki selected"))
		.stdout(predicate::str::contains(
			"No --dir, WIKID_DIR, remote server, or config wiki was found.",
		))
		.stdout(predicate::str::contains("next:"))
		.stdout(predicate::str::contains("wikid --dir <path> status"))
		.stdout(predicate::str::contains("wikid init <path>"))
		.stdout(predicate::str::contains(
			"hint: a wiki is just a directory of Markdown files",
		));
}

#[test]
fn dir_and_server_together_is_a_usage_error() {
	let vault = fixture_vault();
	// Flag + flag: caught by clap.
	wikid(vault.path())
		.args(["--server", "http://localhost:7448", "status"])
		.assert()
		.code(2);
	// Explicit remote targeting wins over a local env var; it then fails for the missing remote wiki.
	let mut cmd = wikid_untargeted();
	cmd.env("WIKID_DIR", vault.path())
		.args(["--server", "http://localhost:7448", "status"])
		.assert()
		.code(1)
		.stdout(predicate::str::starts_with("error[no_wiki]:"));
	let mut env_only = wikid_untargeted();
	env_only
		.env("WIKID_DIR", vault.path())
		.env("WIKID_SERVER", "http://localhost:7448")
		.arg("status")
		.assert()
		.code(2);
}

#[test]
fn remote_mode_without_a_server_does_not_fall_back_to_config() {
	let config_vault = fixture_vault();
	let cwd = TempDir::new().unwrap();
	let config_path = cwd.path().join("wikid.toml");
	write_config(
		&config_path,
		"127.0.0.1:7448",
		&[("configured", config_vault.path())],
		Some("configured"),
		Some("t"),
	);

	wikid_untargeted()
		.current_dir(cwd.path())
		.args(["--wiki", "configured", "status"])
		.assert()
		.code(1)
		.stdout(predicate::str::starts_with("error[no_target]:"));
	wikid_untargeted()
		.current_dir(cwd.path())
		.args(["--token", "t", "status"])
		.assert()
		.code(1)
		.stdout(predicate::str::starts_with("error[no_target]:"));
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
fn config_fallback_precedence_flag_env_config_and_cwd_inside_wiki() {
	let flag_vault = fixture_vault();
	let env_vault = fixture_vault();
	let config_vault = fixture_vault();
	let home = TempDir::new().unwrap();
	let cwd = TempDir::new().unwrap();
	let config_path = cwd.path().join("wikid.toml");
	write_config(
		&config_path,
		"127.0.0.1:7448",
		&[("configured", config_vault.path())],
		None,
		None,
	);

	wikid_untargeted()
		.env("HOME", home.path())
		.current_dir(cwd.path())
		.arg("status")
		.assert()
		.success()
		.stdout(predicate::str::contains(config_vault.path().display().to_string()));

	wikid_untargeted()
		.env("HOME", home.path())
		.env("WIKID_DIR", env_vault.path())
		.current_dir(cwd.path())
		.arg("status")
		.assert()
		.success()
		.stdout(predicate::str::contains(env_vault.path().display().to_string()));

	wikid_untargeted()
		.env("HOME", home.path())
		.env("WIKID_DIR", env_vault.path())
		.current_dir(cwd.path())
		.args(["--dir"])
		.arg(flag_vault.path())
		.arg("status")
		.assert()
		.success()
		.stdout(predicate::str::contains(flag_vault.path().display().to_string()));

	let nested = config_vault.path().join("notes/deep");
	fs::create_dir_all(&nested).unwrap();
	wikid_untargeted()
		.env("HOME", home.path())
		.current_dir(&nested)
		.args(["--config"])
		.arg(&config_path)
		.args(["cat", "index.md"])
		.assert()
		.success()
		.stdout(predicate::str::contains("# Home"));
}

#[test]
fn config_fallback_default_and_multi_wiki_ambiguity() {
	let one = fixture_vault();
	let two = fixture_vault();
	let cwd = TempDir::new().unwrap();
	let config_path = cwd.path().join("wikid.toml");
	write_config(
		&config_path,
		"127.0.0.1:7448",
		&[("one", one.path()), ("two", two.path())],
		Some("two"),
		None,
	);
	wikid_untargeted()
		.current_dir(cwd.path())
		.arg("status")
		.assert()
		.success()
		.stdout(predicate::str::contains(two.path().display().to_string()));

	write_config(
		&config_path,
		"127.0.0.1:7448",
		&[("one", one.path()), ("two", two.path())],
		None,
		None,
	);
	wikid_untargeted()
		.current_dir(cwd.path())
		.arg("status")
		.assert()
		.code(1)
		.stdout(predicate::str::starts_with("error[ambiguous_wiki]:"))
		.stdout(predicate::str::contains("one"))
		.stdout(predicate::str::contains("two"))
		.stdout(predicate::str::contains("default_wiki"));
}

#[test]
fn explicit_local_flag_ignores_remote_env_and_explicit_remote_ignores_dir_env() {
	let vault = fixture_vault();
	wikid_untargeted()
		.env("WIKID_SERVER", "http://127.0.0.1:1")
		.args(["--dir"])
		.arg(vault.path())
		.arg("status")
		.assert()
		.success();
	wikid_untargeted()
		.env("WIKID_DIR", vault.path())
		.args(["--server", "http://127.0.0.1:1", "--wiki", "main", "status"])
		.assert()
		.code(1)
		.stdout(predicate::str::starts_with("error[transport]:"));
}

#[test]
fn init_creates_skeleton_registers_config_and_is_idempotent() {
	let home = TempDir::new().unwrap();
	let wiki = TempDir::new().unwrap();
	wikid_untargeted()
		.env("HOME", home.path())
		.current_dir(wiki.path())
		.arg("init")
		.assert()
		.success()
		.stdout(predicate::str::contains("created:"))
		.stdout(predicate::str::contains("registered:"))
		.stdout(predicate::str::contains("not printed"));
	for path in [
		"index.md",
		"log.md",
		"AGENTS.md",
		"raw",
		"raw/assets",
		"concepts",
		"entities",
		"questions",
		"syntheses",
	] {
		assert!(wiki.path().join(path).exists(), "missing {path}");
	}
	let config_path = home.path().join(".config/wikid/config.toml");
	let config = wikid_server::Config::load(&config_path).unwrap();
	let name = wiki.path().file_name().unwrap().to_str().unwrap();
	assert_eq!(config.default_wiki.as_deref(), Some(name));
	assert_eq!(config.wikis[name], wiki.path().canonicalize().unwrap());
	assert_eq!(
		config.tokens.values().filter(|actor| actor.as_str() == "admin").count(),
		1
	);
	wikid_untargeted()
		.env("HOME", home.path())
		.current_dir(wiki.path())
		.arg("status")
		.assert()
		.success();
	wikid_untargeted()
		.env("HOME", home.path())
		.current_dir(wiki.path())
		.arg("init")
		.assert()
		.success()
		.stdout(predicate::str::contains("skipped:"))
		.stdout(predicate::str::contains("already registered"));
	let config2 = wikid_server::Config::load(&config_path).unwrap();
	assert_eq!(
		config2
			.tokens
			.values()
			.filter(|actor| actor.as_str() == "admin")
			.count(),
		1
	);
}

#[test]
fn init_partial_and_existing_vault_only_adds_missing_files() {
	let home = TempDir::new().unwrap();
	let wiki = TempDir::new().unwrap();
	fs::create_dir_all(wiki.path().join("raw/assets")).unwrap();
	fs::create_dir_all(wiki.path().join(".obsidian")).unwrap();
	fs::write(wiki.path().join("index.md"), "# Existing\n").unwrap();
	wikid_untargeted()
		.env("HOME", home.path())
		.args(["init"])
		.arg(wiki.path())
		.assert()
		.success()
		.stdout(predicate::str::contains("skipped:"))
		.stdout(predicate::str::contains("index.md"));
	assert_eq!(
		fs::read_to_string(wiki.path().join("index.md")).unwrap(),
		"# Existing\n"
	);
	assert!(wiki.path().join("log.md").exists());
	assert!(wiki.path().join("AGENTS.md").exists());
}

#[test]
fn init_collision_suffix_and_token_show() {
	let home = TempDir::new().unwrap();
	let parent = TempDir::new().unwrap();
	let first = parent.path().join("notes");
	let second_parent = TempDir::new().unwrap();
	let second = second_parent.path().join("notes");
	fs::create_dir_all(&first).unwrap();
	fs::create_dir_all(&second).unwrap();
	for path in [&first, &second] {
		wikid_untargeted()
			.env("HOME", home.path())
			.args(["init"])
			.arg(path)
			.assert()
			.success();
	}
	let config_path = home.path().join(".config/wikid/config.toml");
	let config = wikid_server::Config::load(&config_path).unwrap();
	assert_eq!(config.wikis["notes"], first.canonicalize().unwrap());
	assert_eq!(config.wikis["notes-2"], second.canonicalize().unwrap());
	let token = config.tokens.keys().next().unwrap().clone();
	wikid_untargeted()
		.env("HOME", home.path())
		.args(["token", "show"])
		.assert()
		.success()
		.stdout(predicate::str::starts_with(token.clone()));
	let json = json_of(
		wikid_untargeted()
			.env("HOME", home.path())
			.args(["token", "show", "admin", "--json"]),
	);
	assert_eq!(json["actor"], "admin");
	assert_eq!(json["token"], token);
	assert_eq!(json["config_path"], config_path.display().to_string());
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
fn serve_without_config_bootstraps_config_and_prints_no_token_value() {
	let home = TempDir::new().unwrap();
	let cwd = TempDir::new().unwrap();
	let bin = assert_cmd::cargo::cargo_bin("wikid");
	let mut child = StdCommand::new(bin)
		.env_remove("WIKID_CONFIG")
		.env_remove("WIKID_DIR")
		.env_remove("WIKID_SERVER")
		.env("HOME", home.path())
		.current_dir(cwd.path())
		.arg("serve")
		.stdout(Stdio::piped())
		.stderr(Stdio::piped())
		.spawn()
		.unwrap();
	let stdout = child.stdout.take().unwrap();
	let mut reader = BufReader::new(stdout);
	let mut startup = String::new();
	for _ in 0..6 {
		let mut line = String::new();
		if reader.read_line(&mut line).unwrap() == 0 {
			break;
		}
		startup.push_str(&line);
		if startup.contains("wikid token show admin") {
			break;
		}
	}
	let _ = child.kill();
	let _ = child.wait();
	let config_path = home.path().join(".config/wikid/config.toml");
	let config_text = fs::read_to_string(&config_path).unwrap();
	let config = wikid_server::Config::from_toml(&config_text).unwrap();
	let token = config.tokens.keys().next().unwrap();
	assert_eq!(config.bind, "127.0.0.1:7448");
	assert_eq!(
		config.wikis[cwd.path().file_name().unwrap().to_str().unwrap()],
		cwd.path().canonicalize().unwrap()
	);
	assert!(token.starts_with("wkd_"));
	assert_eq!(token.len(), 68);
	assert!(!startup.contains(token), "serve startup leaked token: {startup}");
	assert!(startup.contains("created config:"), "startup: {startup}");
	assert!(startup.contains("admin token written"), "startup: {startup}");
	#[cfg(unix)]
	{
		use std::os::unix::fs::PermissionsExt as _;
		assert_eq!(fs::metadata(&config_path).unwrap().permissions().mode() & 0o777, 0o600);
	}
}

#[test]
fn serve_json_startup_is_one_json_object() {
	let home = TempDir::new().unwrap();
	let cwd = TempDir::new().unwrap();
	let config_path = home.path().join("wikid.toml");
	let port = free_port();
	write_config(
		&config_path,
		&format!("127.0.0.1:{port}"),
		&[("configured", cwd.path())],
		Some("configured"),
		Some("t"),
	);
	let bin = assert_cmd::cargo::cargo_bin("wikid");
	let mut child = StdCommand::new(bin)
		.env_remove("WIKID_CONFIG")
		.env("HOME", home.path())
		.current_dir(cwd.path())
		.args(["--json", "--config"])
		.arg(&config_path)
		.arg("serve")
		.stdout(Stdio::piped())
		.stderr(Stdio::piped())
		.spawn()
		.unwrap();
	let stdout = child.stdout.take().unwrap();
	let mut reader = BufReader::new(stdout);
	let mut line = String::new();
	reader.read_line(&mut line).unwrap();
	let value: serde_json::Value = serde_json::from_str(&line).unwrap_or_else(|e| panic!("bad json {e}: {line}"));
	assert_eq!(value["bootstrapped"], false);
	assert_eq!(value["config_path"], config_path.display().to_string());
	assert!(value["admin_token"].as_str().unwrap().contains("not printed"));
	let health_url = format!("http://127.0.0.1:{port}/health");
	let deadline = std::time::Instant::now() + std::time::Duration::from_secs(5);
	while ureq::get(&health_url).call().is_err() {
		assert!(
			std::time::Instant::now() < deadline,
			"wikid serve did not answer {health_url}"
		);
		std::thread::sleep(std::time::Duration::from_millis(25));
	}
	let _ = child.kill();
	let _ = child.wait();
	let mut remaining_stdout = String::new();
	reader.read_to_string(&mut remaining_stdout).unwrap();
	assert!(
		remaining_stdout.trim().is_empty(),
		"serve logs leaked to stdout: {remaining_stdout}"
	);
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
fn concise_help_documents_obsidian_fragments_embeds_and_tags() {
	let cat_help = stdout_of(wikid_untargeted().args(["cat", "--help"]));
	assert!(cat_help.contains("#Heading"), "{cat_help}");
	assert!(cat_help.contains("#^block-id"), "{cat_help}");
	let links_help = stdout_of(wikid_untargeted().args(["links", "--help"]));
	assert!(links_help.contains("![[...]] has embed=true"), "{links_help}");
	let tags_help = stdout_of(wikid_untargeted().args(["tags", "--help"]));
	assert!(tags_help.contains("inline and frontmatter tags"), "{tags_help}");
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
fn links_marks_embeds_in_human_output() {
	let vault = fixture_vault();
	fs::write(vault.path().join("index.md"), "# Home\n\n![[alpha]]\n").unwrap();
	let out = stdout_of(wikid(vault.path()).args(["links", "index.md"]));
	assert!(out.contains("embed ![[alpha]] → notes/alpha.md"), "embed marker: {out}");
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
fn cat_hashes_lists_line_number_hash_and_text_with_the_edit_hint() {
	let vault = fixture_vault();
	let out = stdout_of(wikid(vault.path()).args(["cat", "notes/alpha.md", "--hashes"]));
	let hash = wikid_core::hash_line("The needle is here.");
	assert!(out.contains(&format!("3:{hash}: The needle is here.")), "{out}");
	assert!(
		out.contains("hint: wikid edit notes/alpha.md --line <n> --hash <hash> --new=<text>"),
		"{out}"
	);
	assert!(out.contains("hint: wikid edit-batch notes/alpha.md"), "{out}");
}

#[test]
fn edit_replaces_a_hash_addressed_line_and_rejects_stale_or_bad_targets() {
	let vault = fixture_vault();
	let hash = wikid_core::hash_line("The needle is here.");
	wikid(vault.path())
		.args([
			"edit",
			"notes/alpha.md",
			"--line",
			"3",
			"--hash",
			&hash,
			"--new",
			"The needle was here.",
		])
		.assert()
		.success()
		.stdout(predicate::str::contains("edited notes/alpha.md: 1 line replaced"));
	let content = fs::read_to_string(vault.path().join("notes/alpha.md")).unwrap();
	assert!(content.contains("The needle was here."));
	// Reusing the now-stale hash → refused, file untouched.
	wikid(vault.path())
		.args(["edit", "notes/alpha.md", "--line", "3", "--hash", &hash, "--new", "x"])
		.assert()
		.code(1)
		.stdout(predicate::str::starts_with("error[stale_edit]:"))
		.stdout(predicate::str::contains("hint:"));
	assert_eq!(
		fs::read_to_string(vault.path().join("notes/alpha.md")).unwrap(),
		content,
		"a refused edit must not touch the file"
	);
	// Out-of-range line → bad_edit pointing back at cat --hashes.
	wikid(vault.path())
		.args(["edit", "notes/alpha.md", "--line", "99", "--hash", &hash, "--new", "x"])
		.assert()
		.code(1)
		.stdout(predicate::str::starts_with("error[bad_edit]:"))
		.stdout(predicate::str::contains("cat notes/alpha.md"));
}

#[test]
fn edit_accepts_leading_dash_values_with_equals_form() {
	let vault = fixture_vault();
	let hash = wikid_core::hash_line("The needle is here.");
	wikid(vault.path())
		.args([
			"edit",
			"notes/alpha.md",
			"--line",
			"3",
			"--hash",
			&hash,
			"--new=- bullet item",
		])
		.assert()
		.success();
	let content = fs::read_to_string(vault.path().join("notes/alpha.md")).unwrap();
	assert!(content.contains("- bullet item"));
}

#[test]
fn edit_batch_replaces_multiple_hash_guarded_lines_from_json_stdin() {
	let vault = fixture_vault();
	let first = wikid_core::hash_line("The needle is here.");
	let second = wikid_core::hash_line("Another needle line.");
	wikid(vault.path())
		.args(["edit-batch", "notes/alpha.md"])
		.write_stdin(format!(
			"[{{\"line\":3,\"expected_hash\":{first:?},\"new_text\":\"first replacement\"}},{{\"line\":4,\"expected_hash\":{second:?},\"new_text\":\"second replacement\"}}]"
		))
		.assert()
		.success()
		.stdout(predicate::str::contains("edited notes/alpha.md: 2 lines replaced"));
	let content = fs::read_to_string(vault.path().join("notes/alpha.md")).unwrap();
	assert!(content.contains("first replacement"));
	assert!(content.contains("second replacement"));
}

#[test]
fn edit_batch_rejects_invalid_json_as_structured_bad_edit() {
	let vault = fixture_vault();
	wikid(vault.path())
		.args(["edit-batch", "notes/alpha.md"])
		.write_stdin("not json")
		.assert()
		.code(1)
		.stdout(predicate::str::starts_with("error[bad_edit]:"))
		.stdout(predicate::str::contains("JSON array"));
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

#[test]
fn skills_are_client_side_and_json_shapes_are_stable() {
	let list = json_of(wikid_untargeted().arg("skills").arg("--json"));
	assert_eq!(list["skills"][0]["name"], "core");
	assert!(list["skills"][0]["description"].as_str().unwrap().contains("wikid"));

	let get = json_of(wikid_untargeted().arg("skills").arg("get").arg("core").arg("--json"));
	assert_eq!(get["name"], "core");
	assert_eq!(get["full"], false);
	assert!(get["content"].as_str().unwrap().starts_with("---\nname: wikid-core\n"));

	let body = include_str!("../../../skills/core/SKILL.md");
	let output = wikid_untargeted()
		.arg("skills")
		.arg("get")
		.arg("core")
		.output()
		.expect("run wikid");
	assert!(output.status.success());
	assert_eq!(output.stdout, body.as_bytes());

	let full = stdout_of(wikid_untargeted().arg("skills").arg("get").arg("core").arg("--full"));
	assert!(full.contains("# Reference: json-shapes"));
	assert!(full.contains("# Reference: doctor-checks"));
	assert!(full.contains("# Reference: link-resolution"));
}

#[test]
fn skills_path_materializes_versioned_tree_and_current_symlink() {
	let data = TempDir::new().unwrap();
	let mut cmd = wikid_untargeted();
	cmd.env("XDG_DATA_HOME", data.path())
		.arg("skills")
		.arg("path")
		.arg("core");
	let path = stdout_of(&mut cmd).trim().to_owned();
	let skill_dir = PathBuf::from(&path);
	assert!(skill_dir.join("SKILL.md").is_file());
	assert!(skill_dir.join("references/json-shapes.md").is_file());
	let version_dir = skill_dir.parent().unwrap();
	assert!(version_dir.join("llm-wiki/SKILL.md").is_file());
	assert!(version_dir.join(".complete").is_file());
	assert!(version_dir.parent().unwrap().join("current").exists());

	let path_again = stdout_of(
		wikid_untargeted()
			.env("XDG_DATA_HOME", data.path())
			.arg("skills")
			.arg("path")
			.arg("core"),
	);
	assert_eq!(path_again.trim(), path);
}

#[test]
fn unknown_skill_is_structured_not_found() {
	wikid_untargeted()
		.arg("skills")
		.arg("get")
		.arg("cor")
		.assert()
		.failure()
		.code(1)
		.stdout(predicate::str::contains("error[not_found]: skill not found: cor"))
		.stdout(predicate::str::contains("hint: did you mean core?"));
}
