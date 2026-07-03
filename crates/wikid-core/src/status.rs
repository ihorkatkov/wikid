//! Vault status aggregation (DESIGN §3): the no-argument default view —
//! live totals, the most recently modified pages, and a doctor severity
//! summary.

use std::fs;

use serde::{Deserialize, Serialize};

use crate::doctor::{DoctorOptions, SeveritySummary};
use crate::error::WikidError;
use crate::ops::{is_page, rfc3339};
use crate::vault::Vault;

/// How many recently modified pages `status` reports.
const RECENT_PAGES: usize = 5;

/// One recently modified page.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct RecentPage {
	/// Vault-relative path.
	pub path: String,
	/// Last modification time, RFC3339 UTC.
	pub modified: String,
}

/// Result of `status`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct VaultStatus {
	/// Absolute vault root on disk.
	pub root: String,
	/// Visible Markdown pages.
	pub total_pages: usize,
	/// Visible non-page files (attachments).
	pub total_files: usize,
	/// Total byte size of all visible files.
	pub total_bytes: u64,
	/// The 5 most recently modified pages, newest first.
	pub recent: Vec<RecentPage>,
	/// Doctor issue totals at default options.
	pub doctor_summary: SeveritySummary,
}

impl Vault {
	/// Aggregates the vault's live status (DESIGN §3): totals over visible
	/// files, the most recent pages, and a default-options doctor summary.
	pub fn status(&self) -> Result<VaultStatus, WikidError> {
		let mut total_pages = 0usize;
		let mut total_files = 0usize;
		let mut total_bytes = 0u64;
		let mut pages: Vec<(String, std::time::SystemTime)> = Vec::new();
		for (rel, abs) in self.visible_files()? {
			let meta = fs::metadata(&abs)?;
			total_bytes += meta.len();
			if is_page(&rel) {
				total_pages += 1;
				pages.push((rel, meta.modified()?));
			} else {
				total_files += 1;
			}
		}
		pages.sort_by(|a, b| b.1.cmp(&a.1).then_with(|| a.0.cmp(&b.0)));
		let recent = pages
			.into_iter()
			.take(RECENT_PAGES)
			.map(|(path, modified)| RecentPage {
				path,
				modified: rfc3339(modified),
			})
			.collect();
		let doctor_summary = self.doctor(&DoctorOptions::default())?.severity_summary();
		Ok(VaultStatus {
			root: self.root().display().to_string(),
			total_pages,
			total_files,
			total_bytes,
			recent,
			doctor_summary,
		})
	}
}

#[cfg(test)]
mod tests {
	use super::*;
	use crate::test_fixtures;

	#[test]
	fn status_counts_visible_pages_files_and_bytes() {
		let (dir, vault) = test_fixtures::vault();
		let status = vault.status().unwrap();
		assert_eq!(status.root, dir.path().canonicalize().unwrap().display().to_string());
		// 4 pages + 2 attachments; hidden and gitignored files don't count.
		assert_eq!(status.total_pages, 4);
		assert_eq!(status.total_files, 2);
		let expected_bytes: u64 = [
			"index.md",
			"projects/alpha.md",
			"projects/beta.md",
			"notes/unicode.md",
			"attachments/logo.png",
			"attachments/data.bin",
		]
		.iter()
		.map(|rel| std::fs::metadata(dir.path().join(rel)).unwrap().len())
		.sum();
		assert_eq!(status.total_bytes, expected_bytes);
	}

	#[test]
	fn status_recent_lists_pages_newest_first_capped_at_five() {
		let (_dir, vault) = test_fixtures::knowledge_vault();
		let status = vault.status().unwrap();
		assert_eq!(status.total_pages, 10);
		assert_eq!(status.total_files, 1);
		assert_eq!(status.recent.len(), 5);
		// The backdated stale page cannot be among the five most recent.
		assert!(status.recent.iter().all(|p| p.path != "stale.md"));
		// Entries are pages with RFC3339 UTC timestamps, newest first.
		for pair in status.recent.windows(2) {
			assert!(pair[0].modified >= pair[1].modified, "not newest first: {pair:?}");
		}
		for page in &status.recent {
			assert!(page.path.ends_with(".md"), "non-page in recent: {}", page.path);
			assert!(page.modified.ends_with('Z') && page.modified.contains('T'));
		}
	}

	#[test]
	fn status_recent_is_shorter_when_the_vault_has_fewer_pages() {
		let (_dir, vault) = test_fixtures::vault();
		let status = vault.status().unwrap();
		assert_eq!(status.recent.len(), 4);
	}

	#[test]
	fn status_carries_the_doctor_severity_summary() {
		let (_dir, vault) = test_fixtures::knowledge_vault();
		let status = vault.status().unwrap();
		assert_eq!(
			status.doctor_summary,
			SeveritySummary {
				high: 1,
				medium: 4,
				low: 6
			}
		);
	}

	#[test]
	fn status_of_an_empty_vault_is_all_zeroes() {
		let dir = tempfile::tempdir().unwrap();
		let vault = crate::Vault::open(dir.path()).unwrap();
		let status = vault.status().unwrap();
		assert_eq!(status.total_pages, 0);
		assert_eq!(status.total_files, 0);
		assert_eq!(status.total_bytes, 0);
		assert!(status.recent.is_empty());
		assert_eq!(status.doctor_summary, SeveritySummary::default());
	}

	#[test]
	fn status_round_trips_as_json() {
		let (_dir, vault) = test_fixtures::knowledge_vault();
		let status = vault.status().unwrap();
		let json = serde_json::to_string(&status).unwrap();
		assert!(json.contains("\"doctor_summary\""));
		let back: VaultStatus = serde_json::from_str(&json).unwrap();
		assert_eq!(status, back);
	}
}
