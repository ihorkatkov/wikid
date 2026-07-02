use std::path::{Path, PathBuf};

use anyhow::{Context, Result, ensure};

/// A wiki: a plain directory of Markdown files.
///
/// Obsidian-compatible by convention: YAML frontmatter and `[[wikilinks]]`
/// are parsed when present, `.obsidian/` is ignored, and nothing is required.
pub struct Vault {
	root: PathBuf,
}

impl Vault {
	/// Opens an existing directory as a vault. The directory must exist;
	/// no setup, migration, or schema is required.
	pub fn open(root: impl Into<PathBuf>) -> Result<Self> {
		let root = root.into();
		ensure!(root.is_dir(), "vault root is not a directory: {}", root.display());
		let root = root.canonicalize().context("failed to canonicalize vault root")?;
		Ok(Self { root })
	}

	pub fn root(&self) -> &Path {
		&self.root
	}
}

#[cfg(test)]
mod tests {
	use super::*;

	#[test]
	fn open_rejects_missing_directory() {
		assert!(Vault::open("/nonexistent/wiki/dir").is_err());
	}

	#[test]
	fn open_accepts_existing_directory() {
		let dir = std::env::temp_dir();
		let vault = Vault::open(&dir).unwrap();
		assert!(vault.root().is_dir());
	}
}
