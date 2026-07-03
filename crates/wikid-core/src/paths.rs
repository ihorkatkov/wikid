//! Path rules and safety (DESIGN §2). User-facing paths are always relative
//! to the vault root with forward slashes. Validation is lexical first (no
//! filesystem access needed to reject an escape), then a defensive
//! canonicalize-containment check applies to whatever already exists on disk.

use std::path::{Component, Path, PathBuf};

use crate::error::WikidError;

/// A validated operation target: the normalized vault-relative path and its
/// absolute location on disk.
#[derive(Debug, Clone)]
pub(crate) struct Resolved {
	/// Normalized vault-relative path, forward slashes, no `.` / `..`.
	pub rel: String,
	/// Absolute path under the (canonicalized) vault root.
	pub abs: PathBuf,
}

fn invalid(path: &str, reason: &str) -> WikidError {
	WikidError::InvalidPath {
		path: path.to_string(),
		reason: reason.to_string(),
	}
}

/// Lexically normalizes a user path into its components: `.` is dropped,
/// `..` pops the previous component. Rejects empty and absolute paths, and
/// any path whose normalized form escapes the root or is the root itself.
pub(crate) fn normalize(path: &str) -> Result<Vec<String>, WikidError> {
	if path.trim().is_empty() {
		return Err(invalid(path, "empty path"));
	}
	let raw = Path::new(path);
	if raw.is_absolute() {
		return Err(invalid(path, "absolute paths are not allowed"));
	}
	let mut stack: Vec<String> = Vec::new();
	for component in raw.components() {
		match component {
			Component::CurDir => {}
			Component::ParentDir => {
				if stack.pop().is_none() {
					return Err(invalid(path, "path escapes the vault root"));
				}
			}
			Component::Normal(part) => stack.push(part.to_string_lossy().into_owned()),
			Component::RootDir | Component::Prefix(_) => {
				return Err(invalid(path, "absolute paths are not allowed"));
			}
		}
	}
	if stack.is_empty() {
		return Err(invalid(path, "path resolves to the vault root itself"));
	}
	Ok(stack)
}

/// Validates a user path against a vault root: normalizes it, refuses hidden
/// (dot-prefixed) components, and containment-checks the result.
pub(crate) fn resolve(root: &Path, path: &str) -> Result<Resolved, WikidError> {
	let components = normalize(path)?;
	if let Some(hidden) = components.iter().find(|c| c.starts_with('.')) {
		return Err(invalid(path, &format!("hidden component: {hidden}")));
	}
	let rel = components.join("/");
	let abs = root.join(&rel);
	ensure_contained(root, &abs, path)?;
	Ok(Resolved { rel, abs })
}

/// Defensive check for existing targets: canonicalize the deepest existing
/// ancestor of `abs` and refuse anything resolving outside the vault root
/// (i.e. symlinks pointing out of the vault).
fn ensure_contained(root: &Path, abs: &Path, original: &str) -> Result<(), WikidError> {
	let mut probe = abs.to_path_buf();
	loop {
		if probe.exists() {
			let canonical = probe.canonicalize()?;
			if !canonical.starts_with(root) {
				return Err(invalid(original, "resolves outside the vault root"));
			}
			return Ok(());
		}
		// Walk up to the deepest existing ancestor; the loop always terminates
		// because the canonicalized root itself exists.
		if !probe.pop() || !probe.starts_with(root) {
			return Ok(());
		}
	}
}

#[cfg(test)]
mod tests {
	use super::*;
	use crate::error::WikidError;

	fn assert_invalid(result: Result<Vec<String>, WikidError>) {
		assert!(
			matches!(result, Err(WikidError::InvalidPath { .. })),
			"expected InvalidPath, got {result:?}"
		);
	}

	#[test]
	fn normalize_accepts_plain_relative_paths() {
		assert_eq!(normalize("projects/alpha.md").unwrap(), vec!["projects", "alpha.md"]);
		assert_eq!(normalize("a/./b").unwrap(), vec!["a", "b"]);
		assert_eq!(normalize("a//b").unwrap(), vec!["a", "b"]);
		assert_eq!(normalize("a/../b").unwrap(), vec!["b"]);
	}

	#[test]
	fn normalize_rejects_escapes_and_absolutes() {
		assert_invalid(normalize("../x"));
		assert_invalid(normalize("a/../../b"));
		assert_invalid(normalize("/etc/passwd"));
		assert_invalid(normalize(""));
		assert_invalid(normalize("  "));
		assert_invalid(normalize("."));
		assert_invalid(normalize("a/.."));
	}

	#[test]
	fn resolve_rejects_hidden_components() {
		let dir = tempfile::tempdir().unwrap();
		let root = dir.path().canonicalize().unwrap();
		assert!(matches!(
			resolve(&root, ".obsidian/app.json"),
			Err(WikidError::InvalidPath { .. })
		));
		assert!(matches!(
			resolve(&root, "notes/.hidden/x.md"),
			Err(WikidError::InvalidPath { .. })
		));
		// A dot inside a name is not a hidden component.
		assert!(resolve(&root, "notes/v1.2.md").is_ok());
	}

	#[test]
	fn resolve_produces_forward_slash_rel_and_contained_abs() {
		let dir = tempfile::tempdir().unwrap();
		let root = dir.path().canonicalize().unwrap();
		let resolved = resolve(&root, "projects/../notes/x.md").unwrap();
		assert_eq!(resolved.rel, "notes/x.md");
		assert!(resolved.abs.starts_with(&root));
	}

	#[cfg(unix)]
	#[test]
	fn resolve_refuses_symlinks_out_of_the_vault() {
		let outside = tempfile::tempdir().unwrap();
		std::fs::write(outside.path().join("secret.txt"), "top secret").unwrap();
		let dir = tempfile::tempdir().unwrap();
		let root = dir.path().canonicalize().unwrap();
		// Symlinked file pointing outside.
		std::os::unix::fs::symlink(outside.path().join("secret.txt"), root.join("escape.md")).unwrap();
		assert!(matches!(
			resolve(&root, "escape.md"),
			Err(WikidError::InvalidPath { .. })
		));
		// Symlinked directory pointing outside: even a not-yet-existing target
		// beneath it is refused (deepest existing ancestor is checked).
		std::os::unix::fs::symlink(outside.path(), root.join("linkdir")).unwrap();
		assert!(matches!(
			resolve(&root, "linkdir/new.md"),
			Err(WikidError::InvalidPath { .. })
		));
		// Symlinks staying inside the vault are fine.
		std::fs::write(root.join("real.md"), "# Real").unwrap();
		std::os::unix::fs::symlink(root.join("real.md"), root.join("alias.md")).unwrap();
		assert!(resolve(&root, "alias.md").is_ok());
	}
}
