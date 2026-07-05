//! YAML frontmatter (DESIGN §4): a leading `---` block parsed into a
//! string-keyed map. Absence is normal; malformed YAML degrades to "no
//! frontmatter" while preserving parser details so doctor can flag it.

use std::collections::BTreeMap;

use serde::{Deserialize, Serialize};

/// Outcome of scanning a page for a leading frontmatter block.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub enum Frontmatter {
	/// No leading `---` block — the normal case for plain Markdown.
	Absent,
	/// A `---` block is present but is not a string-keyed YAML map.
	/// Consumers treat this as no frontmatter; doctor flags it with detail.
	Malformed(String),
	/// A well-formed block: the parsed string-keyed map.
	Present(BTreeMap<String, serde_yaml::Value>),
}

impl Frontmatter {
	/// True when a block was present but unparseable.
	pub fn is_malformed(&self) -> bool {
		matches!(self, Self::Malformed(_))
	}

	/// Parser/type error detail for a malformed block.
	pub fn malformed_detail(&self) -> Option<&str> {
		match self {
			Self::Malformed(detail) => Some(detail),
			_ => None,
		}
	}
}

/// Parses the leading frontmatter block of `content`.
pub fn parse(content: &str) -> Frontmatter {
	split(content).0
}

/// The display title of a page (DESIGN §4): frontmatter `title` (string,
/// non-empty) → first `# ` heading in the body → the file stem.
pub fn page_title(content: &str, stem: &str) -> String {
	let (fm, body) = split(content);
	if let Frontmatter::Present(map) = &fm
		&& let Some(serde_yaml::Value::String(title)) = map.get("title")
		&& !title.trim().is_empty()
	{
		return title.trim().to_string();
	}
	body.lines()
		.find_map(|line| line.strip_prefix("# "))
		.map(|heading| heading.trim().to_string())
		.filter(|heading| !heading.is_empty())
		.unwrap_or_else(|| stem.to_string())
}

/// Splits `content` into its frontmatter and the body after the block. An
/// opening `---` with no closing line is not a block: everything is body.
fn split(content: &str) -> (Frontmatter, &str) {
	let mut offset = 0usize;
	let mut block_start = 0usize;
	let mut opened = false;
	for line in content.split_inclusive('\n') {
		let line_start = offset;
		offset += line.len();
		if !opened {
			if line_start == 0 && line.trim_end() == "---" {
				opened = true;
				block_start = offset;
				continue;
			}
			return (Frontmatter::Absent, content);
		}
		if line.trim_end() == "---" {
			return (parse_block(&content[block_start..line_start]), &content[offset..]);
		}
	}
	(Frontmatter::Absent, content)
}

/// Parses the text between the `---` delimiters. An empty block is valid
/// (Obsidian writes them); anything that is not a string-keyed map is
/// malformed.
fn parse_block(block: &str) -> Frontmatter {
	if block.trim().is_empty() {
		return Frontmatter::Present(BTreeMap::new());
	}
	match serde_yaml::from_str::<BTreeMap<String, serde_yaml::Value>>(block) {
		Ok(map) => Frontmatter::Present(map),
		Err(err) => Frontmatter::Malformed(err.to_string()),
	}
}

#[cfg(test)]
mod tests {
	use super::*;

	#[test]
	fn absent_when_no_block() {
		assert_eq!(parse("# Title\n\nbody\n"), Frontmatter::Absent);
		assert_eq!(parse(""), Frontmatter::Absent);
	}

	#[test]
	fn absent_when_block_does_not_start_at_byte_zero() {
		assert_eq!(parse("\n---\ntitle: X\n---\n"), Frontmatter::Absent);
		assert_eq!(parse(" ---\ntitle: X\n---\n"), Frontmatter::Absent);
	}

	#[test]
	fn absent_when_opener_is_never_closed() {
		// A lone leading --- is a horizontal rule, not frontmatter.
		assert_eq!(parse("---\ntitle: X\n\nbody\n"), Frontmatter::Absent);
		assert_eq!(parse("---"), Frontmatter::Absent);
		assert_eq!(parse("---\n"), Frontmatter::Absent);
	}

	#[test]
	fn present_parses_string_keyed_map() {
		let fm = parse("---\ntitle: Alpha\ntags:\n  - a\n  - b\n---\n\nbody\n");
		let Frontmatter::Present(map) = fm else {
			panic!("expected Present, got {fm:?}");
		};
		assert_eq!(map.get("title"), Some(&serde_yaml::Value::String("Alpha".into())));
		assert_eq!(map["tags"].as_sequence().unwrap().len(), 2);
	}

	#[test]
	fn present_empty_block_is_an_empty_map() {
		assert_eq!(parse("---\n---\nbody\n"), Frontmatter::Present(BTreeMap::new()));
		assert_eq!(parse("---\n\n---\n"), Frontmatter::Present(BTreeMap::new()));
	}

	#[test]
	fn present_handles_crlf_line_endings() {
		let fm = parse("---\r\ntitle: X\r\n---\r\nbody\r\n");
		let Frontmatter::Present(map) = fm else {
			panic!("expected Present, got {fm:?}");
		};
		assert_eq!(map.get("title"), Some(&serde_yaml::Value::String("X".into())));
	}

	#[test]
	fn malformed_when_yaml_does_not_parse() {
		let fm = parse("---\ntitle: [unclosed\n---\nbody\n");
		assert!(fm.is_malformed(), "got {fm:?}");
	}

	#[test]
	fn malformed_when_yaml_is_not_a_string_keyed_map() {
		assert!(parse("---\n- just\n- a list\n---\n").is_malformed());
		assert!(parse("---\nplain scalar\n---\n").is_malformed());
	}

	#[test]
	fn closing_delimiter_may_be_the_last_line_without_newline() {
		assert_eq!(parse("---\n---"), Frontmatter::Present(BTreeMap::new()));
	}

	// --- title precedence ---

	#[test]
	fn title_prefers_frontmatter() {
		let content = "---\ntitle: Alpha Project\n---\n\n# Alpha Heading\n";
		assert_eq!(page_title(content, "alpha"), "Alpha Project");
	}

	#[test]
	fn title_falls_back_to_first_h1_heading() {
		assert_eq!(page_title("# Beta\n\nbody\n", "beta"), "Beta");
		// Frontmatter present but without a usable title.
		assert_eq!(
			page_title("---\ntags: [x]\n---\n\n# From Heading\n", "s"),
			"From Heading"
		);
		// Non-string titles are not usable.
		assert_eq!(page_title("---\ntitle: 42\n---\n\n# Heading\n", "s"), "Heading");
		// Deeper headings do not count.
		assert_eq!(page_title("## Sub\n\n# Real\n", "s"), "Real");
	}

	#[test]
	fn title_falls_back_to_stem() {
		assert_eq!(page_title("no headings here\n", "orphan"), "orphan");
		assert_eq!(page_title("", "empty"), "empty");
		// A malformed block is skipped: heading search starts after it.
		assert_eq!(page_title("---\ntitle: [bad\n---\nno heading\n", "bad-fm"), "bad-fm");
	}

	#[test]
	fn title_of_malformed_frontmatter_uses_body_heading() {
		assert_eq!(page_title("---\ntitle: [bad\n---\n\n# Rescue\n", "x"), "Rescue");
	}

	#[test]
	fn frontmatter_round_trips_as_json() {
		let fm = parse("---\ntitle: X\n---\n");
		let json = serde_json::to_string(&fm).unwrap();
		let back: Frontmatter = serde_json::from_str(&json).unwrap();
		assert_eq!(fm, back);
	}
}
