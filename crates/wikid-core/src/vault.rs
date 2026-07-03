use std::path::{Path, PathBuf};

use crate::error::WikidError;
use crate::paths::{self, Resolved};

/// A wiki: a plain directory of Markdown files.
///
/// Obsidian-compatible by convention: YAML frontmatter and `[[wikilinks]]`
/// are parsed when present, `.obsidian/` is ignored, and nothing is required.
#[derive(Debug)]
pub struct Vault {
	root: PathBuf,
}

impl Vault {
	/// Opens an existing directory as a vault. The directory must exist;
	/// no setup, migration, or schema is required. The root is canonicalized
	/// once here so containment checks compare against a stable base.
	pub fn open(root: impl Into<PathBuf>) -> Result<Self, WikidError> {
		let root = root.into();
		if !root.is_dir() {
			return Err(WikidError::NotFound {
				path: root.display().to_string(),
			});
		}
		let root = root.canonicalize()?;
		Ok(Self { root })
	}

	/// The canonicalized vault root on disk.
	pub fn root(&self) -> &Path {
		&self.root
	}

	/// Validates a user-supplied vault-relative path (see `paths`).
	pub(crate) fn resolve(&self, path: &str) -> Result<Resolved, WikidError> {
		paths::resolve(&self.root, path)
	}

	/// The shared ignore-rules walker used by every read operation: hidden
	/// files and dot-directories are skipped at any depth, and `.gitignore` /
	/// `.ignore` files inside the vault are respected. Ignore files are only
	/// read from within the vault (no parent dirs, no machine-global config)
	/// and apply whether or not the vault is a git repository.
	pub(crate) fn walker(&self) -> ignore::Walk {
		ignore::WalkBuilder::new(&self.root)
			.hidden(true)
			.ignore(true)
			.git_ignore(true)
			.require_git(false)
			.git_global(false)
			.git_exclude(false)
			.parents(false)
			.follow_links(false)
			.sort_by_file_name(|a, b| a.cmp(b))
			.build()
	}
}

#[cfg(test)]
mod tests {
	use super::*;

	#[test]
	fn open_rejects_missing_directory() {
		let err = Vault::open("/nonexistent/wiki/dir").unwrap_err();
		assert!(matches!(err, WikidError::NotFound { .. }));
	}

	#[test]
	fn open_accepts_existing_directory() {
		let dir = tempfile::tempdir().unwrap();
		let vault = Vault::open(dir.path()).unwrap();
		assert!(vault.root().is_dir());
	}
}
