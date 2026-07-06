//! Human-readable renderers (DESIGN §6): compact text, pre-computed totals,
//! explicit zero-result messages, and trailing `hint:` lines. `--json` mode
//! bypasses this module entirely — it serializes the core structs directly.
//!
//! Every function returns the full output without a trailing newline; the
//! caller prints it with `println!`.

use wikid_core::{
	Document, EditResult, Entry, EntryKind, GlobResult, GrepResult, HashlinesResult, HealthReport, IssueCategory,
	LinkReport, Listing, MvResult, RmResult, Severity, TagReport, VaultStatus, WriteResult,
};

/// Formats bytes as a compact human size (B / KiB / MiB).
fn human_size(bytes: u64) -> String {
	const KIB: f64 = 1024.0;
	if bytes < 1024 {
		format!("{bytes} B")
	} else if (bytes as f64) < KIB * KIB {
		format!("{:.1} KiB", bytes as f64 / KIB)
	} else {
		format!("{:.1} MiB", bytes as f64 / (KIB * KIB))
	}
}

/// One listing line: dirs are bare paths, files show path, size, modified.
fn entry_line(entry: &Entry) -> String {
	match entry.kind {
		EntryKind::Dir => entry.path.clone(),
		_ => format!("{}  {}  {}", entry.path, human_size(entry.size), entry.modified),
	}
}

fn severity_name(severity: Severity) -> &'static str {
	match severity {
		Severity::High => "high",
		Severity::Medium => "medium",
		Severity::Low => "low",
	}
}

pub fn status(status: &VaultStatus, remote: bool) -> String {
	let wiki_name = status
		.root
		.rsplit(['/', '\\'])
		.find(|part| !part.is_empty())
		.unwrap_or("selected");
	let root_label = if remote { "root (server)" } else { "root" };
	let mut lines = vec![
		format!("wiki: {wiki_name}"),
		format!("version: {}", status.version),
		format!("{root_label}: {}", status.root),
		format!(
			"pages: {}  files: {}  size: {}",
			status.total_pages,
			status.total_files,
			human_size(status.total_bytes)
		),
	];
	let health = &status.doctor_summary;
	lines.push(format!(
		"health: {} high  {} medium  {} low",
		health.high, health.medium, health.low
	));
	if !status.recent.is_empty() {
		lines.push("recent:".to_string());
		for page in &status.recent {
			lines.push(format!("  {}  {}", page.modified, page.path));
		}
	}
	lines.push("hint: wikid grep <pattern> — search this wiki".to_string());
	lines.push("hint: wikid doctor — inspect structural issues".to_string());
	lines.join("\n")
}

pub fn listing(listing: &Listing, tree: bool) -> String {
	let mut lines: Vec<String> = listing.entries.iter().map(entry_line).collect();
	lines.push(format!(
		"total: {} dirs, {} files, {} pages",
		listing.total_dirs, listing.total_files, listing.total_pages
	));
	lines.push("hint: wikid cat <path> — read a page".to_string());
	if !tree {
		lines.push("hint: wikid tree <path> — see deeper structure".to_string());
	}
	lines.join("\n")
}

pub fn document(doc: &Document) -> String {
	let mut lines: Vec<String> = Vec::new();
	if !doc.content.is_empty() {
		lines.push(doc.content.strip_suffix('\n').unwrap_or(&doc.content).to_string());
	}
	if let (Some(start), Some(end)) = (doc.range_start, doc.range_end) {
		lines.push(format!(
			"lines {start}-{end} of {} ({} bytes total)",
			doc.total_lines, doc.total_bytes
		));
		if end < doc.total_lines {
			let next_start = end + 1;
			let next_end = (end + (end - start + 1)).min(doc.total_lines);
			lines.push(format!(
				"hint: wikid cat {} --lines {next_start}-{next_end} — read the next window",
				doc.path
			));
		} else {
			lines.push(format!("hint: wikid links {} — outgoing links and backlinks", doc.path));
		}
	} else if doc.truncated {
		lines.push(format!(
			"… truncated ({} lines / {} bytes total) — use --full or --lines <START-END>",
			doc.total_lines, doc.total_bytes
		));
		lines.push(format!("hint: wikid cat {} --lines 1-120 — read a window", doc.path));
	} else {
		lines.push(format!("hint: wikid links {} — outgoing links and backlinks", doc.path));
	}
	lines.join("\n")
}

pub fn grep(result: &GrepResult, pattern: &str, files_only: bool, ignore_case: bool) -> String {
	if result.total_matches == 0 {
		let mut lines = vec![format!("no matches for \"{pattern}\" in {} files", result.total_files)];
		if !ignore_case {
			lines.push("hint: wikid grep <pattern> -i — retry case-insensitively".to_string());
		}
		return lines.join("\n");
	}
	let mut lines = Vec::new();
	for hit in &result.matches {
		if files_only {
			lines.push(hit.path.clone());
			continue;
		}
		if let Some(before) = &hit.context_before {
			for (i, text) in before.iter().enumerate() {
				lines.push(format!("{}:{}- {}", hit.path, hit.line - (before.len() - i), text));
			}
		}
		lines.push(format!("{}:{}: {}", hit.path, hit.line, hit.text));
		if let Some(after) = &hit.context_after {
			for (i, text) in after.iter().enumerate() {
				lines.push(format!("{}:{}- {}", hit.path, hit.line + i + 1, text));
			}
		}
	}
	// matched_files counts files with hits; total_files counts files searched.
	let match_word = if result.total_matches == 1 { "match" } else { "matches" };
	let file_word = if result.matched_files == 1 { "file" } else { "files" };
	let totals = format!(
		"total: {} {match_word} in {} {file_word} ({} searched)",
		result.total_matches, result.matched_files, result.total_files
	);
	if result.truncated {
		lines.push(format!(
			"{totals} (showing first {}) — use --limit <n>",
			result.matches.len()
		));
	} else {
		lines.push(totals);
	}
	lines.push("hint: wikid cat <path> — read a match".to_string());
	lines.join("\n")
}

pub fn glob(result: &GlobResult, pattern: &str) -> String {
	if result.total == 0 {
		return [
			format!("no matches for \"{pattern}\""),
			"hint: wikid ls — see what exists at the root".to_string(),
		]
		.join("\n");
	}
	let mut lines: Vec<String> = result.entries.iter().map(entry_line).collect();
	lines.push(format!("total: {}", result.total));
	lines.push("hint: wikid cat <path> — read a page".to_string());
	lines.join("\n")
}

pub fn write(result: &WriteResult) -> String {
	let action = if result.created { "created" } else { "updated" };
	[
		format!("wrote {} ({action}, {})", result.path, human_size(result.bytes)),
		format!("hint: wikid cat {} — verify the content", result.path),
	]
	.join("\n")
}

pub fn edit(result: &EditResult) -> String {
	let plural = if result.replacements == 1 { "" } else { "s" };
	[
		format!(
			"edited {}: {} line{plural} replaced ({})",
			result.path,
			result.replacements,
			human_size(result.bytes)
		),
		format!("hint: wikid cat {} — verify the change", result.path),
	]
	.join("\n")
}

pub fn hashlines(result: &HashlinesResult) -> String {
	let mut lines: Vec<String> = result
		.lines
		.iter()
		.map(|l| format!("{}:{}: {}", l.line, l.hash, l.text))
		.collect();
	if let (Some(start), Some(end)) = (result.range_start, result.range_end) {
		lines.push(format!(
			"lines {start}-{end} of {} ({} bytes total)",
			result.total_lines, result.total_bytes
		));
	} else if result.truncated {
		lines.push(format!(
			"… truncated ({} lines / {} bytes total) — use --full or --lines <START-END>",
			result.total_lines, result.total_bytes
		));
	}
	lines.push(format!(
		"hint: wikid edit {} --line <n> --hash <hash> --new=<text> — replace a line",
		result.path
	));
	lines.push(format!(
		"hint: wikid edit-batch {} — replace multiple hashed lines from JSON stdin",
		result.path
	));
	lines.join("\n")
}

pub fn mv(result: &MvResult) -> String {
	[
		format!("moved {} → {}", result.from, result.to),
		"hint: wikid doctor --checks broken_links — find links broken by the move".to_string(),
	]
	.join("\n")
}

pub fn rm(result: &RmResult) -> String {
	[
		format!("removed {}", result.path),
		"hint: wikid doctor --checks broken_links — find links broken by the removal".to_string(),
	]
	.join("\n")
}

pub fn links(report: &LinkReport) -> String {
	let mut lines = vec![format!("outgoing: {}", report.outgoing.len())];
	for link in &report.outgoing {
		let resolved = link.resolved.as_deref().unwrap_or("(unresolved)");
		let embed = if link.embed { "embed " } else { "" };
		lines.push(format!("  {embed}{} → {resolved}", link.raw));
	}
	lines.push(format!("backlinks: {}", report.backlinks.len()));
	for path in &report.backlinks {
		lines.push(format!("  {path}"));
	}
	lines.push("hint: wikid cat <path> — follow a link".to_string());
	lines.join("\n")
}

pub fn tags(report: &TagReport) -> String {
	let mut lines = Vec::new();
	if report.tags.is_empty() {
		lines.push("no tags found".to_string());
	} else {
		for tag in &report.tags {
			let occurrence_word = if tag.count == 1 { "occurrence" } else { "occurrences" };
			let implied = if tag.implied == Some(true) { " (implied)" } else { "" };
			let preview = tag.pages.iter().take(3).cloned().collect::<Vec<_>>().join(", ");
			let suffix = if tag.pages.len() > 3 {
				format!(", … {} more", tag.pages.len() - 3)
			} else {
				String::new()
			};
			lines.push(format!(
				"#{}{}  {} {occurrence_word}  {preview}{suffix}",
				tag.tag, implied, tag.count
			));
		}
	}
	lines.push("hint: wikid grep '#tag' — inspect a tag's source context".to_string());
	lines.join("\n")
}

pub fn doctor(report: &HealthReport) -> String {
	let mut lines = vec![report.summary.clone()];
	for category in [
		IssueCategory::AuthoredPages,
		IssueCategory::RawSource,
		IssueCategory::AssetHygiene,
		IssueCategory::GraphNavigation,
		IssueCategory::SizePerformance,
	] {
		let issues: Vec<_> = report
			.issues
			.iter()
			.filter(|issue| issue.category == category)
			.collect();
		if issues.is_empty() {
			continue;
		}
		lines.push(String::new());
		lines.push(format!("{} ({})", category_title(category), issues.len()));
		for issue in issues.iter().take(5) {
			lines.push(format!(
				"[{}] {} {} — {}",
				severity_name(issue.severity),
				issue.check.name(),
				issue.path,
				issue.detail
			));
		}
		if issues.len() > 5 {
			lines.push(format!(
				"… {} more omitted; use --json for full detail",
				issues.len() - 5
			));
		}
	}
	lines.push(
		if report.issues.is_empty() {
			"hint: wikid status — vault overview"
		} else {
			"hint: wikid cat <path> — inspect a flagged page; use --profile strict for raw structural lint"
		}
		.to_string(),
	);
	lines.join("\n")
}

fn category_title(category: IssueCategory) -> &'static str {
	match category {
		IssueCategory::AuthoredPages => "actionable authored-page issues",
		IssueCategory::RawSource => "raw-source warnings",
		IssueCategory::AssetHygiene => "asset hygiene",
		IssueCategory::GraphNavigation => "graph/navigation issues",
		IssueCategory::SizePerformance => "size/performance warnings",
	}
}

#[cfg(test)]
mod tests {
	use super::*;

	#[test]
	fn human_size_units() {
		assert_eq!(human_size(0), "0 B");
		assert_eq!(human_size(1023), "1023 B");
		assert_eq!(human_size(1024), "1.0 KiB");
		assert_eq!(human_size(1536), "1.5 KiB");
		assert_eq!(human_size(2 * 1024 * 1024), "2.0 MiB");
	}

	#[test]
	fn entry_lines_are_compact() {
		let dir = Entry {
			path: "notes/".to_string(),
			kind: EntryKind::Dir,
			size: 0,
			modified: "2026-07-02T10:00:00Z".to_string(),
		};
		assert_eq!(entry_line(&dir), "notes/");
		let page = Entry {
			path: "index.md".to_string(),
			kind: EntryKind::Page,
			size: 42,
			modified: "2026-07-02T10:00:00Z".to_string(),
		};
		assert_eq!(entry_line(&page), "index.md  42 B  2026-07-02T10:00:00Z");
	}

	#[test]
	fn document_truncation_marker_matches_the_axi_format() {
		let doc = Document {
			path: "big.md".to_string(),
			content: "line 1\n".to_string(),
			truncated: true,
			range_start: None,
			range_end: None,
			total_lines: 500,
			total_bytes: 4242,
			modified: "2026-07-02T10:00:00Z".to_string(),
		};
		let out = document(&doc);
		assert!(
			out.contains("… truncated (500 lines / 4242 bytes total) — use --full or --lines <START-END>"),
			"{out}"
		);
		assert!(
			out.ends_with("hint: wikid cat big.md --lines 1-120 — read a window"),
			"{out}"
		);
	}

	#[test]
	fn links_marks_embeds_in_human_output() {
		let report = LinkReport {
			outgoing: vec![wikid_core::Link {
				raw: "![[Target]]".to_string(),
				target: "Target".to_string(),
				resolved: Some("target.md".to_string()),
				kind: wikid_core::LinkKind::Wikilink,
				embed: true,
				fragment: None,
			}],
			backlinks: vec![],
		};
		let out = links(&report);
		assert!(out.contains("embed ![[Target]] → target.md"), "{out}");
	}

	#[test]
	fn grep_zero_matches_message_is_explicit() {
		let result = GrepResult {
			matches: vec![],
			total_matches: 0,
			matched_files: 0,
			total_files: 7,
			truncated: false,
		};
		let out = grep(&result, "needle", false, false);
		assert!(out.starts_with("no matches for \"needle\" in 7 files"), "{out}");
	}

	#[test]
	fn grep_zero_matches_omits_case_hint_when_already_case_insensitive() {
		let result = GrepResult {
			matches: vec![],
			total_matches: 0,
			matched_files: 0,
			total_files: 7,
			truncated: false,
		};
		let out = grep(&result, "needle", false, true);
		assert_eq!(out, "no matches for \"needle\" in 7 files");
	}

	#[test]
	fn grep_totals_separate_matched_from_searched_and_pluralize() {
		let hit = wikid_core::GrepMatch {
			path: "a.md".to_string(),
			line: 1,
			text: "x".to_string(),
			context_before: None,
			context_after: None,
		};
		let result = GrepResult {
			matches: vec![hit.clone()],
			total_matches: 1,
			matched_files: 1,
			total_files: 17,
			truncated: false,
		};
		let out = grep(&result, "x", false, false);
		assert!(out.contains("total: 1 match in 1 file (17 searched)"), "{out}");
		let result = GrepResult {
			matches: vec![hit.clone(), hit],
			total_matches: 2,
			matched_files: 2,
			total_files: 17,
			truncated: false,
		};
		let out = grep(&result, "x", false, false);
		assert!(out.contains("total: 2 matches in 2 files (17 searched)"), "{out}");
	}

	#[test]
	fn grep_truncation_totals_carry_the_limit_hint() {
		let result = GrepResult {
			matches: vec![wikid_core::GrepMatch {
				path: "a.md".to_string(),
				line: 1,
				text: "x".to_string(),
				context_before: None,
				context_after: None,
			}],
			total_matches: 9,
			matched_files: 2,
			total_files: 3,
			truncated: true,
		};
		let out = grep(&result, "x", false, false);
		assert!(
			out.contains("total: 9 matches in 2 files (3 searched) (showing first 1) — use --limit <n>"),
			"{out}"
		);
	}

	#[test]
	fn edit_pluralizes_replaced_lines() {
		let one = EditResult {
			path: "a.md".to_string(),
			replacements: 1,
			bytes: 10,
		};
		assert!(edit(&one).contains("1 line replaced ("));
		let many = EditResult {
			path: "a.md".to_string(),
			replacements: 3,
			bytes: 10,
		};
		assert!(edit(&many).contains("3 lines replaced ("));
	}

	#[test]
	fn hashlines_render_line_hash_text_with_truncation_and_hint() {
		let result = HashlinesResult {
			path: "a.md".to_string(),
			lines: vec![wikid_core::Hashline {
				line: 1,
				hash: "abcdef012345".to_string(),
				text: "# Alpha".to_string(),
			}],
			truncated: true,
			range_start: None,
			range_end: None,
			total_lines: 500,
			total_bytes: 4242,
			modified: "2026-07-02T10:00:00Z".to_string(),
		};
		let out = hashlines(&result);
		assert!(out.starts_with("1:abcdef012345: # Alpha"), "{out}");
		assert!(
			out.contains("… truncated (500 lines / 4242 bytes total) — use --full or --lines <START-END>"),
			"{out}"
		);
		assert!(
			out.contains("hint: wikid edit a.md --line <n> --hash <hash> --new=<text> — replace a line"),
			"{out}"
		);
		assert!(
			out.ends_with("hint: wikid edit-batch a.md — replace multiple hashed lines from JSON stdin"),
			"{out}"
		);
	}
}
