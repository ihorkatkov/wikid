//! Obsidian callout metadata extraction.
//!
//! Callouts are recognized for structured metadata only. The parser does not
//! render, strip, or otherwise transform page content.

use serde::{Deserialize, Serialize};

use crate::markdown::FenceTracker;

/// One Obsidian callout block header found in a Markdown page.
///
/// This is a wire-visible core type when surfaced by CLI/HTTP/MCP adapters.
/// `foldable` is `Some(true)` for folded callouts written with `-`,
/// `Some(false)` for explicitly expanded callouts written with `+`, and
/// `None` when no fold marker is present.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Callout {
	/// Callout type from `[!TYPE]`, normalized to lowercase.
	pub kind: String,
	/// Optional title text after the marker and optional fold state.
	#[serde(default, skip_serializing_if = "Option::is_none")]
	pub title: Option<String>,
	/// Optional fold state: `true` = folded (`-`), `false` = expanded (`+`).
	#[serde(default, skip_serializing_if = "Option::is_none")]
	pub foldable: Option<bool>,
}

/// Extracts Obsidian callout block headers from Markdown content.
///
/// Recognized grammar is a blockquote header line of the form
/// `> [!TYPE]`, optionally followed by `+` or `-` and title text. Callouts
/// inside fenced code blocks are ignored. Only metadata from the header is
/// returned; nested quoted content is left untouched and is not included in the
/// result.
pub fn extract_callouts(content: &str) -> Vec<Callout> {
	let mut callouts = Vec::new();
	let mut fences = FenceTracker::new();

	for line in content.lines() {
		if fences.observe(line) || fences.in_fence() {
			continue;
		}
		let trimmed_start = line.trim_start();

		let Some(after_quote) = trimmed_start.strip_prefix('>') else {
			continue;
		};
		let after_quote = after_quote.trim_start();
		let Some(after_bang) = after_quote.strip_prefix("[!") else {
			continue;
		};
		let Some(close) = after_bang.find(']') else {
			continue;
		};

		let kind = after_bang[..close].trim();
		if kind.is_empty() {
			continue;
		}

		let rest = after_bang[close + 1..].trim_start();
		let (foldable, title) = match rest.as_bytes().first().copied() {
			Some(b'-') => (Some(true), rest[1..].trim()),
			Some(b'+') => (Some(false), rest[1..].trim()),
			_ => (None, rest.trim()),
		};

		callouts.push(Callout {
			kind: kind.to_ascii_lowercase(),
			title: (!title.is_empty()).then(|| title.to_string()),
			foldable,
		});
	}

	callouts
}

#[cfg(test)]
mod tests {
	use super::*;

	#[test]
	fn extracts_kind_and_title_from_callout_header() {
		let callouts = extract_callouts("> [!note] Title\n> body\n");
		assert_eq!(
			callouts,
			vec![Callout {
				kind: "note".into(),
				title: Some("Title".into()),
				foldable: None,
			}]
		);
	}

	#[test]
	fn extracts_folded_and_expanded_fold_markers() {
		let callouts = extract_callouts("> [!warning]-\n> [!tip]+ Open\n");
		assert_eq!(callouts[0].kind, "warning");
		assert_eq!(callouts[0].foldable, Some(true));
		assert_eq!(callouts[0].title, None);
		assert_eq!(callouts[1].kind, "tip");
		assert_eq!(callouts[1].foldable, Some(false));
		assert_eq!(callouts[1].title.as_deref(), Some("Open"));
	}

	#[test]
	fn normal_text_marker_is_not_a_callout() {
		assert!(extract_callouts("[!note] in normal text\n").is_empty());
	}

	#[test]
	fn callouts_inside_fenced_code_blocks_are_ignored() {
		let callouts = extract_callouts("```\n> [!note] ignored\n```\n> [!note] kept\n~~~\n> [!tip] ignored\n~~~\n");
		assert_eq!(callouts.len(), 1);
		assert_eq!(callouts[0].title.as_deref(), Some("kept"));
	}

	#[test]
	fn mixed_fence_delimiters_do_not_flip_callout_scanning() {
		let callouts = extract_callouts("```\n~~~\n> [!note] ignored\n```\n> [!note] kept\n");
		assert_eq!(callouts.len(), 1);
		assert_eq!(callouts[0].title.as_deref(), Some("kept"));
	}
}
