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

	/// A page exists, but the requested heading or block fragment does not.
	#[error("fragment not found in {path}: {fragment}")]
	FragmentNotFound {
		/// Vault-relative page path.
		path: String,
		/// Fragment text as requested after the trailing `#`.
		fragment: String,
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

	/// An edit named line hashes that no longer match the file: the page
	/// changed since it was read. The whole batch is refused.
	#[error("stale edit in {path}: {detail}")]
	StaleEdit {
		/// Vault-relative path of the page.
		path: String,
		/// Per-line mismatch report: `line N is <hash> "<text>", not <hash>`,
		/// one clause per stale line.
		detail: String,
	},

	/// An edit batch is structurally invalid (empty, line out of range,
	/// duplicate line numbers).
	#[error("bad edit in {path}: {reason}")]
	BadEdit {
		/// Vault-relative path of the page.
		path: String,
		/// Which rule the batch violated.
		reason: String,
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
			Self::FragmentNotFound { .. } => "fragment_not_found",
			Self::InvalidPath { .. } => "invalid_path",
			Self::AlreadyExists { .. } => "already_exists",
			Self::StaleEdit { .. } => "stale_edit",
			Self::BadEdit { .. } => "bad_edit",
			Self::NotUtf8 { .. } => "not_utf8",
			Self::BadPattern { .. } => "bad_pattern",
			Self::Io(_) => "io",
		}
	}

	/// Optional next-step suggestion, rendered verbatim by CLI and HTTP.
	pub fn hint(&self) -> Option<String> {
		match self {
			Self::NotFound { .. } => Some("run ls or glob to discover valid paths".to_string()),
			Self::FragmentNotFound { path, .. } => Some(format!(
				"run links {path} or cat {path} to inspect available headings and block anchors"
			)),
			Self::InvalidPath { .. } => Some(
				"paths are vault-relative with forward slashes; absolute, '..', and hidden ('.') components are refused"
					.to_string(),
			),
			Self::AlreadyExists { .. } => Some("pass force to overwrite the destination".to_string()),
			Self::StaleEdit { path, .. } => Some(format!(
				"the page changed since it was read — run cat {path} with hashes and retry with fresh line hashes"
			)),
			Self::BadEdit { path, .. } => Some(format!("run cat {path} with hashes to see current line numbers")),
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
				WikidError::FragmentNotFound {
					path: "a.md".into(),
					fragment: "Missing".into(),
				},
				"fragment_not_found",
			),
			(
				WikidError::InvalidPath {
					path: "../a".into(),
					reason: "escape".into(),
				},
				"invalid_path",
			),
			(WikidError::AlreadyExists { path: "a.md".into() }, "already_exists"),
			(
				WikidError::StaleEdit {
					path: "a.md".into(),
					detail: "line 7 changed".into(),
				},
				"stale_edit",
			),
			(
				WikidError::BadEdit {
					path: "a.md".into(),
					reason: "line 99 is out of range".into(),
				},
				"bad_edit",
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
	fn stale_edit_message_carries_detail_and_hint_names_the_page() {
		let err = WikidError::StaleEdit {
			path: "a.md".into(),
			detail: "line 7 is now 4d5e6f7a8b9c".into(),
		};
		assert!(err.to_string().contains("line 7 is now 4d5e6f7a8b9c"));
		assert!(err.hint().unwrap().contains("a.md"));
	}

	#[test]
	fn bad_edit_message_carries_reason() {
		let err = WikidError::BadEdit {
			path: "a.md".into(),
			reason: "line 99 is out of range (page has 3 lines)".into(),
		};
		assert!(err.to_string().contains("out of range"));
		assert!(err.hint().unwrap().contains("a.md"));
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
