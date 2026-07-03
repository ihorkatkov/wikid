//! The vault operation core (DESIGN §3): one implementation shared verbatim
//! by CLI, HTTP, and MCP. The result structs here ARE the wire format —
//! `--json` output, HTTP response bodies, and remote-client parsing all use
//! these exact shapes.

use std::collections::HashSet;
use std::fs;
use std::io::Write as _;
use std::path::{Path, PathBuf};
use std::time::SystemTime;

use serde::{Deserialize, Serialize};

use crate::error::WikidError;
use crate::vault::Vault;

/// Bytes sniffed for a NUL byte to classify a file as binary.
const BINARY_SNIFF_BYTES: usize = 8 * 1024;
/// Minimum bigram-overlap score for a nearest-line hint to be offered.
const NEAREST_LINE_MIN_SCORE: f64 = 0.3;

/// What a listing entry is: a directory, a page (`.md`), or any other file.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum EntryKind {
	/// A directory.
	Dir,
	/// A non-Markdown file (attachment).
	File,
	/// A Markdown page.
	Page,
}

/// One entry in a listing or glob result.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Entry {
	/// Vault-relative path; directories carry a trailing `/`.
	pub path: String,
	/// Directory, file, or page.
	pub kind: EntryKind,
	/// Size in bytes (0 for directories).
	pub size: u64,
	/// Last modification time, RFC3339 UTC.
	pub modified: String,
}

/// Result of `ls`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Listing {
	/// Entries within the requested depth, sorted by path.
	pub entries: Vec<Entry>,
	/// Directories in the whole subtree, regardless of depth.
	pub total_dirs: usize,
	/// Non-page files in the whole subtree, regardless of depth.
	pub total_files: usize,
	/// Pages in the whole subtree, regardless of depth.
	pub total_pages: usize,
}

/// Truncation limits for `cat`; whichever limit is hit first wins.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct ReadLimit {
	/// Maximum number of lines returned.
	pub max_lines: usize,
	/// Maximum number of content bytes returned (cut at a line boundary).
	pub max_bytes: usize,
}

impl Default for ReadLimit {
	fn default() -> Self {
		Self {
			max_lines: 400,
			max_bytes: 32 * 1024,
		}
	}
}

/// Result of `cat`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Document {
	/// Vault-relative path.
	pub path: String,
	/// File content, possibly truncated (see `truncated`).
	pub content: String,
	/// Whether `content` was cut by a `ReadLimit`.
	pub truncated: bool,
	/// Line count of the full file.
	pub total_lines: usize,
	/// Byte size of the full file.
	pub total_bytes: u64,
	/// Last modification time, RFC3339 UTC.
	pub modified: String,
}

/// Options for `grep`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct GrepOptions {
	/// Case-insensitive matching.
	pub ignore_case: bool,
	/// Emit one entry per matching file instead of one per matching line.
	pub files_only: bool,
	/// Lines of context to attach before and after each match (0 = none).
	pub context: usize,
	/// Maximum number of match entries returned.
	pub limit: usize,
}

impl Default for GrepOptions {
	fn default() -> Self {
		Self {
			ignore_case: false,
			files_only: false,
			context: 0,
			limit: 50,
		}
	}
}

/// One `grep` match.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct GrepMatch {
	/// Vault-relative path of the matching file.
	pub path: String,
	/// 1-based line number of the match.
	pub line: usize,
	/// The matching line.
	pub text: String,
	/// Lines before the match; present only when context was requested.
	#[serde(default, skip_serializing_if = "Option::is_none")]
	pub context_before: Option<Vec<String>>,
	/// Lines after the match; present only when context was requested.
	#[serde(default, skip_serializing_if = "Option::is_none")]
	pub context_after: Option<Vec<String>>,
}

/// Result of `grep`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct GrepResult {
	/// Match entries, stem-matching files ranked first, capped at `limit`.
	pub matches: Vec<GrepMatch>,
	/// All matches found, including those beyond `limit`.
	pub total_matches: usize,
	/// Files with at least one match.
	pub matched_files: usize,
	/// Text files searched (binary and non-UTF-8 files are skipped).
	pub total_files: usize,
	/// Whether `matches` was cut by `limit`.
	pub truncated: bool,
}

/// Result of `glob`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct GlobResult {
	/// Matching files, sorted by path.
	pub entries: Vec<Entry>,
	/// Number of matching files.
	pub total: usize,
}

/// Result of `write`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct WriteResult {
	/// Vault-relative path written.
	pub path: String,
	/// True if the file did not exist before.
	pub created: bool,
	/// Bytes written.
	pub bytes: u64,
}

/// Result of `edit`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct EditResult {
	/// Vault-relative path edited.
	pub path: String,
	/// Number of occurrences replaced.
	pub replacements: usize,
	/// Byte size of the file after the edit.
	pub bytes: u64,
}

/// Result of `mv`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct MvResult {
	/// Vault-relative source path.
	pub from: String,
	/// Vault-relative destination path.
	pub to: String,
}

/// Result of `rm`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct RmResult {
	/// Vault-relative path removed.
	pub path: String,
}

impl Vault {
	/// Lists directories and files at `path` (vault root when `None`),
	/// recursing `depth` levels for entries. Subtree totals always cover the
	/// whole subtree regardless of `depth`.
	pub fn ls(&self, path: Option<&str>, depth: usize) -> Result<Listing, WikidError> {
		let base = match path {
			Some(p) => Some(self.resolve(p)?),
			None => None,
		};
		if let Some(base) = &base {
			if !base.abs.exists() {
				return Err(WikidError::NotFound { path: base.rel.clone() });
			}
			if base.abs.is_file() {
				// Ignore rules apply to `ls` even for an explicitly named file:
				// a gitignored/ignored file is as invisible as its directory.
				if !self.visible_files()?.iter().any(|(rel, _)| *rel == base.rel) {
					return Err(WikidError::NotFound { path: base.rel.clone() });
				}
				let entry = entry_for(&base.rel, &fs::metadata(&base.abs)?)?;
				let (files, pages) = match entry.kind {
					EntryKind::Page => (0, 1),
					_ => (1, 0),
				};
				return Ok(Listing {
					entries: vec![entry],
					total_dirs: 0,
					total_files: files,
					total_pages: pages,
				});
			}
		}
		let base_rel = base.as_ref().map(|b| b.rel.as_str());
		// Ignored directories are invisible: the base must be seen in the walk.
		let mut base_seen = base_rel.is_none();
		let mut entries = Vec::new();
		let (mut total_dirs, mut total_files, mut total_pages) = (0usize, 0usize, 0usize);
		for result in self.walker() {
			let dent = result.map_err(walk_err)?;
			let rel_path = dent.path().strip_prefix(self.root()).unwrap_or(dent.path());
			if rel_path.as_os_str().is_empty() {
				continue; // the vault root itself
			}
			if !self.contained(&dent) {
				continue;
			}
			let rel = rel_path.to_string_lossy().into_owned();
			let within = match base_rel {
				Some(b) if rel == b => {
					base_seen = true;
					continue;
				}
				Some(b) => match rel.strip_prefix(&format!("{b}/")) {
					Some(sub) => sub,
					None => continue,
				},
				None => rel.as_str(),
			};
			let meta = dent.metadata().map_err(walk_err)?;
			let kind = kind_of(meta.is_dir(), &rel);
			match kind {
				EntryKind::Dir => total_dirs += 1,
				EntryKind::File => total_files += 1,
				EntryKind::Page => total_pages += 1,
			}
			if within.split('/').count() <= depth {
				entries.push(entry_for(&rel, &meta)?);
			}
		}
		if !base_seen {
			return Err(WikidError::NotFound {
				path: base_rel.unwrap_or_default().to_string(),
			});
		}
		entries.sort_by(|a, b| a.path.cmp(&b.path));
		Ok(Listing {
			entries,
			total_dirs,
			total_files,
			total_pages,
		})
	}

	/// Reads a file. `limit` truncates at a line boundary — `None` means the
	/// full content; callers wanting the default cap pass
	/// `Some(ReadLimit::default())` (400 lines / 32 KiB, whichever first).
	pub fn cat(&self, path: &str, limit: Option<ReadLimit>) -> Result<Document, WikidError> {
		let target = self.resolve(path)?;
		if !target.abs.exists() {
			return Err(WikidError::NotFound { path: target.rel });
		}
		if target.abs.is_dir() {
			return Err(WikidError::InvalidPath {
				path: target.rel,
				reason: "is a directory, not a file".into(),
			});
		}
		let bytes = fs::read(&target.abs)?;
		let text = String::from_utf8(bytes).map_err(|_| WikidError::NotUtf8 {
			path: target.rel.clone(),
		})?;
		let meta = fs::metadata(&target.abs)?;
		let total_lines = text.lines().count();
		let total_bytes = meta.len();
		let modified = rfc3339(meta.modified()?);
		let (content, truncated) = match limit {
			None => (text, false),
			Some(limit) => truncate(&text, limit),
		};
		Ok(Document {
			path: target.rel,
			content,
			truncated,
			total_lines,
			total_bytes,
			modified,
		})
	}

	/// Regex search over pages and UTF-8 text files. Binary files (NUL byte
	/// in the first 8 KiB) and non-UTF-8 files are skipped. Files whose path
	/// stem matches the pattern are ranked first.
	pub fn grep(&self, pattern: &str, opts: &GrepOptions) -> Result<GrepResult, WikidError> {
		let re = regex::RegexBuilder::new(pattern)
			.case_insensitive(opts.ignore_case)
			.build()
			.map_err(|e| WikidError::BadPattern {
				pattern: pattern.to_string(),
				reason: e.to_string(),
			})?;
		let mut files = self.visible_files()?;
		files.sort_by_cached_key(|(rel, _)| {
			let stem = Path::new(rel)
				.file_stem()
				.map(|s| s.to_string_lossy().into_owned())
				.unwrap_or_default();
			(!re.is_match(&stem), rel.clone())
		});
		let mut matches = Vec::new();
		let mut total_matches = 0usize;
		let mut matched_files = 0usize;
		let mut total_files = 0usize;
		let mut truncated = false;
		for (rel, abs) in files {
			let Ok(bytes) = fs::read(&abs) else { continue };
			if is_binary(&bytes) {
				continue;
			}
			let Ok(text) = String::from_utf8(bytes) else { continue };
			total_files += 1;
			let lines: Vec<&str> = text.lines().collect();
			let hit_lines: Vec<usize> = lines
				.iter()
				.enumerate()
				.filter(|(_, line)| re.is_match(line))
				.map(|(i, _)| i)
				.collect();
			if hit_lines.is_empty() {
				continue;
			}
			matched_files += 1;
			total_matches += hit_lines.len();
			if opts.files_only {
				if matches.len() < opts.limit {
					let first = hit_lines[0];
					matches.push(GrepMatch {
						path: rel.clone(),
						line: first + 1,
						text: lines[first].to_string(),
						context_before: None,
						context_after: None,
					});
				} else {
					truncated = true;
				}
			} else {
				for &i in &hit_lines {
					if matches.len() < opts.limit {
						matches.push(grep_match(&rel, &lines, i, opts.context));
					} else {
						truncated = true;
					}
				}
			}
		}
		Ok(GrepResult {
			matches,
			total_matches,
			matched_files,
			total_files,
			truncated,
		})
	}

	/// Glob match (globset syntax, `*` does not cross `/`) over vault-relative
	/// file paths, e.g. `**/*.md`. Sorted by path.
	pub fn glob(&self, pattern: &str) -> Result<GlobResult, WikidError> {
		let matcher = globset::GlobBuilder::new(pattern)
			.literal_separator(true)
			.build()
			.map_err(|e| WikidError::BadPattern {
				pattern: pattern.to_string(),
				reason: e.to_string(),
			})?
			.compile_matcher();
		let mut entries = Vec::new();
		for (rel, abs) in self.visible_files()? {
			if !matcher.is_match(&rel) {
				continue;
			}
			let Ok(meta) = fs::metadata(&abs) else { continue };
			entries.push(entry_for(&rel, &meta)?);
		}
		entries.sort_by(|a, b| a.path.cmp(&b.path));
		Ok(GlobResult {
			total: entries.len(),
			entries,
		})
	}

	/// Creates or overwrites a file atomically (temp file in the target's
	/// parent dir + rename). Parent directories are created as needed.
	pub fn write(&self, path: &str, content: &str) -> Result<WriteResult, WikidError> {
		let target = self.resolve(path)?;
		if target.abs.is_dir() {
			return Err(WikidError::InvalidPath {
				path: target.rel,
				reason: "is a directory, not a file".into(),
			});
		}
		let created = !target.abs.exists();
		atomic_write(&target.abs, content)?;
		Ok(WriteResult {
			path: target.rel,
			created,
			bytes: content.len() as u64,
		})
	}

	/// Literal (non-regex) string replacement. With `all` false the old text
	/// must occur exactly once: zero occurrences fail with `NoMatch` (plus a
	/// best-effort nearest-line hint), several with `Ambiguous`. With `all`
	/// true every occurrence is replaced. The write is atomic.
	pub fn edit(&self, path: &str, old: &str, new: &str, all: bool) -> Result<EditResult, WikidError> {
		if old.is_empty() {
			return Err(WikidError::BadPattern {
				pattern: String::new(),
				reason: "old text must not be empty".into(),
			});
		}
		let target = self.resolve(path)?;
		if !target.abs.exists() {
			return Err(WikidError::NotFound { path: target.rel });
		}
		if target.abs.is_dir() {
			return Err(WikidError::InvalidPath {
				path: target.rel,
				reason: "is a directory, not a file".into(),
			});
		}
		let bytes = fs::read(&target.abs)?;
		let text = String::from_utf8(bytes).map_err(|_| WikidError::NotUtf8 {
			path: target.rel.clone(),
		})?;
		let count = text.matches(old).count();
		match (count, all) {
			(0, _) => Err(WikidError::NoMatch {
				path: target.rel,
				nearest_line: nearest_line(&text, old),
			}),
			(1, _) | (_, true) => {
				let updated = text.replace(old, new);
				atomic_write(&target.abs, &updated)?;
				Ok(EditResult {
					path: target.rel,
					replacements: count,
					bytes: updated.len() as u64,
				})
			}
			(count, false) => Err(WikidError::Ambiguous {
				path: target.rel,
				count,
			}),
		}
	}

	/// Renames a file (never a directory). Destination parent directories are
	/// created; an existing destination is refused unless `force`.
	pub fn mv(&self, from: &str, to: &str, force: bool) -> Result<MvResult, WikidError> {
		let src = self.resolve(from)?;
		if !src.abs.exists() {
			return Err(WikidError::NotFound { path: src.rel });
		}
		if src.abs.is_dir() {
			return Err(WikidError::InvalidPath {
				path: src.rel,
				reason: "directories cannot be moved".into(),
			});
		}
		let dst = self.resolve(to)?;
		if dst.abs.exists() {
			if dst.abs.is_dir() {
				return Err(WikidError::InvalidPath {
					path: dst.rel,
					reason: "destination is a directory".into(),
				});
			}
			if !force {
				return Err(WikidError::AlreadyExists { path: dst.rel });
			}
		}
		// abs always has a parent: resolve() guarantees at least one component
		// under the vault root.
		fs::create_dir_all(dst.abs.parent().expect("resolved path has a parent"))?;
		fs::rename(&src.abs, &dst.abs)?;
		Ok(MvResult {
			from: src.rel,
			to: dst.rel,
		})
	}

	/// Deletes a file (never a directory). The `--force` confirmation gate
	/// lives at the CLI/HTTP layer, not here.
	pub fn rm(&self, path: &str) -> Result<RmResult, WikidError> {
		let target = self.resolve(path)?;
		if !target.abs.exists() {
			return Err(WikidError::NotFound { path: target.rel });
		}
		if target.abs.is_dir() {
			return Err(WikidError::InvalidPath {
				path: target.rel,
				reason: "directories cannot be removed".into(),
			});
		}
		fs::remove_file(&target.abs)?;
		Ok(RmResult { path: target.rel })
	}

	/// All non-directory entries visible under the ignore rules, as
	/// (vault-relative path, absolute path) pairs in walk order.
	pub(crate) fn visible_files(&self) -> Result<Vec<(String, PathBuf)>, WikidError> {
		let mut files = Vec::new();
		for result in self.walker() {
			let dent = result.map_err(walk_err)?;
			let rel_path = dent.path().strip_prefix(self.root()).unwrap_or(dent.path());
			if rel_path.as_os_str().is_empty() {
				continue;
			}
			if dent.file_type().is_some_and(|t| t.is_dir()) {
				continue;
			}
			if !self.contained(&dent) {
				continue;
			}
			files.push((rel_path.to_string_lossy().into_owned(), dent.into_path()));
		}
		Ok(files)
	}

	/// True when a walked entry stays inside the vault. The walker never
	/// follows links, but still yields symlink entries themselves — reading
	/// through one pointing outside the vault would disclose out-of-vault
	/// content (DESIGN §2: such symlinks are refused). Symlinks are
	/// canonicalized and containment-checked; broken ones are dropped too.
	fn contained(&self, dent: &ignore::DirEntry) -> bool {
		if !dent.path_is_symlink() {
			return true;
		}
		dent.path()
			.canonicalize()
			.is_ok_and(|canonical| canonical.starts_with(self.root()))
	}
}

fn walk_err(err: ignore::Error) -> WikidError {
	WikidError::Io(std::io::Error::other(err))
}

pub(crate) fn rfc3339(time: SystemTime) -> String {
	humantime::format_rfc3339_seconds(time).to_string()
}

/// True when `bytes` sniff as binary: a NUL byte within the first 8 KiB.
pub(crate) fn is_binary(bytes: &[u8]) -> bool {
	bytes[..bytes.len().min(BINARY_SNIFF_BYTES)].contains(&0)
}

/// True when a vault-relative path denotes a page (`.md`, case-insensitive).
pub(crate) fn is_page(rel: &str) -> bool {
	Path::new(rel)
		.extension()
		.is_some_and(|ext| ext.eq_ignore_ascii_case("md"))
}

/// Reads a file as text: `Ok(None)` when binary or not UTF-8 (a deliberate
/// content-check skip), `Err` when the file cannot be read at all — callers
/// must not silently drop unreadable files from their results.
pub(crate) fn read_text(abs: &Path) -> Result<Option<String>, WikidError> {
	let bytes = fs::read(abs)?;
	if is_binary(&bytes) {
		return Ok(None);
	}
	Ok(String::from_utf8(bytes).ok())
}

fn kind_of(is_dir: bool, rel: &str) -> EntryKind {
	if is_dir {
		EntryKind::Dir
	} else if Path::new(rel)
		.extension()
		.is_some_and(|ext| ext.eq_ignore_ascii_case("md"))
	{
		EntryKind::Page
	} else {
		EntryKind::File
	}
}

fn entry_for(rel: &str, meta: &fs::Metadata) -> Result<Entry, WikidError> {
	let is_dir = meta.is_dir();
	Ok(Entry {
		path: if is_dir { format!("{rel}/") } else { rel.to_string() },
		kind: kind_of(is_dir, rel),
		size: if is_dir { 0 } else { meta.len() },
		modified: rfc3339(meta.modified()?),
	})
}

/// Cuts `text` at whole-line boundaries once either limit would be exceeded.
fn truncate(text: &str, limit: ReadLimit) -> (String, bool) {
	let mut out = String::new();
	// `emitted` lines are already in `out`, so `emitted + 1` is this line.
	for (emitted, line) in text.split_inclusive('\n').enumerate() {
		if emitted + 1 > limit.max_lines || out.len() + line.len() > limit.max_bytes {
			return (out, true);
		}
		out.push_str(line);
	}
	(out, false)
}

fn grep_match(rel: &str, lines: &[&str], index: usize, context: usize) -> GrepMatch {
	let (before, after) = if context == 0 {
		(None, None)
	} else {
		let start = index.saturating_sub(context);
		let end = (index + 1 + context).min(lines.len());
		(
			Some(lines[start..index].iter().map(|l| l.to_string()).collect()),
			Some(lines[index + 1..end].iter().map(|l| l.to_string()).collect()),
		)
	};
	GrepMatch {
		path: rel.to_string(),
		line: index + 1,
		text: lines[index].to_string(),
		context_before: before,
		context_after: after,
	}
}

/// Atomic write: temp file in the target's parent directory, then rename.
fn atomic_write(abs: &Path, content: &str) -> Result<(), WikidError> {
	// Write through in-vault symlinks instead of replacing them with regular
	// files: the rename target is the canonical destination. Containment was
	// already verified by `resolve()`; a missing file canonicalizes to itself.
	let abs = &abs.canonicalize().unwrap_or_else(|_| abs.to_path_buf());
	let parent = abs.parent().expect("resolved path has a parent");
	fs::create_dir_all(parent)?;
	let mut tmp = tempfile::NamedTempFile::new_in(parent)?;
	tmp.write_all(content.as_bytes())?;
	// `NamedTempFile` is created 0600; overwriting must keep the destination's
	// existing permissions instead of silently resetting them.
	if let Ok(meta) = fs::metadata(abs) {
		fs::set_permissions(tmp.path(), meta.permissions())?;
	}
	tmp.persist(abs).map_err(|e| WikidError::Io(e.error))?;
	Ok(())
}

/// Best-effort locator for edit misses: the 1-based line most similar to the
/// first non-empty line of the needle, by character-bigram Dice overlap.
fn nearest_line(text: &str, old: &str) -> Option<usize> {
	let needle = old.lines().find(|l| !l.trim().is_empty())?.trim();
	let needle_bigrams = bigrams(needle);
	if needle_bigrams.is_empty() {
		return None;
	}
	let mut best: Option<(usize, f64)> = None;
	for (i, line) in text.lines().enumerate() {
		let line_bigrams = bigrams(line.trim());
		if line_bigrams.is_empty() {
			continue;
		}
		let overlap = needle_bigrams.intersection(&line_bigrams).count();
		let score = 2.0 * overlap as f64 / (needle_bigrams.len() + line_bigrams.len()) as f64;
		if best.is_none_or(|(_, s)| score > s) {
			best = Some((i + 1, score));
		}
	}
	best.filter(|&(_, score)| score >= NEAREST_LINE_MIN_SCORE)
		.map(|(line, _)| line)
}

fn bigrams(s: &str) -> HashSet<(char, char)> {
	let chars: Vec<char> = s.chars().collect();
	chars.windows(2).map(|w| (w[0], w[1])).collect()
}

#[cfg(test)]
mod tests {
	use super::*;
	use crate::test_fixtures::{self, ALPHA_MD, UNICODE_MD};

	fn fixture() -> (tempfile::TempDir, Vault) {
		test_fixtures::vault()
	}

	fn paths(entries: &[Entry]) -> Vec<&str> {
		entries.iter().map(|e| e.path.as_str()).collect()
	}

	// --- ls ---

	#[test]
	fn ls_root_depth_one_lists_visible_entries_with_subtree_totals() {
		let (_dir, vault) = fixture();
		let listing = vault.ls(None, 1).unwrap();
		assert_eq!(
			paths(&listing.entries),
			vec!["attachments/", "index.md", "notes/", "projects/"]
		);
		// Totals cover the whole subtree even though depth is 1.
		assert_eq!(listing.total_dirs, 3);
		assert_eq!(listing.total_files, 2);
		assert_eq!(listing.total_pages, 4);
	}

	#[test]
	fn ls_deeper_depth_includes_nested_entries() {
		let (_dir, vault) = fixture();
		let listing = vault.ls(None, 2).unwrap();
		assert!(paths(&listing.entries).contains(&"projects/alpha.md"));
		assert_eq!(listing.total_pages, 4);
	}

	#[test]
	fn ls_hidden_dirs_are_invisible() {
		let (_dir, vault) = fixture();
		let listing = vault.ls(None, 3).unwrap();
		for entry in &listing.entries {
			assert!(!entry.path.contains(".obsidian"), "hidden dir leaked: {}", entry.path);
			assert!(!entry.path.contains(".trash"), "hidden dir leaked: {}", entry.path);
			assert!(!entry.path.starts_with(".git"), "hidden file leaked: {}", entry.path);
		}
	}

	#[test]
	fn ls_gitignored_dirs_are_invisible_even_without_git() {
		let (_dir, vault) = fixture();
		let listing = vault.ls(None, 3).unwrap();
		assert!(!paths(&listing.entries).iter().any(|p| p.starts_with("drafts")));
		// Explicitly addressing an ignored directory behaves as not-found.
		assert!(matches!(vault.ls(Some("drafts"), 1), Err(WikidError::NotFound { .. })));
		// ...and so does explicitly addressing an ignored file.
		assert!(matches!(
			vault.ls(Some("drafts/wip.md"), 1),
			Err(WikidError::NotFound { .. })
		));
	}

	#[test]
	fn ls_subdirectory_returns_full_relative_paths() {
		let (_dir, vault) = fixture();
		let listing = vault.ls(Some("projects"), 1).unwrap();
		assert_eq!(paths(&listing.entries), vec!["projects/alpha.md", "projects/beta.md"]);
		assert_eq!(
			(listing.total_dirs, listing.total_files, listing.total_pages),
			(0, 0, 2)
		);
	}

	#[test]
	fn ls_single_file_target() {
		let (_dir, vault) = fixture();
		let listing = vault.ls(Some("index.md"), 1).unwrap();
		assert_eq!(paths(&listing.entries), vec!["index.md"]);
		assert_eq!(listing.entries[0].kind, EntryKind::Page);
		assert_eq!(listing.total_pages, 1);
	}

	#[test]
	fn ls_missing_and_hidden_paths_fail() {
		let (_dir, vault) = fixture();
		assert!(matches!(vault.ls(Some("nope"), 1), Err(WikidError::NotFound { .. })));
		assert!(matches!(
			vault.ls(Some(".obsidian"), 1),
			Err(WikidError::InvalidPath { .. })
		));
	}

	#[test]
	fn ls_entries_have_rfc3339_utc_timestamps() {
		let (_dir, vault) = fixture();
		let listing = vault.ls(None, 1).unwrap();
		for entry in &listing.entries {
			assert_eq!(entry.modified.len(), 20, "unexpected timestamp: {}", entry.modified);
			assert!(entry.modified.ends_with('Z') && entry.modified.contains('T'));
		}
	}

	// --- cat ---

	#[test]
	fn cat_full_returns_unicode_content_intact() {
		let (_dir, vault) = fixture();
		let doc = vault.cat("notes/unicode.md", None).unwrap();
		assert_eq!(doc.content, UNICODE_MD);
		assert!(!doc.truncated);
		assert_eq!(doc.total_bytes, UNICODE_MD.len() as u64);
	}

	#[test]
	fn cat_truncates_at_line_limit_with_metadata() {
		let (_dir, vault) = fixture();
		let content: String = (1..=500).map(|i| format!("line {i}\n")).collect();
		vault.write("big.md", &content).unwrap();
		let doc = vault.cat("big.md", Some(ReadLimit::default())).unwrap();
		assert!(doc.truncated);
		assert_eq!(doc.content.lines().count(), 400);
		assert_eq!(doc.total_lines, 500);
		assert_eq!(doc.total_bytes, content.len() as u64);
		// None = full content.
		let full = vault.cat("big.md", None).unwrap();
		assert!(!full.truncated);
		assert_eq!(full.content, content);
	}

	#[test]
	fn cat_truncates_at_byte_limit_on_line_boundary() {
		let (_dir, vault) = fixture();
		let content = "aaaa\nbbbb\ncccc\n";
		vault.write("bytes.md", content).unwrap();
		let doc = vault
			.cat(
				"bytes.md",
				Some(ReadLimit {
					max_lines: 100,
					max_bytes: 12,
				}),
			)
			.unwrap();
		assert!(doc.truncated);
		assert_eq!(doc.content, "aaaa\nbbbb\n");
		assert_eq!(doc.total_lines, 3);
	}

	#[test]
	fn cat_within_limit_is_not_truncated() {
		let (_dir, vault) = fixture();
		let doc = vault.cat("index.md", Some(ReadLimit::default())).unwrap();
		assert!(!doc.truncated);
	}

	#[test]
	fn cat_binary_file_is_not_utf8() {
		let (_dir, vault) = fixture();
		assert!(matches!(
			vault.cat("attachments/logo.png", None),
			Err(WikidError::NotUtf8 { .. })
		));
	}

	#[test]
	fn cat_rejects_missing_dirs_and_hidden() {
		let (_dir, vault) = fixture();
		assert!(matches!(vault.cat("nope.md", None), Err(WikidError::NotFound { .. })));
		assert!(matches!(
			vault.cat("projects", None),
			Err(WikidError::InvalidPath { .. })
		));
		assert!(matches!(
			vault.cat(".obsidian/app.json", None),
			Err(WikidError::InvalidPath { .. })
		));
	}

	// --- path safety through operations ---

	#[test]
	fn operations_reject_escaping_and_absolute_paths() {
		let (_dir, vault) = fixture();
		for bad in ["../x", "/etc/passwd", "a/../../b", ""] {
			assert!(
				matches!(vault.cat(bad, None), Err(WikidError::InvalidPath { .. })),
				"cat accepted bad path {bad:?}"
			);
			assert!(
				matches!(vault.write(bad, "x"), Err(WikidError::InvalidPath { .. })),
				"write accepted bad path {bad:?}"
			);
			assert!(
				matches!(vault.rm(bad), Err(WikidError::InvalidPath { .. })),
				"rm accepted bad path {bad:?}"
			);
		}
		// Normalization keeps in-vault dotted traversal working.
		assert!(vault.cat("projects/../index.md", None).is_ok());
	}

	#[cfg(unix)]
	#[test]
	fn operations_refuse_symlinks_out_of_the_vault() {
		let outside = tempfile::tempdir().unwrap();
		std::fs::write(outside.path().join("secret.txt"), "top secret").unwrap();
		let (dir, vault) = fixture();
		std::os::unix::fs::symlink(outside.path().join("secret.txt"), dir.path().join("escape.md")).unwrap();
		assert!(matches!(
			vault.cat("escape.md", None),
			Err(WikidError::InvalidPath { .. })
		));
		std::os::unix::fs::symlink(outside.path(), dir.path().join("linkdir")).unwrap();
		assert!(matches!(
			vault.write("linkdir/new.md", "x"),
			Err(WikidError::InvalidPath { .. })
		));
	}

	#[cfg(unix)]
	#[test]
	fn walker_operations_skip_symlinks_out_of_the_vault() {
		let outside = tempfile::tempdir().unwrap();
		std::fs::write(outside.path().join("secret.md"), "top secret\n").unwrap();
		let (dir, vault) = fixture();
		std::os::unix::fs::symlink(outside.path().join("secret.md"), dir.path().join("escape.md")).unwrap();
		std::os::unix::fs::symlink(dir.path().join("void.md"), dir.path().join("dangling.md")).unwrap();
		// grep must not read through the link (content disclosure), and the
		// link must not count as a searched file.
		let result = vault.grep("top secret", &GrepOptions::default()).unwrap();
		assert_eq!(result.total_matches, 0);
		assert_eq!(result.total_files, 4);
		// ls and glob must not list it (nor the broken link).
		let listing = vault.ls(None, 1).unwrap();
		assert!(
			!paths(&listing.entries)
				.iter()
				.any(|p| p.contains("escape") || p.contains("dangling")),
			"symlink leaked into ls: {:?}",
			paths(&listing.entries)
		);
		assert_eq!(listing.total_pages, 4);
		let globbed = vault.glob("*.md").unwrap();
		assert_eq!(paths(&globbed.entries), vec!["index.md"]);
		// In-vault symlinks stay visible.
		std::os::unix::fs::symlink(dir.path().join("index.md"), dir.path().join("alias.md")).unwrap();
		let globbed = vault.glob("*.md").unwrap();
		assert_eq!(paths(&globbed.entries), vec!["alias.md", "index.md"]);
	}

	#[cfg(unix)]
	#[test]
	fn write_preserves_existing_file_permissions() {
		use std::os::unix::fs::PermissionsExt;
		let (dir, vault) = fixture();
		let path = dir.path().join("index.md");
		std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o644)).unwrap();
		vault.write("index.md", "overwritten\n").unwrap();
		let mode = std::fs::metadata(&path).unwrap().permissions().mode() & 0o777;
		assert_eq!(mode, 0o644, "write reset the file mode");
		vault.edit("index.md", "overwritten", "edited", false).unwrap();
		let mode = std::fs::metadata(&path).unwrap().permissions().mode() & 0o777;
		assert_eq!(mode, 0o644, "edit reset the file mode");
	}

	#[cfg(unix)]
	#[test]
	fn write_through_an_in_vault_symlink_keeps_the_link() {
		let (dir, vault) = fixture();
		std::os::unix::fs::symlink(dir.path().join("index.md"), dir.path().join("alias.md")).unwrap();
		vault.write("alias.md", "via alias\n").unwrap();
		assert!(
			std::fs::symlink_metadata(dir.path().join("alias.md"))
				.unwrap()
				.file_type()
				.is_symlink(),
			"write replaced the symlink with a regular file"
		);
		assert_eq!(
			std::fs::read_to_string(dir.path().join("index.md")).unwrap(),
			"via alias\n"
		);
	}

	// --- grep ---

	#[test]
	fn grep_ranks_stem_matches_first_and_counts_all() {
		let (_dir, vault) = fixture();
		let result = vault.grep("alpha", &GrepOptions::default()).unwrap();
		// alpha.md is ranked first (stem match) although index.md sorts earlier.
		assert_eq!(result.matches[0].path, "projects/alpha.md");
		assert_eq!(result.matches[0].line, 7);
		assert_eq!(result.total_matches, 3);
		// Matches live in alpha.md and index.md only.
		assert_eq!(result.matched_files, 2);
		// index.md, alpha.md, beta.md, unicode.md are searched; binaries are not.
		assert_eq!(result.total_files, 4);
		assert!(!result.truncated);
	}

	#[test]
	fn grep_ignore_case() {
		let (_dir, vault) = fixture();
		let sensitive = vault.grep("ALPHA", &GrepOptions::default()).unwrap();
		assert_eq!(sensitive.total_matches, 0);
		let insensitive = vault
			.grep(
				"ALPHA",
				&GrepOptions {
					ignore_case: true,
					..Default::default()
				},
			)
			.unwrap();
		assert_eq!(insensitive.total_matches, 5);
	}

	#[test]
	fn grep_files_only_emits_one_entry_per_file() {
		let (_dir, vault) = fixture();
		let result = vault
			.grep(
				"alpha",
				&GrepOptions {
					files_only: true,
					..Default::default()
				},
			)
			.unwrap();
		assert_eq!(paths_of(&result.matches), vec!["projects/alpha.md", "index.md"]);
		assert_eq!(result.total_matches, 3);
	}

	#[test]
	fn grep_context_lines() {
		let (_dir, vault) = fixture();
		let result = vault
			.grep(
				"status: green",
				&GrepOptions {
					context: 1,
					..Default::default()
				},
			)
			.unwrap();
		assert_eq!(result.matches.len(), 1);
		let m = &result.matches[0];
		assert_eq!(m.context_before.as_deref(), Some(&[String::new()][..]));
		assert_eq!(m.context_after.as_deref(), Some(&["second alpha line".to_string()][..]));
		// Without context the fields stay off the wire entirely.
		let plain = vault.grep("status: green", &GrepOptions::default()).unwrap();
		assert!(plain.matches[0].context_before.is_none());
		let json = serde_json::to_string(&plain).unwrap();
		assert!(!json.contains("context_before"));
	}

	#[test]
	fn grep_limit_truncates_but_still_counts() {
		let (_dir, vault) = fixture();
		let result = vault
			.grep(
				"alpha",
				&GrepOptions {
					limit: 2,
					..Default::default()
				},
			)
			.unwrap();
		assert_eq!(result.matches.len(), 2);
		assert_eq!(result.total_matches, 3);
		assert!(result.truncated);
	}

	#[test]
	fn grep_skips_binary_and_hidden_and_ignored_files() {
		let (_dir, vault) = fixture();
		// data.bin is valid UTF-8 but has a NUL byte: the sniff must skip it.
		let result = vault.grep("needle", &GrepOptions::default()).unwrap();
		assert_eq!(result.total_matches, 0);
		assert!(result.matches.is_empty());
		// logo.png mentions alpha but is binary.
		let alpha = vault.grep("alpha", &GrepOptions::default()).unwrap();
		assert!(alpha.matches.iter().all(|m| !m.path.contains("logo.png")));
		// Hidden .trash and gitignored drafts are unreachable.
		assert_eq!(vault.grep("# Old", &GrepOptions::default()).unwrap().total_matches, 0);
		assert_eq!(vault.grep("# WIP", &GrepOptions::default()).unwrap().total_matches, 0);
	}

	#[test]
	fn grep_bad_pattern() {
		let (_dir, vault) = fixture();
		assert!(matches!(
			vault.grep("(", &GrepOptions::default()),
			Err(WikidError::BadPattern { .. })
		));
	}

	#[test]
	fn grep_unicode_content() {
		let (_dir, vault) = fixture();
		let result = vault.grep("Київ", &GrepOptions::default()).unwrap();
		assert_eq!(result.total_matches, 1);
		assert_eq!(result.matches[0].path, "notes/unicode.md");
	}

	fn paths_of(matches: &[GrepMatch]) -> Vec<&str> {
		matches.iter().map(|m| m.path.as_str()).collect()
	}

	// --- glob ---

	#[test]
	fn glob_matches_pages_sorted_and_ignores_hidden() {
		let (_dir, vault) = fixture();
		let result = vault.glob("**/*.md").unwrap();
		assert_eq!(
			paths(&result.entries),
			vec!["index.md", "notes/unicode.md", "projects/alpha.md", "projects/beta.md"]
		);
		assert_eq!(result.total, 4);
		assert_eq!(result.entries[0].kind, EntryKind::Page);
	}

	#[test]
	fn glob_matches_attachments() {
		let (_dir, vault) = fixture();
		let result = vault.glob("attachments/*").unwrap();
		assert_eq!(
			paths(&result.entries),
			vec!["attachments/data.bin", "attachments/logo.png"]
		);
		assert!(result.entries.iter().all(|e| e.kind == EntryKind::File));
	}

	#[test]
	fn glob_star_does_not_cross_directories() {
		let (_dir, vault) = fixture();
		let result = vault.glob("*.md").unwrap();
		assert_eq!(paths(&result.entries), vec!["index.md"]);
	}

	#[test]
	fn glob_bad_pattern() {
		let (_dir, vault) = fixture();
		assert!(matches!(vault.glob("a["), Err(WikidError::BadPattern { .. })));
	}

	// --- write ---

	#[test]
	fn write_creates_parents_and_reports_created() {
		let (dir, vault) = fixture();
		let result = vault.write("new/deep/page.md", "hello\n").unwrap();
		assert!(result.created);
		assert_eq!(result.bytes, 6);
		assert_eq!(
			std::fs::read_to_string(dir.path().join("new/deep/page.md")).unwrap(),
			"hello\n"
		);
		// Atomicity leftovers check: the temp file must not linger.
		let names: Vec<_> = std::fs::read_dir(dir.path().join("new/deep")).unwrap().collect();
		assert_eq!(names.len(), 1);
	}

	#[test]
	fn write_overwrites_atomically() {
		let (dir, vault) = fixture();
		let result = vault.write("index.md", "replaced\n").unwrap();
		assert!(!result.created);
		assert_eq!(
			std::fs::read_to_string(dir.path().join("index.md")).unwrap(),
			"replaced\n"
		);
		let names: Vec<_> = std::fs::read_dir(dir.path())
			.unwrap()
			.map(|e| e.unwrap().file_name().to_string_lossy().into_owned())
			.filter(|n| n.starts_with(".tmp"))
			.collect();
		assert!(names.is_empty(), "temp files left behind: {names:?}");
	}

	#[test]
	fn write_rejects_hidden_and_directory_targets() {
		let (_dir, vault) = fixture();
		assert!(matches!(
			vault.write(".obsidian/x.md", "x"),
			Err(WikidError::InvalidPath { .. })
		));
		assert!(matches!(
			vault.write("projects", "x"),
			Err(WikidError::InvalidPath { .. })
		));
	}

	// --- edit ---

	#[test]
	fn edit_single_match_replaces() {
		let (_dir, vault) = fixture();
		let result = vault
			.edit("projects/alpha.md", "status: green", "status: blue", false)
			.unwrap();
		assert_eq!(result.replacements, 1);
		let doc = vault.cat("projects/alpha.md", None).unwrap();
		assert!(doc.content.contains("status: blue"));
		assert!(!doc.content.contains("status: green"));
		assert_eq!(result.bytes, doc.total_bytes);
	}

	#[test]
	fn edit_zero_matches_hints_nearest_line() {
		let (_dir, vault) = fixture();
		let err = vault
			.edit("projects/alpha.md", "alpha status: red", "x", false)
			.unwrap_err();
		// "alpha status: green" is line 7 of the fixture page.
		assert_eq!(ALPHA_MD.lines().nth(6), Some("alpha status: green"));
		assert!(
			matches!(
				err,
				WikidError::NoMatch {
					nearest_line: Some(7),
					..
				}
			),
			"got {err:?}"
		);
		assert!(err.hint().unwrap().contains("line 7"));
	}

	#[test]
	fn edit_zero_matches_without_similar_content() {
		let (_dir, vault) = fixture();
		let err = vault
			.edit("projects/alpha.md", "zzzz qqqq wwww", "x", false)
			.unwrap_err();
		assert!(
			matches!(err, WikidError::NoMatch { nearest_line: None, .. }),
			"got {err:?}"
		);
	}

	#[test]
	fn edit_multiple_matches_is_ambiguous_unless_all() {
		let (_dir, vault) = fixture();
		let err = vault.edit("projects/alpha.md", "alpha", "beta", false).unwrap_err();
		assert!(matches!(err, WikidError::Ambiguous { count: 2, .. }), "got {err:?}");
		let result = vault.edit("projects/alpha.md", "alpha", "beta", true).unwrap();
		assert_eq!(result.replacements, 2);
		let content = vault.cat("projects/alpha.md", None).unwrap().content;
		assert!(!content.contains("alpha"));
	}

	#[test]
	fn edit_unicode_content() {
		let (_dir, vault) = fixture();
		let result = vault.edit("notes/unicode.md", "Київ", "Львів", false).unwrap();
		assert_eq!(result.replacements, 1);
		assert!(
			vault
				.cat("notes/unicode.md", None)
				.unwrap()
				.content
				.contains("Львів — місто.")
		);
	}

	#[test]
	fn edit_rejects_empty_old_and_missing_files() {
		let (_dir, vault) = fixture();
		assert!(matches!(
			vault.edit("index.md", "", "x", false),
			Err(WikidError::BadPattern { .. })
		));
		assert!(matches!(
			vault.edit("nope.md", "a", "b", false),
			Err(WikidError::NotFound { .. })
		));
		assert!(matches!(
			vault.edit("attachments/logo.png", "a", "b", false),
			Err(WikidError::NotUtf8 { .. })
		));
	}

	// --- mv ---

	#[test]
	fn mv_moves_files_and_creates_parents() {
		let (dir, vault) = fixture();
		let result = vault.mv("projects/beta.md", "archive/2026/beta.md", false).unwrap();
		assert_eq!(result.to, "archive/2026/beta.md");
		assert!(!dir.path().join("projects/beta.md").exists());
		assert!(dir.path().join("archive/2026/beta.md").exists());
	}

	#[test]
	fn mv_refuses_existing_destination_unless_forced() {
		let (_dir, vault) = fixture();
		let err = vault.mv("projects/beta.md", "index.md", false).unwrap_err();
		assert!(matches!(err, WikidError::AlreadyExists { .. }));
		vault.mv("projects/beta.md", "index.md", true).unwrap();
		assert!(vault.cat("index.md", None).unwrap().content.contains("Beta"));
	}

	#[test]
	fn mv_rejects_directories_and_missing_sources() {
		let (_dir, vault) = fixture();
		assert!(matches!(
			vault.mv("projects", "archive", false),
			Err(WikidError::InvalidPath { .. })
		));
		assert!(matches!(
			vault.mv("nope.md", "x.md", false),
			Err(WikidError::NotFound { .. })
		));
		assert!(matches!(
			vault.mv("index.md", "projects", false),
			Err(WikidError::InvalidPath { .. })
		));
	}

	// --- rm ---

	#[test]
	fn rm_deletes_files_only() {
		let (dir, vault) = fixture();
		vault.rm("projects/beta.md").unwrap();
		assert!(!dir.path().join("projects/beta.md").exists());
		assert!(matches!(vault.rm("projects"), Err(WikidError::InvalidPath { .. })));
		assert!(matches!(vault.rm("projects/beta.md"), Err(WikidError::NotFound { .. })));
	}

	// --- wire format ---

	#[test]
	fn result_structs_round_trip_as_json() {
		let (_dir, vault) = fixture();
		let listing = vault.ls(None, 2).unwrap();
		let json = serde_json::to_string(&listing).unwrap();
		assert!(json.contains("\"kind\":\"page\""));
		assert!(json.contains("\"kind\":\"dir\""));
		let back: Listing = serde_json::from_str(&json).unwrap();
		assert_eq!(listing, back);

		let grep = vault
			.grep(
				"alpha",
				&GrepOptions {
					context: 1,
					..Default::default()
				},
			)
			.unwrap();
		let back: GrepResult = serde_json::from_str(&serde_json::to_string(&grep).unwrap()).unwrap();
		assert_eq!(grep, back);

		let doc = vault.cat("index.md", None).unwrap();
		let back: Document = serde_json::from_str(&serde_json::to_string(&doc).unwrap()).unwrap();
		assert_eq!(doc, back);
	}
}
