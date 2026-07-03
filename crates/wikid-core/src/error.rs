//! The wikid error model: one stable enum shared by every surface (CLI,
//! HTTP, MCP). Surfaces render errors from `code()`, `Display`, and `hint()`
//! — they never match on variants for presentation.

use thiserror::Error;

/// Every failure a vault operation can produce.
#[derive(Debug, Error)]
pub enum WikidError {
	/// The target does not exist in the vault.
	#[error("not found: {path}")]
	NotFound {
		/// Vault-relative path that was requested.
		path: String,
	},

	/// The path is absolute, escapes the vault root, contains hidden
	/// components, or otherwise violates the vault path rules.
	#[error("invalid path: {path} ({reason})")]
	InvalidPath {
		/// The path as the caller supplied it.
		path: String,
		/// Which rule it violated.
		reason: String,
	},

	/// The destination already exists (mv without force).
	#[error("already exists: {path}")]
	AlreadyExists {
		/// Vault-relative path of the existing destination.
		path: String,
	},

	/// An edit found zero occurrences of the old text.
	#[error("no match in {path}")]
	NoMatch {
		/// Vault-relative path of the page that was searched.
		path: String,
		/// Best-effort location of the most similar existing line (1-based).
		nearest_line: Option<usize>,
	},

	/// An edit found more than one occurrence without `all`.
	#[error("ambiguous edit in {path}: {count} matches")]
	Ambiguous {
		/// Vault-relative path of the page.
		path: String,
		/// How many occurrences were found.
		count: usize,
	},

	/// The file exists but is not valid UTF-8 (binary attachment).
	#[error("not valid UTF-8: {path}")]
	NotUtf8 {
		/// Vault-relative path of the binary file.
		path: String,
	},

	/// A regex or glob pattern failed to compile.
	#[error("bad pattern {pattern:?}: {reason}")]
	BadPattern {
		/// The pattern as supplied.
		pattern: String,
		/// Compiler error text.
		reason: String,
	},

	/// An underlying filesystem error.
	#[error("io error: {0}")]
	Io(#[from] std::io::Error),
}

impl WikidError {
	/// Stable machine-readable identifier, used verbatim in CLI and HTTP
	/// error bodies. Never change these strings.
	pub fn code(&self) -> &'static str {
		match self {
			Self::NotFound { .. } => "not_found",
			Self::InvalidPath { .. } => "invalid_path",
			Self::AlreadyExists { .. } => "already_exists",
			Self::NoMatch { .. } => "no_match",
			Self::Ambiguous { .. } => "ambiguous",
			Self::NotUtf8 { .. } => "not_utf8",
			Self::BadPattern { .. } => "bad_pattern",
			Self::Io(_) => "io",
		}
	}

	/// Optional next-step suggestion, rendered verbatim by CLI and HTTP.
	pub fn hint(&self) -> Option<String> {
		match self {
			Self::NotFound { .. } => Some("run ls or glob to discover valid paths".to_string()),
			Self::InvalidPath { .. } => Some(
				"paths are vault-relative with forward slashes; absolute, '..', and hidden ('.') components are refused"
					.to_string(),
			),
			Self::AlreadyExists { .. } => Some("pass force to overwrite the destination".to_string()),
			Self::NoMatch { nearest_line: Some(line), .. } => {
				Some(format!("closest similar content is near line {line} — cat the page and copy the exact text"))
			}
			Self::NoMatch { nearest_line: None, .. } => {
				Some("cat the page and copy the exact text to replace".to_string())
			}
			Self::Ambiguous { count, .. } => {
				Some(format!("add surrounding context to match uniquely, or pass all to replace all {count} occurrences"))
			}
			Self::NotUtf8 { .. } | Self::BadPattern { .. } | Self::Io(_) => None,
		}
	}
}

#[cfg(test)]
mod tests {
	use super::*;

	#[test]
	fn codes_are_stable() {
		let cases: Vec<(WikidError, &str)> = vec![
			(WikidError::NotFound { path: "a.md".into() }, "not_found"),
			(
				WikidError::InvalidPath {
					path: "../a".into(),
					reason: "escape".into(),
				},
				"invalid_path",
			),
			(WikidError::AlreadyExists { path: "a.md".into() }, "already_exists"),
			(
				WikidError::NoMatch {
					path: "a.md".into(),
					nearest_line: None,
				},
				"no_match",
			),
			(
				WikidError::Ambiguous {
					path: "a.md".into(),
					count: 3,
				},
				"ambiguous",
			),
			(WikidError::NotUtf8 { path: "a.png".into() }, "not_utf8"),
			(
				WikidError::BadPattern {
					pattern: "(".into(),
					reason: "unclosed".into(),
				},
				"bad_pattern",
			),
			(WikidError::Io(std::io::Error::other("disk")), "io"),
		];
		for (err, code) in cases {
			assert_eq!(err.code(), code);
		}
	}

	#[test]
	fn no_match_hint_carries_nearest_line() {
		let err = WikidError::NoMatch {
			path: "a.md".into(),
			nearest_line: Some(7),
		};
		assert!(err.hint().unwrap().contains("line 7"));
		let err = WikidError::NoMatch {
			path: "a.md".into(),
			nearest_line: None,
		};
		assert!(err.hint().is_some());
	}

	#[test]
	fn ambiguous_hint_carries_count() {
		let err = WikidError::Ambiguous {
			path: "a.md".into(),
			count: 4,
		};
		assert!(err.hint().unwrap().contains('4'));
	}

	#[test]
	fn io_and_pattern_errors_have_no_hint() {
		assert!(WikidError::Io(std::io::Error::other("disk")).hint().is_none());
		assert!(
			WikidError::BadPattern {
				pattern: "(".into(),
				reason: "x".into()
			}
			.hint()
			.is_none()
		);
		assert!(WikidError::NotUtf8 { path: "a.png".into() }.hint().is_none());
	}
}
