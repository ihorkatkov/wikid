//! Targeted, tolerant reads of Obsidian vault configuration.
//!
//! Only `.obsidian/app.json`'s `attachmentFolderPath` setting is in scope.
//! The normal vault walker still hides `.obsidian/`; this module reads that
//! one known path directly and treats absence or malformed content as empty
//! configuration, matching the crate's tolerant frontmatter posture.

use std::path::Path;

use serde::Deserialize;

use crate::paths;

/// Obsidian configuration understood by wikid.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub(crate) struct ObsidianConfig {
	/// Vault-relative attachment folder from `.obsidian/app.json`.
	pub(crate) attachment_folder: Option<String>,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct AppJson {
	attachment_folder_path: Option<String>,
}

impl ObsidianConfig {
	/// Loads the single supported setting from `.obsidian/app.json`.
	///
	/// Missing `.obsidian/`, missing `app.json`, unreadable files, malformed
	/// JSON, non-string values, and invalid/hidden paths all degrade to empty
	/// config without surfacing an error.
	pub(crate) fn load(root: &Path) -> Self {
		let Ok(text) = std::fs::read_to_string(root.join(".obsidian").join("app.json")) else {
			return Self::default();
		};
		let Ok(parsed) = serde_json::from_str::<AppJson>(&text) else {
			return Self::default();
		};
		let attachment_folder = parsed
			.attachment_folder_path
			.and_then(|path| normalize_attachment_folder(&path));
		Self { attachment_folder }
	}
}

fn normalize_attachment_folder(path: &str) -> Option<String> {
	let components = paths::normalize(path).ok()?;
	if components.iter().any(|component| component.starts_with('.')) {
		return None;
	}
	Some(components.join("/"))
}

#[cfg(test)]
mod tests {
	use super::*;

	fn write(root: &Path, rel: &str, content: &str) {
		let path = root.join(rel);
		std::fs::create_dir_all(path.parent().unwrap()).unwrap();
		std::fs::write(path, content).unwrap();
	}

	#[test]
	fn loads_attachment_folder_path_from_app_json() {
		let dir = tempfile::tempdir().unwrap();
		write(dir.path(), ".obsidian/app.json", r#"{"attachmentFolderPath":"assets"}"#);
		let config = ObsidianConfig::load(dir.path());
		assert_eq!(config.attachment_folder.as_deref(), Some("assets"));
	}

	#[test]
	fn missing_obsidian_dir_loads_empty_config() {
		let dir = tempfile::tempdir().unwrap();
		let config = ObsidianConfig::load(dir.path());
		assert_eq!(config, ObsidianConfig::default());
	}

	#[test]
	fn malformed_app_json_loads_empty_config() {
		let dir = tempfile::tempdir().unwrap();
		write(dir.path(), ".obsidian/app.json", "{not json");
		let config = ObsidianConfig::load(dir.path());
		assert_eq!(config, ObsidianConfig::default());
	}

	#[test]
	fn unsupported_or_invalid_attachment_folder_loads_empty_config() {
		let dir = tempfile::tempdir().unwrap();
		write(
			dir.path(),
			".obsidian/app.json",
			r#"{"attachmentFolderPath":".secret"}"#,
		);
		assert_eq!(ObsidianConfig::load(dir.path()), ObsidianConfig::default());

		write(dir.path(), ".obsidian/app.json", r#"{"attachmentFolderPath":42}"#);
		assert_eq!(ObsidianConfig::load(dir.path()), ObsidianConfig::default());
	}
}
