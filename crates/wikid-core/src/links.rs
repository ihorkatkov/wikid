//! The Obsidian-compatible link model (DESIGN §4): extraction of
//! `[[Target]]`, `[[Target|alias]]`, `[[Target#Heading]]`, and markdown
//! links, plus resolution against the vault's visible files.

use std::collections::BTreeMap;
use std::sync::OnceLock;

use serde::{Deserialize, Serialize};

use crate::error::WikidError;
use crate::frontmatter;
use crate::obsidian_config::ObsidianConfig;
use crate::ops::{is_page, read_text};
use crate::paths;
use crate::vault::Vault;

/// How a link was written in the source page.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum LinkKind {
	/// An Obsidian `[[wikilink]]`.
	Wikilink,
	/// A standard markdown `[text](path)`-style link.
	Markdown,
}

/// One outgoing link found in a page.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Link {
	/// The link exactly as written, e.g. `[[Note|alias]]`.
	pub raw: String,
	/// The resolution target after stripping alias and fragment parts.
	pub target: String,
	/// Vault-relative path the link resolves to; `None` when broken or ambiguous.
	pub resolved: Option<String>,
	/// Wikilink or markdown.
	pub kind: LinkKind,
	/// Whether the link was written as an embed/transclusion (`![[…]]` or `![…](…)`).
	#[serde(default, skip_serializing_if = "is_false")]
	pub embed: bool,
	/// Fragment after the first unescaped `#`, e.g. `Heading` or `^block-id`.
	#[serde(default, skip_serializing_if = "Option::is_none")]
	pub fragment: Option<String>,
}

fn is_false(value: &bool) -> bool {
	!*value
}

/// Result of `links`: a page's outgoing links and the pages linking back to it.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct LinkReport {
	/// Links found in the page, in document order.
	pub outgoing: Vec<Link>,
	/// Pages containing at least one link resolving to this path, sorted.
	pub backlinks: Vec<String>,
}

/// A link as written, before resolution.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct ExtractedLink {
	/// The exact matched text.
	pub raw: String,
	/// The target with alias and fragment parts stripped.
	pub target: String,
	/// Wikilink or markdown.
	pub kind: LinkKind,
	/// Whether the link was written as an embed/transclusion.
	pub embed: bool,
	/// Fragment after the first unescaped `#`, e.g. `Heading` or `^block-id`.
	pub fragment: Option<String>,
}

/// Outcome of resolving a single link target.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum Resolution {
	/// Exactly one file matched.
	Resolved(String),
	/// Multiple candidates matched; resolution refuses to guess.
	Ambiguous(Vec<String>),
	/// Nothing matched.
	Broken,
}

fn wikilink_re() -> &'static regex::Regex {
	static RE: OnceLock<regex::Regex> = OnceLock::new();
	RE.get_or_init(|| regex::Regex::new(r"!?\[\[([^\[\]\n]+)\]\]").expect("static regex"))
}

fn markdown_re() -> &'static regex::Regex {
	static RE: OnceLock<regex::Regex> = OnceLock::new();
	RE.get_or_init(|| regex::Regex::new(r"!?\[[^\]\n]*\]\(([^()\n]*)\)").expect("static regex"))
}

/// Extracts every wikilink and markdown link from `content`, in document
/// order. External targets (`http://`, `https://`, `mailto:`) and pure
/// anchors (`#heading`, `[[#heading]]`) are skipped.
pub(crate) fn extract_links(content: &str) -> Vec<ExtractedLink> {
	let mut found: Vec<(usize, ExtractedLink)> = Vec::new();
	for caps in wikilink_re().captures_iter(content) {
		let whole = caps.get(0).expect("capture 0");
		// Alias splits first ([[target#heading|alias]]), then the fragment.
		let inner = &caps[1];
		let target = inner.split('|').next().unwrap_or(inner);
		let (target, fragment) = split_fragment(target);
		let target = target.trim();
		if target.is_empty() {
			continue;
		}
		found.push((
			whole.start(),
			ExtractedLink {
				raw: whole.as_str().to_string(),
				target: target.to_string(),
				kind: LinkKind::Wikilink,
				embed: whole.as_str().starts_with('!'),
				fragment,
			},
		));
	}
	for caps in markdown_re().captures_iter(content) {
		let whole = caps.get(0).expect("capture 0");
		// CommonMark allows a quoted title after the destination:
		// `[text](dest "title")` — the title is not part of the target.
		let inner = strip_markdown_title(caps[1].trim());
		let inner = inner
			.strip_prefix('<')
			.and_then(|s| s.strip_suffix('>'))
			.unwrap_or(inner);
		if inner.starts_with("http://")
			|| inner.starts_with("https://")
			|| inner.starts_with("mailto:")
			|| inner.starts_with('/')
		{
			continue;
		}
		let (target, fragment) = split_fragment(inner);
		let target = target.trim();
		if target.is_empty() || target.starts_with('/') {
			continue;
		}
		found.push((
			whole.start(),
			ExtractedLink {
				raw: whole.as_str().to_string(),
				// Markdown destinations are URLs: `My%20Note.md` on disk is
				// `My Note.md`. Wikilink targets stay literal.
				target: percent_decode(target),
				kind: LinkKind::Markdown,
				embed: whole.as_str().starts_with('!'),
				fragment,
			},
		));
	}
	found.sort_by_key(|(start, _)| *start);
	found.into_iter().map(|(_, link)| link).collect()
}

/// Strips a trailing CommonMark link title — `"…"` or `'…'` preceded by
/// whitespace after the destination, e.g. `notes/guide.md "The Guide"` →
/// `notes/guide.md`. A quoted string with nothing before it is a destination,
/// not a title, and is left alone.
fn split_fragment(target: &str) -> (&str, Option<String>) {
	let Some(hash) = find_unescaped_hash(target) else {
		return (target, None);
	};
	(&target[..hash], Some(target[hash + 1..].to_string()))
}

fn find_unescaped_hash(target: &str) -> Option<usize> {
	let mut escaped = false;
	for (i, ch) in target.char_indices() {
		if escaped {
			escaped = false;
			continue;
		}
		match ch {
			'\\' => escaped = true,
			'#' => return Some(i),
			_ => {}
		}
	}
	None
}

fn strip_markdown_title(inner: &str) -> &str {
	for quote in ['"', '\''] {
		let Some(without_close) = inner.strip_suffix(quote) else {
			continue;
		};
		let Some(open) = without_close.rfind(quote) else {
			continue;
		};
		let before = &without_close[..open];
		if before.ends_with(char::is_whitespace) && !before.trim_end().is_empty() {
			return before.trim_end();
		}
	}
	inner
}

/// Decodes RFC 3986 percent-escapes (`%20` → space) in a markdown link
/// destination. Malformed escapes pass through literally; a decode that is
/// not valid UTF-8 leaves the target unchanged.
fn percent_decode(target: &str) -> String {
	let bytes = target.as_bytes();
	let mut out: Vec<u8> = Vec::with_capacity(bytes.len());
	let mut i = 0;
	while i < bytes.len() {
		let escape = (bytes[i] == b'%' && i + 2 < bytes.len())
			.then(|| Some((hex_val(bytes[i + 1])?, hex_val(bytes[i + 2])?)))
			.flatten();
		match escape {
			Some((hi, lo)) => {
				out.push(hi * 16 + lo);
				i += 3;
			}
			None => {
				out.push(bytes[i]);
				i += 1;
			}
		}
	}
	String::from_utf8(out).unwrap_or_else(|_| target.to_string())
}

fn hex_val(byte: u8) -> Option<u8> {
	(byte as char).to_digit(16).map(|v| v as u8)
}

/// Returns Obsidian block anchors declared as trailing `^block-id` tokens.
///
/// The supported grammar is intentionally pragmatic: `^` followed by one or
/// more ASCII letters, digits, or `-`, at the end of a line after optional
/// whitespace. Anchors inside fenced code blocks are ignored.
pub(crate) fn block_anchors(content: &str) -> Vec<String> {
	let mut anchors = Vec::new();
	let mut in_code_fence = false;
	for line in content.lines() {
		let trimmed_start = line.trim_start();
		if trimmed_start.starts_with("```") || trimmed_start.starts_with("~~~") {
			in_code_fence = !in_code_fence;
			continue;
		}
		if in_code_fence {
			continue;
		}
		let trimmed_end = line.trim_end();
		let Some(caret) = trimmed_end.rfind('^') else {
			continue;
		};
		let candidate = &trimmed_end[caret + 1..];
		if !candidate.is_empty() && candidate.bytes().all(|b| b.is_ascii_alphanumeric() || b == b'-') {
			anchors.push(candidate.to_string());
		}
	}
	anchors
}

/// The resolution index: every visible file in the vault, addressable by
/// exact path, stem or file name, or path suffix (DESIGN §4).
pub(crate) struct LinkIndex {
	/// All visible vault-relative file paths.
	files: Vec<String>,
	/// Lowercased stem and full file name → indices into `files`.
	by_name: BTreeMap<String, Vec<usize>>,
	/// Lowercased frontmatter alias → indices into `files`.
	by_alias: BTreeMap<String, Vec<usize>>,
	/// Optional Obsidian attachment folder used to prefer configured assets.
	attachment_folder: Option<String>,
}

impl LinkIndex {
	/// Builds the index from the vault-relative paths of all visible files and
	/// per-file frontmatter aliases.
	pub(crate) fn build(
		files: Vec<String>,
		aliases: &[(usize, Vec<String>)],
		attachment_folder: Option<String>,
	) -> Self {
		let mut by_name: BTreeMap<String, Vec<usize>> = BTreeMap::new();
		for (i, rel) in files.iter().enumerate() {
			let name = rel.rsplit('/').next().unwrap_or(rel).to_lowercase();
			let stem = name.rsplit_once('.').map(|(s, _)| s.to_string()).unwrap_or_default();
			by_name.entry(name).or_default().push(i);
			if !stem.is_empty() {
				by_name.entry(stem).or_default().push(i);
			}
		}
		let mut by_alias: BTreeMap<String, Vec<usize>> = BTreeMap::new();
		for (i, page_aliases) in aliases {
			for alias in page_aliases {
				by_alias.entry(alias.to_lowercase()).or_default().push(*i);
			}
		}
		Self {
			files,
			by_name,
			by_alias,
			attachment_folder,
		}
	}

	/// Resolution order (DESIGN §4): (1) exact relative path from the root,
	/// with or without `.md`; (2) for bare targets, a unique case-insensitive
	/// stem or file-name match anywhere in the vault; (3) a unique
	/// case-insensitive path-suffix match (`folder/Note`); (4) for bare targets,
	/// a unique case-insensitive frontmatter alias. Multiple candidates at a
	/// stage yield `Ambiguous`; no candidates yield `Broken`.
	pub(crate) fn resolve(&self, target: &str) -> Resolution {
		let Ok(components) = paths::normalize(target) else {
			return Resolution::Broken;
		};
		let rel = components.join("/");
		if self.files.contains(&rel) {
			return Resolution::Resolved(rel);
		}
		let with_md = format!("{rel}.md");
		if self.files.contains(&with_md) {
			return Resolution::Resolved(with_md);
		}
		if let Some(configured_attachment) = self.configured_attachment_candidate(&rel) {
			return Resolution::Resolved(configured_attachment);
		}
		if !rel.contains('/') {
			let candidates: Vec<String> = match self.by_name.get(&rel.to_lowercase()) {
				Some(indices) => indices.iter().map(|&i| self.files[i].clone()).collect(),
				None => Vec::new(),
			};
			if !candidates.is_empty() {
				return decide(candidates);
			}
			return decide(self.alias_candidates(&rel));
		}
		let rel_lower = rel.to_lowercase();
		let suffixes = [rel_lower.clone(), format!("{rel_lower}.md")];
		let candidates: Vec<String> = self
			.files
			.iter()
			.filter(|f| {
				let f = f.to_lowercase();
				suffixes.iter().any(|s| f == *s || f.ends_with(&format!("/{s}")))
			})
			.cloned()
			.collect();
		if !candidates.is_empty() {
			return decide(candidates);
		}
		decide(self.alias_candidates(&rel))
	}

	fn alias_candidates(&self, rel: &str) -> Vec<String> {
		self.by_alias
			.get(&rel.to_lowercase())
			.map(|indices| indices.iter().map(|&i| self.files[i].clone()).collect())
			.unwrap_or_default()
	}

	fn configured_attachment_candidate(&self, rel: &str) -> Option<String> {
		let folder = self.attachment_folder.as_deref()?;
		if rel == folder || rel.starts_with(&format!("{folder}/")) {
			return None;
		}
		let candidate = format!("{folder}/{rel}");
		self.files.contains(&candidate).then_some(candidate)
	}
}

/// Maps a candidate set to a resolution: none = broken, one = resolved,
/// several = ambiguous (sorted for deterministic reporting).
fn decide(mut candidates: Vec<String>) -> Resolution {
	candidates.sort();
	candidates.dedup();
	match candidates.len() {
		0 => Resolution::Broken,
		1 => Resolution::Resolved(candidates.remove(0)),
		_ => Resolution::Ambiguous(candidates),
	}
}

impl Vault {
	/// Outgoing links and backlinks for a file (DESIGN §3). Outgoing links
	/// are extracted from pages only — attachments carry none but can still
	/// have backlinks. Backlinks scan every page for links resolving to `path`.
	pub fn links(&self, path: &str) -> Result<LinkReport, WikidError> {
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
		let files = self.visible_files()?;
		let config = ObsidianConfig::load(self.root());
		let mut page_links = Vec::new();
		let mut aliases = Vec::new();
		for (i, (rel, abs)) in files.iter().enumerate() {
			if !is_page(rel) {
				continue;
			}
			// IO failures must surface (a silently missing page would mean
			// silently wrong outgoing links or backlinks); binary/non-UTF-8
			// pages are deliberately skipped.
			let Some(text) = read_text(abs)? else { continue };
			let fm = frontmatter::parse(&text);
			let page_aliases = frontmatter::aliases(&fm);
			if !page_aliases.is_empty() {
				aliases.push((i, page_aliases));
			}
			page_links.push((rel.clone(), extract_links(&text)));
		}
		let index = LinkIndex::build(
			files.iter().map(|(rel, _)| rel.clone()).collect(),
			&aliases,
			config.attachment_folder,
		);
		let mut outgoing = Vec::new();
		let mut backlinks = Vec::new();
		for (rel, extracted) in page_links {
			if rel == target.rel {
				outgoing = extracted
					.into_iter()
					.map(|link| {
						let resolved = match index.resolve(&link.target) {
							Resolution::Resolved(p) => Some(p),
							_ => None,
						};
						Link {
							raw: link.raw,
							target: link.target,
							resolved,
							kind: link.kind,
							embed: link.embed,
							fragment: link.fragment,
						}
					})
					.collect();
			} else if extracted
				.iter()
				.any(|l| matches!(index.resolve(&l.target), Resolution::Resolved(p) if p == target.rel))
			{
				backlinks.push(rel);
			}
		}
		backlinks.sort();
		Ok(LinkReport { outgoing, backlinks })
	}
}

#[cfg(test)]
mod tests {
	use super::*;
	use crate::test_fixtures;

	// --- extraction ---

	#[test]
	fn extracts_wikilink_variants() {
		let links = extract_links("See [[Alpha]], [[beta|The Beta]], [[guide#Setup]], and [[a#H|label]].\n");
		let targets: Vec<(&str, &str)> = links.iter().map(|l| (l.raw.as_str(), l.target.as_str())).collect();
		assert_eq!(
			targets,
			vec![
				("[[Alpha]]", "Alpha"),
				("[[beta|The Beta]]", "beta"),
				("[[guide#Setup]]", "guide"),
				("[[a#H|label]]", "a"),
			]
		);
		assert!(links.iter().all(|l| l.kind == LinkKind::Wikilink));
	}

	#[test]
	fn extracts_markdown_links_and_skips_external_and_anchors() {
		let links = extract_links(
			"[guide](notes/guide.md) ![img](img/logo.png) [w](https://e.com) \
			 [h](http://e.com) [m](mailto:a@b.c) [root](/docs/page) [a](#top) [[#heading]] [b](<my file.md>)\n",
		);
		let targets: Vec<(&str, &str)> = links.iter().map(|l| (l.raw.as_str(), l.target.as_str())).collect();
		assert_eq!(
			targets,
			vec![
				("[guide](notes/guide.md)", "notes/guide.md"),
				("![img](img/logo.png)", "img/logo.png"),
				("[b](<my file.md>)", "my file.md"),
			]
		);
		assert!(links.iter().all(|l| l.kind == LinkKind::Markdown));
	}

	#[test]
	fn extraction_preserves_document_order_across_kinds() {
		let links = extract_links("[md first](a.md) then [[wiki]] then [md again](b.md)\n");
		let raws: Vec<&str> = links.iter().map(|l| l.raw.as_str()).collect();
		assert_eq!(raws, vec!["[md first](a.md)", "[[wiki]]", "[md again](b.md)"]);
	}

	#[test]
	fn extracts_embeds_with_bang_in_raw() {
		let links = extract_links("Embedded: ![[logo.png]]\n");
		assert_eq!(links.len(), 1);
		assert_eq!(links[0].raw, "![[logo.png]]");
		assert_eq!(links[0].target, "logo.png");
		assert_eq!(links[0].kind, LinkKind::Wikilink);
		assert!(links[0].embed);
		assert_eq!(links[0].fragment, None);
	}

	#[test]
	fn plain_wikilinks_are_not_embeds() {
		let links = extract_links("Plain: [[Note]]\n");
		assert_eq!(links.len(), 1);
		assert!(!links[0].embed);
		assert_eq!(links[0].kind, LinkKind::Wikilink);
	}

	#[test]
	fn wikilink_embeds_capture_fragments_without_changing_target() {
		let links = extract_links("Embedded section: ![[Note#Section]]\n");
		assert_eq!(links.len(), 1);
		assert!(links[0].embed);
		assert_eq!(links[0].target, "Note");
		assert_eq!(links[0].fragment.as_deref(), Some("Section"));
	}

	#[test]
	fn wikilink_block_references_capture_fragment() {
		let links = extract_links("See [[note#^abc123]]\n");
		assert_eq!(links.len(), 1);
		assert_eq!(links[0].target, "note");
		assert_eq!(links[0].fragment.as_deref(), Some("^abc123"));
		assert!(!links[0].embed);
	}

	#[test]
	fn wikilink_heading_fragment_and_alias_split_correctly() {
		let links = extract_links("See [[note#Heading|Alias]]\n");
		assert_eq!(links.len(), 1);
		assert_eq!(links[0].raw, "[[note#Heading|Alias]]");
		assert_eq!(links[0].target, "note");
		assert_eq!(links[0].fragment.as_deref(), Some("Heading"));
		assert!(!links[0].embed);
	}

	#[test]
	fn block_anchors_scan_trailing_tokens_outside_code_fences() {
		let anchors = block_anchors(
			"Some text ^abc123\nnot an ^anchor in middle of line\n- bullet ^with-dash\n```\ncode ^ignored\n```\n~~~\nmore ^ignored\n~~~\n",
		);
		assert_eq!(anchors, vec!["abc123", "with-dash"]);
	}

	#[test]
	fn markdown_embeds_are_marked_as_embeds() {
		let links = extract_links("![alt](img.png)\n");
		assert_eq!(links.len(), 1);
		assert_eq!(links[0].target, "img.png");
		assert_eq!(links[0].kind, LinkKind::Markdown);
		assert!(links[0].embed);
	}

	#[test]
	fn callout_markers_are_not_markdown_links() {
		let links = extract_links("[!note] in normal text\n> [!warning]- Title\n");
		assert!(links.is_empty());
	}

	#[test]
	fn links_inside_callouts_are_still_extracted() {
		let links = extract_links("> [!note] Related\n> See [[Link]] for details\n");
		assert_eq!(links.len(), 1);
		assert_eq!(links[0].raw, "[[Link]]");
		assert_eq!(links[0].target, "Link");
		assert_eq!(links[0].kind, LinkKind::Wikilink);
	}

	#[test]
	fn markdown_fragment_is_stripped_for_resolution_and_captured() {
		let links = extract_links("[s](notes/guide.md#setup)\n");
		assert_eq!(links[0].target, "notes/guide.md");
		assert_eq!(links[0].fragment.as_deref(), Some("setup"));
	}

	#[test]
	fn markdown_titles_are_stripped_for_resolution() {
		let links = extract_links(
			"[a](notes/guide.md \"The Guide\") [b](notes/guide.md 'Single') \
			 [c](notes/guide.md#setup \"Titled\") [d](<my file.md> \"T\") [e](\"quoted name.md\")\n",
		);
		let targets: Vec<&str> = links.iter().map(|l| l.target.as_str()).collect();
		assert_eq!(
			targets,
			vec![
				"notes/guide.md",
				"notes/guide.md",
				"notes/guide.md",
				"my file.md",
				// A bare quoted string is a destination, not a title.
				"\"quoted name.md\"",
			]
		);
		// The raw text keeps the title as written.
		assert_eq!(links[0].raw, "[a](notes/guide.md \"The Guide\")");
	}

	#[test]
	fn markdown_targets_are_percent_decoded_but_wikilinks_stay_literal() {
		let links = extract_links("[n](My%20Note.md) [bad](odd%2Zname.md) [pct](50%25.md) [[My%20Note]]\n");
		let targets: Vec<(&str, LinkKind)> = links.iter().map(|l| (l.target.as_str(), l.kind)).collect();
		assert_eq!(
			targets,
			vec![
				("My Note.md", LinkKind::Markdown),
				// Malformed escapes pass through literally.
				("odd%2Zname.md", LinkKind::Markdown),
				("50%.md", LinkKind::Markdown),
				("My%20Note", LinkKind::Wikilink),
			]
		);
	}

	#[test]
	fn percent_encoded_markdown_link_resolves_to_the_file_on_disk() {
		let (_dir, vault) = test_fixtures::vault();
		vault.write("My Note.md", "# My Note\n").unwrap();
		vault.write("linker.md", "[n](My%20Note.md)\n").unwrap();
		let report = vault.links("linker.md").unwrap();
		assert_eq!(report.outgoing[0].resolved.as_deref(), Some("My Note.md"));
		let back = vault.links("My Note.md").unwrap();
		assert_eq!(back.backlinks, vec!["linker.md"]);
	}

	// --- resolution order ---

	fn index(files: &[&str]) -> LinkIndex {
		index_with_aliases(files, &[])
	}

	fn index_with_aliases(files: &[&str], aliases: &[(usize, Vec<&str>)]) -> LinkIndex {
		index_with_aliases_and_attachment_folder(files, aliases, None)
	}

	fn index_with_aliases_and_attachment_folder(
		files: &[&str],
		aliases: &[(usize, Vec<&str>)],
		attachment_folder: Option<&str>,
	) -> LinkIndex {
		let aliases: Vec<(usize, Vec<String>)> = aliases
			.iter()
			.map(|(i, values)| (*i, values.iter().map(|value| value.to_string()).collect()))
			.collect();
		LinkIndex::build(
			files.iter().map(|f| f.to_string()).collect(),
			&aliases,
			attachment_folder.map(str::to_string),
		)
	}

	#[test]
	fn exact_path_wins_over_stem_matches() {
		let idx = index(&["alpha.md", "notes/alpha.md"]);
		// Stem "alpha" matches two files, but the exact root path wins first.
		assert_eq!(idx.resolve("alpha"), Resolution::Resolved("alpha.md".into()));
		assert_eq!(idx.resolve("alpha.md"), Resolution::Resolved("alpha.md".into()));
		assert_eq!(
			idx.resolve("notes/alpha"),
			Resolution::Resolved("notes/alpha.md".into())
		);
	}

	#[test]
	fn unique_stem_match_is_case_insensitive() {
		let idx = index(&["projects/Alpha.md", "notes/other.md"]);
		assert_eq!(idx.resolve("alpha"), Resolution::Resolved("projects/Alpha.md".into()));
		assert_eq!(idx.resolve("ALPHA"), Resolution::Resolved("projects/Alpha.md".into()));
	}

	#[test]
	fn file_name_targets_resolve_attachments() {
		let idx = index(&["attachments/logo.png", "notes/a.md"]);
		assert_eq!(
			idx.resolve("logo.png"),
			Resolution::Resolved("attachments/logo.png".into())
		);
		assert_eq!(idx.resolve("logo"), Resolution::Resolved("attachments/logo.png".into()));
	}

	#[test]
	fn configured_attachment_folder_wins_for_unqualified_attachments() {
		let idx = index_with_aliases_and_attachment_folder(
			&["attachments/logo.png", "other/logo.png", "notes/a.md"],
			&[],
			Some("attachments"),
		);
		assert_eq!(
			idx.resolve("logo.png"),
			Resolution::Resolved("attachments/logo.png".into())
		);
	}

	#[test]
	fn configured_attachment_folder_is_ignored_when_candidate_is_absent() {
		let idx = index_with_aliases_and_attachment_folder(&["other/logo.png", "notes/a.md"], &[], Some("attachments"));
		assert_eq!(idx.resolve("logo.png"), Resolution::Resolved("other/logo.png".into()));
	}

	#[test]
	fn multiple_stem_candidates_are_ambiguous_not_suffix_resolved() {
		let idx = index(&["notes/todo.md", "projects/todo.md"]);
		assert_eq!(
			idx.resolve("todo"),
			Resolution::Ambiguous(vec!["notes/todo.md".into(), "projects/todo.md".into()])
		);
	}

	#[test]
	fn unique_path_suffix_match() {
		let idx = index(&["deep/folder/Note.md", "other/Note2.md"]);
		assert_eq!(
			idx.resolve("folder/Note"),
			Resolution::Resolved("deep/folder/Note.md".into())
		);
		// Case-insensitive, with or without .md.
		assert_eq!(
			idx.resolve("FOLDER/note.md"),
			Resolution::Resolved("deep/folder/Note.md".into())
		);
	}

	#[test]
	fn multiple_suffix_candidates_are_ambiguous() {
		let idx = index(&["a/f/n.md", "b/f/n.md"]);
		assert_eq!(
			idx.resolve("f/n"),
			Resolution::Ambiguous(vec!["a/f/n.md".into(), "b/f/n.md".into()])
		);
	}

	#[test]
	fn unresolvable_targets_are_broken() {
		let idx = index(&["a.md"]);
		assert_eq!(idx.resolve("zzz"), Resolution::Broken);
		assert_eq!(idx.resolve("no/such/path"), Resolution::Broken);
		// Escaping and empty targets cannot resolve.
		assert_eq!(idx.resolve("../a"), Resolution::Broken);
		assert_eq!(idx.resolve(""), Resolution::Broken);
	}

	#[test]
	fn aliases_resolve_after_file_matches() {
		let idx = index_with_aliases(&["entities/acme.md", "client.md"], &[(0, vec!["Client", "ACME Corp"])]);
		assert_eq!(
			idx.resolve("ACME Corp"),
			Resolution::Resolved("entities/acme.md".into())
		);
		// A real file/stem wins over an alias of the same name.
		assert_eq!(idx.resolve("Client"), Resolution::Resolved("client.md".into()));
	}

	#[test]
	fn alias_matching_is_case_insensitive_and_collisions_are_ambiguous() {
		let idx = index_with_aliases(
			&["entities/acme.md", "entities/other.md"],
			&[(0, vec!["Client"]), (1, vec!["CLIENT"])],
		);
		assert_eq!(
			idx.resolve("client"),
			Resolution::Ambiguous(vec!["entities/acme.md".into(), "entities/other.md".into()])
		);
	}

	#[test]
	fn existing_resolution_behaviour_is_unchanged_with_aliases() {
		let idx = index_with_aliases(
			&["projects/Alpha.md", "deep/folder/Note.md"],
			&[(0, vec!["Something Else"])],
		);
		assert_eq!(idx.resolve("alpha"), Resolution::Resolved("projects/Alpha.md".into()));
		assert_eq!(
			idx.resolve("folder/Note"),
			Resolution::Resolved("deep/folder/Note.md".into())
		);
	}

	// --- Vault::links ---

	#[test]
	fn links_reports_outgoing_in_document_order() {
		let (_dir, vault) = test_fixtures::knowledge_vault();
		let report = vault.links("index.md").unwrap();
		let outgoing: Vec<(&str, Option<&str>)> = report
			.outgoing
			.iter()
			.map(|l| (l.raw.as_str(), l.resolved.as_deref()))
			.collect();
		assert_eq!(
			outgoing,
			vec![
				("[[Alpha]]", Some("projects/alpha.md")),
				("[[projects/beta|The Beta]]", Some("projects/beta.md")),
				("[[guide#Setup]]", Some("notes/guide.md")),
				("[guide](notes/guide.md)", Some("notes/guide.md")),
				("[[todo]]", None),         // ambiguous
				("[[missing-page]]", None), // broken
				("[[big]]", Some("big.md")),
				("[[stale]]", Some("stale.md")),
				("[[broken-fm]]", Some("notes/broken-fm.md")),
				("![[logo.png]]", Some("attachments/logo.png")),
			]
		);
		assert_eq!(report.outgoing[3].kind, LinkKind::Markdown);
		assert_eq!(report.outgoing[0].kind, LinkKind::Wikilink);
	}

	#[test]
	fn links_reports_backlinks_sorted() {
		let (_dir, vault) = test_fixtures::knowledge_vault();
		// index.md links [[Alpha]] (stem), guide.md links [[Alpha#Status|…]].
		let report = vault.links("projects/alpha.md").unwrap();
		assert_eq!(report.backlinks, vec!["index.md", "notes/guide.md"]);
	}

	#[test]
	fn links_resolve_frontmatter_aliases() {
		let (_dir, vault) = test_fixtures::vault();
		vault
			.write("entities/acme.md", "---\naliases: [ACME Corp, Acme]\n---\n# Acme\n")
			.unwrap();
		vault.write("linker.md", "[[Acme]] and [[ACME Corp]]\n").unwrap();
		let report = vault.links("linker.md").unwrap();
		let resolved: Vec<Option<&str>> = report.outgoing.iter().map(|link| link.resolved.as_deref()).collect();
		assert_eq!(resolved, vec![Some("entities/acme.md"), Some("entities/acme.md")]);
	}

	#[test]
	fn single_string_alias_is_honored() {
		let (_dir, vault) = test_fixtures::vault();
		vault
			.write("entities/acme.md", "---\naliases: ACME Corp\n---\n# Acme\n")
			.unwrap();
		vault.write("linker.md", "[[ACME Corp]]\n").unwrap();
		let report = vault.links("linker.md").unwrap();
		assert_eq!(report.outgoing[0].resolved.as_deref(), Some("entities/acme.md"));
	}

	#[test]
	fn embeds_count_as_backlinks() {
		let (_dir, vault) = test_fixtures::vault();
		vault.write("target.md", "# Target\n").unwrap();
		vault.write("embedder.md", "![[target]]\n").unwrap();
		let report = vault.links("target.md").unwrap();
		assert!(report.backlinks.contains(&"embedder.md".to_string()), "{report:?}");
	}

	#[test]
	fn links_inside_callouts_appear_in_the_link_graph() {
		let (_dir, vault) = test_fixtures::vault();
		vault.write("Link.md", "# Link\n").unwrap();
		vault.write("source.md", "> [!note] Related\n> See [[Link]]\n").unwrap();
		let report = vault.links("source.md").unwrap();
		assert_eq!(report.outgoing.len(), 1);
		assert_eq!(report.outgoing[0].resolved.as_deref(), Some("Link.md"));
		let back = vault.links("Link.md").unwrap();
		assert_eq!(back.backlinks, vec!["source.md"]);
	}

	#[test]
	fn attachment_embeds_resolve_and_are_not_broken() {
		let (_dir, vault) = test_fixtures::vault();
		vault.write("page.md", "![[logo.png]]\n").unwrap();
		let report = vault.links("page.md").unwrap();
		assert_eq!(report.outgoing.len(), 1);
		assert!(report.outgoing[0].embed);
		assert_eq!(report.outgoing[0].resolved.as_deref(), Some("attachments/logo.png"));
	}

	#[test]
	fn obsidian_attachment_folder_resolves_configured_duplicate_attachment() {
		let (dir, vault) = test_fixtures::vault();
		std::fs::write(
			dir.path().join(".obsidian/app.json"),
			r#"{"attachmentFolderPath":"assets"}"#,
		)
		.unwrap();
		vault.write("assets/logo.png", "configured").unwrap();
		vault.write("other/logo.png", "duplicate").unwrap();
		vault.write("page.md", "![[logo.png]]\n").unwrap();
		let report = vault.links("page.md").unwrap();
		assert_eq!(report.outgoing.len(), 1);
		assert_eq!(report.outgoing[0].resolved.as_deref(), Some("assets/logo.png"));
	}

	#[test]
	fn links_on_attachment_has_no_outgoing_but_backlinks_work() {
		let (_dir, vault) = test_fixtures::knowledge_vault();
		let report = vault.links("attachments/logo.png").unwrap();
		assert!(report.outgoing.is_empty());
		assert_eq!(report.backlinks, vec!["index.md"]);
	}

	#[test]
	fn links_page_without_backlinks_is_empty() {
		let (_dir, vault) = test_fixtures::knowledge_vault();
		let report = vault.links("orphan.md").unwrap();
		assert!(report.outgoing.is_empty());
		assert!(report.backlinks.is_empty());
	}

	#[test]
	fn links_rejects_missing_and_hidden_paths() {
		let (_dir, vault) = test_fixtures::knowledge_vault();
		assert!(matches!(vault.links("nope.md"), Err(WikidError::NotFound { .. })));
		assert!(matches!(
			vault.links(".obsidian/app.json"),
			Err(WikidError::InvalidPath { .. })
		));
		assert!(matches!(vault.links("projects"), Err(WikidError::InvalidPath { .. })));
	}

	#[cfg(unix)]
	#[test]
	fn links_surfaces_unreadable_pages_as_io_errors() {
		use std::os::unix::fs::PermissionsExt;
		let (dir, vault) = test_fixtures::vault();
		std::fs::set_permissions(dir.path().join("index.md"), std::fs::Permissions::from_mode(0o000)).unwrap();
		let err = vault.links("index.md").unwrap_err();
		assert!(matches!(err, WikidError::Io(_)), "got {err:?}");
		// Restore so TempDir cleanup and other assertions behave.
		std::fs::set_permissions(dir.path().join("index.md"), std::fs::Permissions::from_mode(0o644)).unwrap();
	}

	#[cfg(unix)]
	#[test]
	fn links_ignores_symlinks_out_of_the_vault() {
		let outside = tempfile::tempdir().unwrap();
		std::fs::write(outside.path().join("secret.md"), "links to [[index]]\n").unwrap();
		let (dir, vault) = test_fixtures::vault();
		std::os::unix::fs::symlink(outside.path().join("secret.md"), dir.path().join("escape.md")).unwrap();
		let report = vault.links("index.md").unwrap();
		assert!(
			!report.backlinks.contains(&"escape.md".to_string()),
			"out-of-vault symlink leaked into backlinks: {:?}",
			report.backlinks
		);
	}

	#[test]
	fn link_report_round_trips_as_json() {
		let (_dir, vault) = test_fixtures::knowledge_vault();
		let report = vault.links("index.md").unwrap();
		let json = serde_json::to_string(&report).unwrap();
		assert!(json.contains("\"kind\":\"wikilink\""));
		assert!(json.contains("\"kind\":\"markdown\""));
		assert!(json.contains("\"embed\":true"));
		assert!(json.contains("\"fragment\":\"Setup\""));
		let back: LinkReport = serde_json::from_str(&json).unwrap();
		assert_eq!(report, back);
	}

	#[test]
	fn old_link_json_defaults_embed_and_fragment() {
		let json = r#"{"raw":"[[Note]]","target":"Note","resolved":null,"kind":"wikilink"}"#;
		let link: Link = serde_json::from_str(json).unwrap();
		assert!(!link.embed);
		assert_eq!(link.fragment, None);
	}
}
