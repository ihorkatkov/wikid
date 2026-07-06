//! Obsidian-compatible tag extraction.
//!
//! Inline tags use the Obsidian tag character set (`A-Za-z0-9_-/`, including
//! nested tags via `/`) and frontmatter tags come from the YAML `tags` key. The
//! public report types are shared wire formats for CLI, HTTP, and future MCP
//! surfaces.

use std::collections::{BTreeMap, HashSet};
use std::sync::OnceLock;

use serde::{Deserialize, Serialize};

use crate::error::WikidError;
use crate::frontmatter::Frontmatter;
use crate::markdown::FenceTracker;
use crate::ops::{is_page, read_text};
use crate::vault::Vault;

/// One tag's vault-wide usage.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct TagSummary {
	/// Tag text without a leading `#`, preserving the first authored casing.
	pub tag: String,
	/// Number of authored occurrences carrying this tag, including implied nested-tag parents.
	pub count: usize,
	/// Pages carrying this tag, sorted by vault-relative path.
	pub pages: Vec<String>,
	/// True when this tag exists only as an implied ancestor of nested tags.
	#[serde(default, skip_serializing_if = "Option::is_none")]
	pub implied: Option<bool>,
}

/// Result of listing all tags in a vault.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct TagReport {
	/// Tags sorted case-insensitively by tag text.
	pub tags: Vec<TagSummary>,
}

/// Candidate inline tag matcher. Callers still apply context exclusions
/// (wikilinks/code/headings) and the numeric-only rule.
pub fn tags_re() -> &'static regex::Regex {
	static RE: OnceLock<regex::Regex> = OnceLock::new();
	RE.get_or_init(|| regex::Regex::new(r"#([\p{Alphabetic}\p{Number}_\-/]+)").expect("static regex"))
}

fn wikilink_re() -> &'static regex::Regex {
	static RE: OnceLock<regex::Regex> = OnceLock::new();
	RE.get_or_init(|| regex::Regex::new(r"!?\[\[[^\[\]\n]+\]\]").expect("static regex"))
}

/// Extracts inline `#tag` occurrences from Markdown body text.
///
/// Excludes wikilink fragments, ATX headings, fenced code blocks, inline code
/// spans, and numeric-only tags.
pub fn extract_tags(content: &str) -> Vec<String> {
	let body_offset = body_start(content);
	let body = &content[body_offset..];
	let wikilinks: Vec<(usize, usize)> = wikilink_re()
		.find_iter(body)
		.map(|m| (body_offset + m.start(), body_offset + m.end()))
		.collect();
	let mut tags = Vec::new();
	let mut fences = FenceTracker::new();
	let mut offset = body_offset;
	for line in body.split_inclusive('\n') {
		if fences.observe(line) || fences.in_fence() {
			offset += line.len();
			continue;
		}
		let code_spans = inline_code_spans(line);
		for caps in tags_re().captures_iter(line) {
			let whole = caps.get(0).expect("capture 0");
			let tag = caps.get(1).expect("capture 1").as_str();
			let line_pos = whole.start();
			let absolute = offset + line_pos;
			if in_ranges(absolute, &wikilinks)
				|| in_ranges(line_pos, &code_spans)
				|| preceded_by_word(line, line_pos)
				|| escaped_hash(line, line_pos)
			{
				continue;
			}
			if !tag.chars().all(char::is_numeric) {
				tags.push(tag.to_string());
			}
		}
		offset += line.len();
	}
	tags
}

/// Frontmatter `tags`: a single string or sequence of strings. Leading `#` is
/// stripped; empty and non-string entries are ignored.
pub fn frontmatter_tags(frontmatter: &Frontmatter) -> Vec<String> {
	let Frontmatter::Present(map) = frontmatter else {
		return Vec::new();
	};
	match map.get("tags") {
		Some(serde_yaml::Value::String(tag)) => normalize_tag(tag).into_iter().collect(),
		Some(serde_yaml::Value::Sequence(values)) => values
			.iter()
			.filter_map(|value| match value {
				serde_yaml::Value::String(tag) => normalize_tag(tag),
				_ => None,
			})
			.collect(),
		_ => Vec::new(),
	}
}

/// Merges frontmatter and inline tags for one page, deduping
/// case-insensitively while preserving first-authored case and order.
pub fn page_tags(frontmatter: &Frontmatter, content: &str) -> Vec<String> {
	let mut seen = HashSet::new();
	let mut merged = Vec::new();
	for tag in frontmatter_tags(frontmatter).into_iter().chain(extract_tags(content)) {
		let key = tag.to_lowercase();
		if seen.insert(key) {
			merged.push(tag);
		}
	}
	merged
}

impl Vault {
	/// Lists tags across all visible Markdown pages in the vault.
	pub fn tags(&self) -> Result<TagReport, WikidError> {
		let mut by_key: BTreeMap<String, TagSummary> = BTreeMap::new();
		let mut literal_keys = HashSet::new();
		for (rel, abs) in self.visible_files()? {
			if !is_page(&rel) {
				continue;
			}
			let Some(text) = read_text(&abs)? else { continue };
			let frontmatter = crate::frontmatter::parse(&text);
			for tag in frontmatter_tags(&frontmatter).into_iter().chain(extract_tags(&text)) {
				literal_keys.insert(tag.to_lowercase());
				for expanded in tag_ancestors(&tag) {
					let key = expanded.to_lowercase();
					let entry = by_key.entry(key).or_insert_with(|| TagSummary {
						tag: expanded,
						count: 0,
						pages: Vec::new(),
						implied: None,
					});
					entry.count += 1;
					if !entry.pages.contains(&rel) {
						entry.pages.push(rel.clone());
					}
				}
			}
		}
		for (key, summary) in &mut by_key {
			if !literal_keys.contains(key) {
				summary.implied = Some(true);
			}
		}
		Ok(TagReport {
			tags: by_key.into_values().collect(),
		})
	}
}

fn normalize_tag(tag: &str) -> Option<String> {
	let tag = tag.trim().trim_start_matches('#').trim();
	(!tag.is_empty()).then(|| tag.to_string())
}

fn tag_ancestors(tag: &str) -> Vec<String> {
	if tag.split('/').any(str::is_empty) {
		return vec![tag.to_string()];
	}
	let parts: Vec<&str> = tag.split('/').collect();
	(1..=parts.len()).map(|end| parts[..end].join("/")).collect()
}

fn body_start(content: &str) -> usize {
	let mut offset = 0usize;
	let mut opened = false;
	for line in content.split_inclusive('\n') {
		let line_start = offset;
		offset += line.len();
		if !opened {
			if line_start == 0 && line.trim_end() == "---" {
				opened = true;
				continue;
			}
			return 0;
		}
		if line.trim_end() == "---" {
			return offset;
		}
	}
	0
}

pub(crate) fn inline_code_spans(line: &str) -> Vec<(usize, usize)> {
	let bytes = line.as_bytes();
	let mut ranges = Vec::new();
	let mut i = 0;
	while i < bytes.len() {
		if bytes[i] != b'`' {
			i += 1;
			continue;
		}
		let start = i;
		let mut ticks = 1;
		while i + ticks < bytes.len() && bytes[i + ticks] == b'`' {
			ticks += 1;
		}
		i += ticks;
		let mut end = None;
		while i < bytes.len() {
			if bytes[i] == b'`'
				&& bytes
					.get(i..i + ticks)
					.is_some_and(|run| run.iter().all(|b| *b == b'`'))
			{
				end = Some(i + ticks);
				break;
			}
			i += 1;
		}
		let span_end = end.unwrap_or(bytes.len());
		ranges.push((start, span_end));
		i = span_end;
	}
	ranges
}

fn in_ranges(pos: usize, ranges: &[(usize, usize)]) -> bool {
	ranges.iter().any(|(start, end)| (*start..*end).contains(&pos))
}

fn preceded_by_word(line: &str, hash: usize) -> bool {
	line[..hash]
		.chars()
		.next_back()
		.is_some_and(|ch| ch.is_alphanumeric() || ch == '_')
}

fn escaped_hash(line: &str, hash: usize) -> bool {
	line[..hash].ends_with('\\')
}

#[cfg(test)]
mod tests {
	use super::*;
	use crate::frontmatter;

	#[test]
	fn extracts_inline_tags_and_nested_tags() {
		assert_eq!(
			extract_tags("Body with #project and #area/work."),
			vec!["project", "area/work"]
		);
	}

	#[test]
	fn excludes_wikilink_fragments() {
		assert!(extract_tags("See [[note#heading]] and ![[other#frag]].").is_empty());
	}

	#[test]
	fn excludes_headings_code_and_numeric_only_tags() {
		let content = "# Heading\n## Sub\ntext #ok and #123\n```\n#notatag\n```\n`#inline` #real\n";
		assert_eq!(extract_tags(content), vec!["ok", "real"]);
	}

	#[test]
	fn mixed_fence_delimiters_do_not_flip_tag_scanning() {
		let content = "```\n~~~\n#ignored\n```\n#real\n";
		assert_eq!(extract_tags(content), vec!["real"]);
	}

	#[test]
	fn tag_must_not_be_preceded_by_word_char() {
		assert_eq!(extract_tags("word#no under_score#no (#yes)"), vec!["yes"]);
	}

	#[test]
	fn escaped_hash_is_not_a_tag() {
		assert_eq!(extract_tags(r"escaped \#foo but #bar"), vec!["bar"]);
	}

	#[test]
	fn reads_frontmatter_list_and_single_string() {
		let fm = frontmatter::parse("---\ntags: [alpha, '#Beta']\n---\n");
		assert_eq!(frontmatter_tags(&fm), vec!["alpha", "Beta"]);
		let fm = frontmatter::parse("---\ntags: '#alpha'\n---\n");
		assert_eq!(frontmatter_tags(&fm), vec!["alpha"]);
	}

	#[test]
	fn merged_page_tags_dedupe_case_insensitively() {
		let content = "---\ntags: [alpha, Beta]\n---\n\nBody #alpha #beta #Gamma\n";
		let fm = frontmatter::parse(content);
		assert_eq!(page_tags(&fm, content), vec!["alpha", "Beta", "Gamma"]);
	}

	#[test]
	fn vault_report_counts_occurrences_and_keeps_page_unions() {
		let dir = tempfile::tempdir().unwrap();
		std::fs::write(dir.path().join("a.md"), "---\ntags: [Alpha]\n---\n\n#beta\n").unwrap();
		std::fs::write(dir.path().join("b.md"), "#alpha #alpha #gamma\n").unwrap();
		let report = Vault::open(dir.path()).unwrap().tags().unwrap();
		assert_eq!(report.tags[0].tag, "Alpha");
		assert_eq!(report.tags[0].count, 3);
		assert_eq!(report.tags[0].pages, vec!["a.md", "b.md"]);
		assert_eq!(report.tags[1].tag, "beta");
		assert_eq!(report.tags[1].pages, vec!["a.md"]);
	}

	#[test]
	fn nested_tags_mark_implied_ancestors_only_when_never_literal() {
		let dir = tempfile::tempdir().unwrap();
		std::fs::write(dir.path().join("A.md"), "#project/wikid #area/research\n").unwrap();
		std::fs::write(dir.path().join("B.md"), "#project\n").unwrap();
		let report = Vault::open(dir.path()).unwrap().tags().unwrap();

		let project = report.tags.iter().find(|summary| summary.tag == "project").unwrap();
		assert_eq!(project.implied, None);
		let area = report.tags.iter().find(|summary| summary.tag == "area").unwrap();
		assert_eq!(area.implied, Some(true));
		let area_research = report
			.tags
			.iter()
			.find(|summary| summary.tag == "area/research")
			.unwrap();
		assert_eq!(area_research.implied, None);
	}

	#[test]
	fn nested_tags_imply_parent_tags_with_aggregate_counts() {
		let dir = tempfile::tempdir().unwrap();
		std::fs::write(
			dir.path().join("Home.md"),
			"---\ntags: [home]\n---\n\n#project/wikid #work #callout-tag\n",
		)
		.unwrap();
		std::fs::write(dir.path().join("Beta.md"), "---\ntags: work\n---\n\n#work\n").unwrap();
		let report = Vault::open(dir.path()).unwrap().tags().unwrap();
		let counts: BTreeMap<&str, usize> = report
			.tags
			.iter()
			.map(|summary| (summary.tag.as_str(), summary.count))
			.collect();
		assert_eq!(counts["callout-tag"], 1);
		assert_eq!(counts["home"], 1);
		assert_eq!(counts["project"], 1);
		assert_eq!(counts["project/wikid"], 1);
		assert_eq!(counts["work"], 3);
		let project = report.tags.iter().find(|summary| summary.tag == "project").unwrap();
		assert_eq!(project.pages, vec!["Home.md"]);
		let work = report.tags.iter().find(|summary| summary.tag == "work").unwrap();
		assert_eq!(work.pages, vec!["Beta.md", "Home.md"]);
	}
}
