//! The Obsidian-compatible link model (DESIGN §4): extraction of
//! `[[Target]]`, `[[Target|alias]]`, `[[Target#Heading]]`, and markdown
//! links, plus resolution against the vault's visible files.

use std::collections::BTreeMap;
use std::sync::OnceLock;

use serde::{Deserialize, Serialize};

use crate::error::WikidError;
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
	/// The resolution target after stripping alias and heading parts.
	pub target: String,
	/// Vault-relative path the link resolves to; `None` when broken or ambiguous.
	pub resolved: Option<String>,
	/// Wikilink or markdown.
	pub kind: LinkKind,
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
	/// The target with alias, heading, and anchor parts stripped.
	pub target: String,
	/// Wikilink or markdown.
	pub kind: LinkKind,
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
		// Alias splits first ([[target#heading|alias]]), then the heading.
		let inner = &caps[1];
		let target = inner.split('|').next().unwrap_or(inner);
		let target = target.split('#').next().unwrap_or(target).trim();
		if target.is_empty() {
			continue;
		}
		found.push((
			whole.start(),
			ExtractedLink {
				raw: whole.as_str().to_string(),
				target: target.to_string(),
				kind: LinkKind::Wikilink,
			},
		));
	}
	for caps in markdown_re().captures_iter(content) {
		let whole = caps.get(0).expect("capture 0");
		let inner = caps[1].trim();
		let inner = inner
			.strip_prefix('<')
			.and_then(|s| s.strip_suffix('>'))
			.unwrap_or(inner);
		if inner.starts_with("http://") || inner.starts_with("https://") || inner.starts_with("mailto:") {
			continue;
		}
		let target = inner.split('#').next().unwrap_or(inner).trim();
		if target.is_empty() {
			continue;
		}
		found.push((
			whole.start(),
			ExtractedLink {
				raw: whole.as_str().to_string(),
				target: target.to_string(),
				kind: LinkKind::Markdown,
			},
		));
	}
	found.sort_by_key(|(start, _)| *start);
	found.into_iter().map(|(_, link)| link).collect()
}

/// The resolution index: every visible file in the vault, addressable by
/// exact path, stem or file name, or path suffix (DESIGN §4).
pub(crate) struct LinkIndex {
	/// All visible vault-relative file paths.
	files: Vec<String>,
	/// Lowercased stem and full file name → indices into `files`.
	by_name: BTreeMap<String, Vec<usize>>,
}

impl LinkIndex {
	/// Builds the index from the vault-relative paths of all visible files.
	pub(crate) fn build(files: Vec<String>) -> Self {
		let mut by_name: BTreeMap<String, Vec<usize>> = BTreeMap::new();
		for (i, rel) in files.iter().enumerate() {
			let name = rel.rsplit('/').next().unwrap_or(rel).to_lowercase();
			let stem = name.rsplit_once('.').map(|(s, _)| s.to_string()).unwrap_or_default();
			by_name.entry(name).or_default().push(i);
			if !stem.is_empty() {
				by_name.entry(stem).or_default().push(i);
			}
		}
		Self { files, by_name }
	}

	/// Resolution order (DESIGN §4): (1) exact relative path from the root,
	/// with or without `.md`; (2) for bare targets, a unique case-insensitive
	/// stem or file-name match anywhere in the vault; (3) a unique
	/// case-insensitive path-suffix match (`folder/Note`). Multiple candidates
	/// at a stage yield `Ambiguous`; no candidates yield `Broken`.
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
		if !rel.contains('/') {
			let candidates = match self.by_name.get(&rel.to_lowercase()) {
				Some(indices) => indices.iter().map(|&i| self.files[i].clone()).collect(),
				None => Vec::new(),
			};
			return decide(candidates);
		}
		let rel_lower = rel.to_lowercase();
		let suffixes = [rel_lower.clone(), format!("{rel_lower}.md")];
		let candidates = self
			.files
			.iter()
			.filter(|f| {
				let f = f.to_lowercase();
				suffixes.iter().any(|s| f == *s || f.ends_with(&format!("/{s}")))
			})
			.cloned()
			.collect();
		decide(candidates)
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
		let index = LinkIndex::build(files.iter().map(|(rel, _)| rel.clone()).collect());
		let mut outgoing = Vec::new();
		let mut backlinks = Vec::new();
		for (rel, abs) in &files {
			if !is_page(rel) {
				continue;
			}
			let Some(text) = read_text(abs) else { continue };
			let extracted = extract_links(&text);
			if *rel == target.rel {
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
						}
					})
					.collect();
			} else if extracted
				.iter()
				.any(|l| matches!(index.resolve(&l.target), Resolution::Resolved(p) if p == target.rel))
			{
				backlinks.push(rel.clone());
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
			 [h](http://e.com) [m](mailto:a@b.c) [a](#top) [[#heading]] [b](<my file.md>)\n",
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
	}

	#[test]
	fn markdown_fragment_is_stripped_for_resolution() {
		let links = extract_links("[s](notes/guide.md#setup)\n");
		assert_eq!(links[0].target, "notes/guide.md");
	}

	// --- resolution order ---

	fn index(files: &[&str]) -> LinkIndex {
		LinkIndex::build(files.iter().map(|f| f.to_string()).collect())
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

	#[test]
	fn link_report_round_trips_as_json() {
		let (_dir, vault) = test_fixtures::knowledge_vault();
		let report = vault.links("index.md").unwrap();
		let json = serde_json::to_string(&report).unwrap();
		assert!(json.contains("\"kind\":\"wikilink\""));
		assert!(json.contains("\"kind\":\"markdown\""));
		let back: LinkReport = serde_json::from_str(&json).unwrap();
		assert_eq!(report, back);
	}
}
