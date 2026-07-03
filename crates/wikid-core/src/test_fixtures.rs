//! Builds the throwaway vault used by co-located tests (DESIGN §8): nested
//! pages, hidden dirs that must stay invisible, a binary attachment, a
//! UTF-8-valid file with a NUL byte (binary-sniff bait), and a gitignored
//! draft.

use std::fs;

use tempfile::TempDir;

use crate::Vault;

/// Contents of `projects/alpha.md` in the fixture, referenced by edit tests
/// that assert exact line numbers.
pub const ALPHA_MD: &str = "---\ntitle: Alpha\n---\n\n# Alpha\n\nalpha status: green\nsecond alpha line\n";

/// Contents of `notes/unicode.md` in the fixture.
pub const UNICODE_MD: &str = "# Ünïcode 🚀\n\nКиїв — місто.\n";

/// Creates the fixture vault. Keep the returned `TempDir` alive for the
/// duration of the test — dropping it deletes the vault.
pub fn vault() -> (TempDir, Vault) {
	let dir = tempfile::tempdir().expect("create temp vault");
	let write = |rel: &str, content: &[u8]| {
		let path = dir.path().join(rel);
		fs::create_dir_all(path.parent().unwrap()).unwrap();
		fs::write(path, content).unwrap();
	};
	write("index.md", b"# Home\n\nWelcome to [[alpha]] and [[projects/beta]].\n");
	write("projects/alpha.md", ALPHA_MD.as_bytes());
	write("projects/beta.md", b"# Beta\n\nplanning notes for beta\n");
	write("notes/unicode.md", UNICODE_MD.as_bytes());
	// Hidden dirs: must be invisible to all read operations.
	write(".obsidian/app.json", b"{}");
	write(".trash/old.md", b"# Old\n");
	// Not valid UTF-8 at all (and mentions alpha, which grep must not see).
	write("attachments/logo.png", b"\x89PNG\r\n\x00\x00alpha\x00binary");
	// Valid UTF-8 but contains a NUL byte: only the binary sniff skips it.
	write("attachments/data.bin", b"needle in valid utf-8\x00tail");
	// Gitignore applies even though the vault is not a git repository.
	write(".gitignore", b"drafts/\n");
	write("drafts/wip.md", b"# WIP\n");
	let vault = Vault::open(dir.path()).expect("open temp vault");
	(dir, vault)
}

/// Creates the knowledge-layer fixture vault (DESIGN §8): nested wikilinks
/// (alias, heading, ambiguous, broken), frontmatter'd and frontmatter-less
/// pages, malformed frontmatter, a hidden `.obsidian/` dir, a binary
/// attachment, an oversized page, and a stale page backdated 200 days.
///
/// Ten pages: index, projects/alpha, projects/beta, notes/guide,
/// notes/broken-fm, notes/todo, projects/todo (ambiguous stem), orphan,
/// big (>1500 lines), stale. Eight of the ten carry a frontmatter block, so
/// the vault is above the 50% `missing_frontmatter` adoption gate.
pub fn knowledge_vault() -> (TempDir, Vault) {
	let dir = tempfile::tempdir().expect("create temp vault");
	let write = |rel: &str, content: &[u8]| {
		let path = dir.path().join(rel);
		fs::create_dir_all(path.parent().unwrap()).unwrap();
		fs::write(path, content).unwrap();
	};
	write(
		"index.md",
		b"---\ntitle: Home\n---\n\n# Home\n\n- [[Alpha]]\n- [[projects/beta|The Beta]]\n- [[guide#Setup]] and [guide](notes/guide.md)\n- [[todo]]\n- [[missing-page]]\n- [[big]], [[stale]], [[broken-fm]]\n\n![[logo.png]]\n",
	);
	write(
		"projects/alpha.md",
		b"---\ntitle: Alpha\n---\n\n# Alpha\n\n## Status\n\ngreen\n",
	);
	write("projects/beta.md", b"# Beta\n\nno frontmatter here\n");
	write(
		"notes/guide.md",
		b"---\ntitle: Guide\n---\n\n# Guide\n\n## Setup\n\nSee [[Alpha#Status|the alpha project]].\n",
	);
	write("notes/broken-fm.md", b"---\ntitle: [unclosed\n---\n\n# Broken FM\n");
	write("notes/todo.md", b"---\ntitle: Notes todo\n---\n\n# Todo (notes)\n");
	write(
		"projects/todo.md",
		b"---\ntitle: Projects todo\n---\n\n# Todo (projects)\n",
	);
	write("orphan.md", b"# Orphan\n\nnobody links here\n");
	// Oversized by line count (>1500) while staying well under 64 KiB.
	let big = format!("---\ntitle: Big\n---\n{}", "filler line\n".repeat(1600));
	write("big.md", big.as_bytes());
	write("stale.md", b"---\ntitle: Stale\n---\n\n# Stale\n");
	write(".obsidian/app.json", b"{}");
	// Binary bait: wikilink-looking bytes that must stay invisible.
	write("attachments/logo.png", b"\x89PNG\r\n\x00\x00[[missing-in-binary]]\x00");
	backdate(&dir.path().join("stale.md"), 200);
	let vault = Vault::open(dir.path()).expect("open temp vault");
	(dir, vault)
}

/// Moves a file's mtime `days` into the past (DESIGN §8: `File::set_times`).
fn backdate(path: &std::path::Path, days: u64) {
	let mtime = std::time::SystemTime::now() - std::time::Duration::from_secs(days * 24 * 60 * 60);
	let file = fs::File::options().write(true).open(path).unwrap();
	file.set_times(fs::FileTimes::new().set_modified(mtime)).unwrap();
}
