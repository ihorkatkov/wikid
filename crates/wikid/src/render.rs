//! Human-readable renderers (DESIGN §6): compact text, pre-computed totals,
//! explicit zero-result messages, and trailing `hint:` lines. `--json` mode
//! bypasses this module entirely — it serializes the core structs directly.
//!
//! Every function returns the full output without a trailing newline; the
//! caller prints it with `println!`.

use wikid_core::{
	Document, EditResult, Entry, EntryKind, GlobResult, GrepResult, HealthReport, LinkReport, Listing, MvResult,
	RmResult, Severity, VaultStatus, WriteResult,
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

pub fn status(status: &VaultStatus) -> String {
	let mut lines = vec![
		format!("vault: {}", status.root),
		format!(
			"pages: {}  files: {}  size: {}",
			status.total_pages,
			status.total_files,
			human_size(status.total_bytes)
		),
	];
	if !status.recent.is_empty() {
		lines.push("recent:".to_string());
		for page in &status.recent {
			lines.push(format!("  {}  {}", page.path, page.modified));
		}
	}
	let health = &status.doctor_summary;
	lines.push(format!(
		"health: {} high, {} medium, {} low",
		health.high, health.medium, health.low
	));
	lines.push("hint: wikid ls <path> — list a directory".to_string());
	lines.push("hint: wikid doctor — full health report".to_string());
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
	if doc.truncated {
		lines.push(format!(
			"… truncated ({} lines / {} bytes total) — use --full",
			doc.total_lines, doc.total_bytes
		));
		lines.push(format!("hint: wikid cat {} --full — read the whole file", doc.path));
	} else {
		lines.push(format!("hint: wikid links {} — outgoing links and backlinks", doc.path));
	}
	lines.join("\n")
}

pub fn grep(result: &GrepResult, pattern: &str, files_only: bool) -> String {
	if result.total_matches == 0 {
		return [
			format!("no matches for \"{pattern}\" in {} files", result.total_files),
			"hint: wikid grep <pattern> -i — retry case-insensitively".to_string(),
		]
		.join("\n");
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
	let totals = format!(
		"total: {} matches in {} files",
		result.total_matches, result.total_files
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
			"edited {}: {} replacement{plural} ({})",
			result.path,
			result.replacements,
			human_size(result.bytes)
		),
		format!("hint: wikid cat {} — verify the change", result.path),
	]
	.join("\n")
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
		lines.push(format!("  {} → {resolved}", link.raw));
	}
	lines.push(format!("backlinks: {}", report.backlinks.len()));
	for path in &report.backlinks {
		lines.push(format!("  {path}"));
	}
	lines.push("hint: wikid cat <path> — follow a link".to_string());
	lines.join("\n")
}

pub fn doctor(report: &HealthReport) -> String {
	let mut lines = vec![report.summary.clone()];
	for issue in &report.issues {
		lines.push(format!(
			"[{}] {} {} — {}",
			severity_name(issue.severity),
			issue.check.name(),
			issue.path,
			issue.detail
		));
	}
	lines.push(
		if report.issues.is_empty() {
			"hint: wikid status — vault overview"
		} else {
			"hint: wikid cat <path> — inspect a flagged page"
		}
		.to_string(),
	);
	lines.join("\n")
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
			total_lines: 500,
			total_bytes: 4242,
			modified: "2026-07-02T10:00:00Z".to_string(),
		};
		let out = document(&doc);
		assert!(
			out.contains("… truncated (500 lines / 4242 bytes total) — use --full"),
			"{out}"
		);
		assert!(
			out.ends_with("hint: wikid cat big.md --full — read the whole file"),
			"{out}"
		);
	}

	#[test]
	fn grep_zero_matches_message_is_explicit() {
		let result = GrepResult {
			matches: vec![],
			total_matches: 0,
			total_files: 7,
			truncated: false,
		};
		let out = grep(&result, "needle", false);
		assert!(out.starts_with("no matches for \"needle\" in 7 files"), "{out}");
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
			total_files: 3,
			truncated: true,
		};
		let out = grep(&result, "x", false);
		assert!(
			out.contains("total: 9 matches in 3 files (showing first 1) — use --limit <n>"),
			"{out}"
		);
	}

	#[test]
	fn edit_pluralizes_replacements() {
		let one = EditResult {
			path: "a.md".to_string(),
			replacements: 1,
			bytes: 10,
		};
		assert!(edit(&one).contains("1 replacement ("));
		let many = EditResult {
			path: "a.md".to_string(),
			replacements: 3,
			bytes: 10,
		};
		assert!(edit(&many).contains("3 replacements ("));
	}
}
