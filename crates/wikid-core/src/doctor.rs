//! Structural health checks (DESIGN §5). Everything is derived from the
//! files — no LLM, no semantics. Doctor is a report: findings never fail
//! the operation.

use std::collections::{BTreeMap, HashSet};
use std::path::PathBuf;
use std::time::{Duration, SystemTime};

use serde::{Deserialize, Serialize};

use crate::error::WikidError;
use crate::frontmatter::{self, Frontmatter};
use crate::links::{ExtractedLink, LinkIndex, Resolution, extract_links};
use crate::ops::{is_page, read_text};
use crate::vault::Vault;

/// Oversized-page byte threshold (DESIGN §5).
const OVERSIZED_BYTES: u64 = 64 * 1024;
/// Oversized-page line threshold (DESIGN §5).
const OVERSIZED_LINES: usize = 1500;
/// Default staleness threshold in days.
const DEFAULT_STALE_DAYS: u64 = 90;
/// Seconds per day, for staleness arithmetic.
const SECS_PER_DAY: u64 = 24 * 60 * 60;

/// Issue severity.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Severity {
	/// Worth knowing, rarely urgent.
	Low,
	/// Degrades the vault's usefulness.
	Medium,
	/// Actively broken.
	High,
}

/// The eight structural checks, in report order.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Check {
	/// A link resolves to nothing.
	BrokenLinks,
	/// A link's stem or suffix matches more than one file.
	AmbiguousLinks,
	/// A page no other page links to (root `index.md`/`README.md` excluded).
	OrphanPages,
	/// A page without frontmatter in a vault where most pages have it.
	MissingFrontmatter,
	/// A `---` block that fails to parse as YAML.
	MalformedFrontmatter,
	/// A page whose mtime is older than the staleness threshold.
	StalePages,
	/// A page over 64 KiB or 1500 lines.
	OversizedPages,
	/// The same case-insensitive stem at multiple paths.
	DuplicateStems,
}

impl Check {
	/// Every check, in report order.
	pub const ALL: [Check; 8] = [
		Check::BrokenLinks,
		Check::AmbiguousLinks,
		Check::OrphanPages,
		Check::MissingFrontmatter,
		Check::MalformedFrontmatter,
		Check::StalePages,
		Check::OversizedPages,
		Check::DuplicateStems,
	];

	/// The stable snake_case name used in counts, CLI filters, and JSON.
	pub fn name(self) -> &'static str {
		match self {
			Check::BrokenLinks => "broken_links",
			Check::AmbiguousLinks => "ambiguous_links",
			Check::OrphanPages => "orphan_pages",
			Check::MissingFrontmatter => "missing_frontmatter",
			Check::MalformedFrontmatter => "malformed_frontmatter",
			Check::StalePages => "stale_pages",
			Check::OversizedPages => "oversized_pages",
			Check::DuplicateStems => "duplicate_stems",
		}
	}

	/// The fixed severity of this check's issues (DESIGN §5 table).
	pub fn severity(self) -> Severity {
		match self {
			Check::BrokenLinks => Severity::High,
			Check::AmbiguousLinks => Severity::Medium,
			Check::OrphanPages => Severity::Low,
			Check::MissingFrontmatter => Severity::Low,
			Check::MalformedFrontmatter => Severity::Medium,
			Check::StalePages => Severity::Low,
			Check::OversizedPages => Severity::Medium,
			Check::DuplicateStems => Severity::Medium,
		}
	}
}

impl std::str::FromStr for Check {
	type Err = WikidError;

	fn from_str(s: &str) -> Result<Self, Self::Err> {
		Self::ALL
			.iter()
			.copied()
			.find(|check| check.name() == s)
			.ok_or_else(|| WikidError::BadPattern {
				pattern: s.to_string(),
				reason: format!(
					"unknown check; expected one of: {}",
					Self::ALL.map(Check::name).join(", ")
				),
			})
	}
}

/// Options for `doctor`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct DoctorOptions {
	/// Pages not modified in this many days are stale.
	pub stale_days: u64,
	/// Which checks to run; `None` runs all eight.
	pub checks: Option<Vec<Check>>,
}

impl Default for DoctorOptions {
	fn default() -> Self {
		Self {
			stale_days: DEFAULT_STALE_DAYS,
			checks: None,
		}
	}
}

/// One doctor finding.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Issue {
	/// The check that produced this finding.
	pub check: Check,
	/// Fixed per check (DESIGN §5 table).
	pub severity: Severity,
	/// Vault-relative path the finding is about.
	pub path: String,
	/// What is wrong, specifically.
	pub detail: String,
	/// What to do about it.
	pub suggested_action: String,
}

/// Issue totals by severity.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct SeveritySummary {
	/// High-severity findings.
	pub high: usize,
	/// Medium-severity findings.
	pub medium: usize,
	/// Low-severity findings.
	pub low: usize,
}

impl SeveritySummary {
	fn of(issues: &[Issue]) -> Self {
		let mut summary = Self::default();
		for issue in issues {
			match issue.severity {
				Severity::High => summary.high += 1,
				Severity::Medium => summary.medium += 1,
				Severity::Low => summary.low += 1,
			}
		}
		summary
	}
}

/// Result of `doctor`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct HealthReport {
	/// All findings, grouped in check order, sorted by path within a check.
	pub issues: Vec<Issue>,
	/// Findings per executed check, including zero counts.
	pub counts: BTreeMap<String, usize>,
	/// One-line human summary.
	pub summary: String,
}

impl HealthReport {
	/// Issue totals by severity.
	pub fn severity_summary(&self) -> SeveritySummary {
		SeveritySummary::of(&self.issues)
	}
}

/// Everything doctor needs to know about one page, gathered in a single read.
struct PageScan {
	rel: String,
	frontmatter: Frontmatter,
	links: Vec<(ExtractedLink, Resolution)>,
	bytes: u64,
	lines: usize,
	modified: SystemTime,
}

impl Vault {
	/// Runs the structural health checks (DESIGN §5). Attachments and binary
	/// or non-UTF-8 `.md` files are skipped by the content checks; every
	/// visible file participates in `duplicate_stems`.
	pub fn doctor(&self, opts: &DoctorOptions) -> Result<HealthReport, WikidError> {
		let files = self.visible_files()?;
		let index = LinkIndex::build(files.iter().map(|(rel, _)| rel.clone()).collect());
		let mut pages = Vec::new();
		for (rel, abs) in &files {
			if !is_page(rel) {
				continue;
			}
			// IO failures must surface (a silently dropped page would skew
			// every check and the page count); binary/non-UTF-8 pages are
			// deliberately skipped.
			let Some(text) = read_text(abs)? else { continue };
			let meta = std::fs::metadata(abs)?;
			pages.push(PageScan {
				rel: rel.clone(),
				frontmatter: frontmatter::parse(&text),
				links: extract_links(&text)
					.into_iter()
					.map(|link| {
						let resolution = index.resolve(&link.target);
						(link, resolution)
					})
					.collect(),
				bytes: meta.len(),
				lines: text.lines().count(),
				modified: meta.modified()?,
			});
		}
		let enabled: Vec<Check> = match &opts.checks {
			Some(filter) => Check::ALL.iter().copied().filter(|c| filter.contains(c)).collect(),
			None => Check::ALL.to_vec(),
		};
		let mut issues = Vec::new();
		for check in &enabled {
			let mut found = match check {
				Check::BrokenLinks => broken_links(&pages),
				Check::AmbiguousLinks => ambiguous_links(&pages),
				Check::OrphanPages => orphan_pages(&pages),
				Check::MissingFrontmatter => missing_frontmatter(&pages),
				Check::MalformedFrontmatter => malformed_frontmatter(&pages),
				Check::StalePages => stale_pages(&pages, opts.stale_days),
				Check::OversizedPages => oversized_pages(&pages),
				Check::DuplicateStems => duplicate_stems(&files),
			};
			found.sort_by(|a, b| a.path.cmp(&b.path));
			issues.extend(found);
		}
		let mut counts: BTreeMap<String, usize> = enabled.iter().map(|c| (c.name().to_string(), 0)).collect();
		for issue in &issues {
			*counts.get_mut(issue.check.name()).expect("issue from enabled check") += 1;
		}
		let summary = summarize(&issues, pages.len());
		Ok(HealthReport {
			issues,
			counts,
			summary,
		})
	}
}

fn issue(check: Check, path: &str, detail: String, action: &str) -> Issue {
	Issue {
		check,
		severity: check.severity(),
		path: path.to_string(),
		detail,
		suggested_action: action.to_string(),
	}
}

fn summarize(issues: &[Issue], total_pages: usize) -> String {
	if issues.is_empty() {
		return format!("no issues across {total_pages} pages");
	}
	let by = SeveritySummary::of(issues);
	format!(
		"{} issues across {total_pages} pages: {} high, {} medium, {} low",
		issues.len(),
		by.high,
		by.medium,
		by.low
	)
}

fn broken_links(pages: &[PageScan]) -> Vec<Issue> {
	pages
		.iter()
		.flat_map(|page| {
			page.links
				.iter()
				.filter(|(_, resolution)| matches!(resolution, Resolution::Broken))
				.map(|(link, _)| {
					issue(
						Check::BrokenLinks,
						&page.rel,
						format!("{} resolves to nothing", link.raw),
						"create the target page or fix the link",
					)
				})
		})
		.collect()
}

fn ambiguous_links(pages: &[PageScan]) -> Vec<Issue> {
	pages
		.iter()
		.flat_map(|page| {
			page.links.iter().filter_map(|(link, resolution)| match resolution {
				Resolution::Ambiguous(candidates) => Some(issue(
					Check::AmbiguousLinks,
					&page.rel,
					format!(
						"{} matches {} files: {}",
						link.raw,
						candidates.len(),
						candidates.join(", ")
					),
					"qualify the link with its folder, e.g. [[folder/name]]",
				)),
				_ => None,
			})
		})
		.collect()
}

fn orphan_pages(pages: &[PageScan]) -> Vec<Issue> {
	let linked: HashSet<&str> = pages
		.iter()
		.flat_map(|page| {
			page.links.iter().filter_map(move |(_, resolution)| match resolution {
				// A page linking to itself does not rescue it from orphanhood.
				Resolution::Resolved(target) if *target != page.rel => Some(target.as_str()),
				_ => None,
			})
		})
		.collect();
	pages
		.iter()
		.filter(|page| !linked.contains(page.rel.as_str()) && !is_root_index(&page.rel))
		.map(|page| {
			issue(
				Check::OrphanPages,
				&page.rel,
				"no other page links here".to_string(),
				"link it from a hub page or archive it",
			)
		})
		.collect()
}

/// Root-level `index.md`/`README.md` are entry points, never orphans.
fn is_root_index(rel: &str) -> bool {
	!rel.contains('/') && (rel.eq_ignore_ascii_case("index.md") || rel.eq_ignore_ascii_case("readme.md"))
}

fn missing_frontmatter(pages: &[PageScan]) -> Vec<Issue> {
	// Only meaningful when the vault "uses" frontmatter: at least half of the
	// pages carry a block (well-formed or not).
	let with_block = pages
		.iter()
		.filter(|page| !matches!(page.frontmatter, Frontmatter::Absent))
		.count();
	if pages.is_empty() || with_block * 2 < pages.len() {
		return Vec::new();
	}
	pages
		.iter()
		.filter(|page| matches!(page.frontmatter, Frontmatter::Absent))
		.map(|page| {
			issue(
				Check::MissingFrontmatter,
				&page.rel,
				"page has no frontmatter while most pages do".to_string(),
				"add a leading --- YAML block",
			)
		})
		.collect()
}

fn malformed_frontmatter(pages: &[PageScan]) -> Vec<Issue> {
	pages
		.iter()
		.filter(|page| page.frontmatter.is_malformed())
		.map(|page| {
			issue(
				Check::MalformedFrontmatter,
				&page.rel,
				"frontmatter block is not valid YAML".to_string(),
				"fix the YAML between the --- markers or remove the block",
			)
		})
		.collect()
}

fn stale_pages(pages: &[PageScan], stale_days: u64) -> Vec<Issue> {
	let now = SystemTime::now();
	pages
		.iter()
		.filter_map(|page| {
			let age = now.duration_since(page.modified).ok()?;
			if age < Duration::from_secs(stale_days.saturating_mul(SECS_PER_DAY)) {
				return None;
			}
			let days = age.as_secs() / SECS_PER_DAY;
			Some(issue(
				Check::StalePages,
				&page.rel,
				format!("not modified in {days} days (threshold: {stale_days})"),
				"review the content, then update or archive it",
			))
		})
		.collect()
}

fn oversized_pages(pages: &[PageScan]) -> Vec<Issue> {
	pages
		.iter()
		.filter(|page| page.bytes > OVERSIZED_BYTES || page.lines > OVERSIZED_LINES)
		.map(|page| {
			issue(
				Check::OversizedPages,
				&page.rel,
				format!(
					"{} KiB / {} lines (limits: 64 KiB / 1500 lines)",
					page.bytes / 1024,
					page.lines
				),
				"split the page into smaller focused pages",
			)
		})
		.collect()
}

fn duplicate_stems(files: &[(String, PathBuf)]) -> Vec<Issue> {
	let mut by_stem: BTreeMap<String, Vec<&str>> = BTreeMap::new();
	for (rel, _) in files {
		let name = rel.rsplit('/').next().unwrap_or(rel);
		let stem = name.rsplit_once('.').map_or(name, |(stem, _)| stem);
		by_stem.entry(stem.to_lowercase()).or_default().push(rel);
	}
	by_stem
		.into_iter()
		.filter(|(_, paths)| paths.len() > 1)
		.map(|(stem, mut paths)| {
			paths.sort_unstable();
			issue(
				Check::DuplicateStems,
				paths[0],
				format!("stem '{stem}' is shared by {}", paths.join(", ")),
				"rename one file so wikilink stems stay unique",
			)
		})
		.collect()
}

#[cfg(test)]
mod tests {
	use super::*;
	use crate::test_fixtures;

	fn counts_of(report: &HealthReport) -> BTreeMap<&str, usize> {
		report.counts.iter().map(|(k, v)| (k.as_str(), *v)).collect()
	}

	fn issue_paths(report: &HealthReport, check: Check) -> Vec<&str> {
		report
			.issues
			.iter()
			.filter(|i| i.check == check)
			.map(|i| i.path.as_str())
			.collect()
	}

	#[test]
	fn every_check_fires_on_the_knowledge_vault() {
		let (_dir, vault) = test_fixtures::knowledge_vault();
		let report = vault.doctor(&DoctorOptions::default()).unwrap();
		let expected: BTreeMap<&str, usize> = [
			("broken_links", 1),
			("ambiguous_links", 1),
			("orphan_pages", 3),
			("missing_frontmatter", 2),
			("malformed_frontmatter", 1),
			("stale_pages", 1),
			("oversized_pages", 1),
			("duplicate_stems", 1),
		]
		.into_iter()
		.collect();
		assert_eq!(counts_of(&report), expected);
		assert_eq!(report.issues.len(), 11);
		assert_eq!(
			report.severity_summary(),
			SeveritySummary {
				high: 1,
				medium: 4,
				low: 6
			}
		);
		assert_eq!(report.summary, "11 issues across 10 pages: 1 high, 4 medium, 6 low");
	}

	#[test]
	fn issue_fields_carry_check_severity_and_action() {
		let (_dir, vault) = test_fixtures::knowledge_vault();
		let report = vault.doctor(&DoctorOptions::default()).unwrap();
		let broken = report
			.issues
			.iter()
			.find(|i| i.check == Check::BrokenLinks)
			.expect("broken link issue");
		assert_eq!(broken.severity, Severity::High);
		assert_eq!(broken.path, "index.md");
		assert!(broken.detail.contains("[[missing-page]]"), "got {}", broken.detail);
		assert!(!broken.suggested_action.is_empty());

		let ambiguous = report
			.issues
			.iter()
			.find(|i| i.check == Check::AmbiguousLinks)
			.expect("ambiguous link issue");
		assert_eq!(ambiguous.severity, Severity::Medium);
		assert!(
			ambiguous.detail.contains("notes/todo.md, projects/todo.md"),
			"got {}",
			ambiguous.detail
		);
	}

	#[test]
	fn no_check_fires_on_a_clean_vault() {
		// The base ops fixture is clean for seven of the eight checks; the
		// orphan (notes/unicode.md) is covered separately below.
		let (_dir, vault) = test_fixtures::vault();
		let report = vault.doctor(&DoctorOptions::default()).unwrap();
		for check in [
			"broken_links",
			"ambiguous_links",
			"missing_frontmatter",
			"malformed_frontmatter",
			"stale_pages",
			"oversized_pages",
			"duplicate_stems",
		] {
			assert_eq!(report.counts[check], 0, "{check} fired unexpectedly");
		}
	}

	#[test]
	fn orphan_pages_fire_and_root_entry_points_are_exempt() {
		let (_dir, vault) = test_fixtures::knowledge_vault();
		let report = vault.doctor(&DoctorOptions::default()).unwrap();
		// index.md has no inbound links but is the root entry point.
		assert_eq!(
			issue_paths(&report, Check::OrphanPages),
			vec!["notes/todo.md", "orphan.md", "projects/todo.md"]
		);
	}

	#[test]
	fn orphan_pages_do_not_fire_when_everything_is_linked() {
		let dir = tempfile::tempdir().unwrap();
		std::fs::write(dir.path().join("index.md"), "[[a]]\n").unwrap();
		std::fs::write(dir.path().join("a.md"), "back to [[index]]\n").unwrap();
		std::fs::write(dir.path().join("README.md"), "unlinked but exempt\n").unwrap();
		let vault = Vault::open(dir.path()).unwrap();
		let report = vault.doctor(&DoctorOptions::default()).unwrap();
		assert_eq!(report.counts["orphan_pages"], 0, "issues: {:?}", report.issues);
	}

	#[test]
	fn nested_index_pages_are_not_exempt_from_orphanhood() {
		let dir = tempfile::tempdir().unwrap();
		std::fs::create_dir(dir.path().join("sub")).unwrap();
		std::fs::write(dir.path().join("sub/index.md"), "# Sub\n").unwrap();
		let vault = Vault::open(dir.path()).unwrap();
		let report = vault.doctor(&DoctorOptions::default()).unwrap();
		assert_eq!(issue_paths(&report, Check::OrphanPages), vec!["sub/index.md"]);
	}

	#[test]
	fn missing_frontmatter_respects_the_adoption_gate() {
		// The base fixture has 1 of 4 pages with frontmatter: below 50%, so
		// the vault does not "use" frontmatter and the check stays silent.
		let (_dir, vault) = test_fixtures::vault();
		let report = vault.doctor(&DoctorOptions::default()).unwrap();
		assert_eq!(report.counts["missing_frontmatter"], 0);
		// The knowledge vault is above the gate: absent pages are flagged.
		let (_dir2, vault2) = test_fixtures::knowledge_vault();
		let report2 = vault2.doctor(&DoctorOptions::default()).unwrap();
		assert_eq!(
			issue_paths(&report2, Check::MissingFrontmatter),
			vec!["orphan.md", "projects/beta.md"]
		);
	}

	#[test]
	fn missing_frontmatter_fires_at_exactly_half_adoption() {
		let dir = tempfile::tempdir().unwrap();
		std::fs::write(dir.path().join("a.md"), "---\ntitle: A\n---\n[[b]] [[c]] [[d]]\n").unwrap();
		std::fs::write(dir.path().join("b.md"), "---\ntitle: B\n---\n[[a]]\n").unwrap();
		std::fs::write(dir.path().join("c.md"), "no frontmatter\n").unwrap();
		std::fs::write(dir.path().join("d.md"), "no frontmatter\n").unwrap();
		let vault = Vault::open(dir.path()).unwrap();
		let report = vault.doctor(&DoctorOptions::default()).unwrap();
		assert_eq!(issue_paths(&report, Check::MissingFrontmatter), vec!["c.md", "d.md"]);
	}

	#[test]
	fn stale_pages_respect_the_stale_days_option() {
		let (_dir, vault) = test_fixtures::knowledge_vault();
		// stale.md is backdated 200 days: stale at the default 90 …
		let default = vault.doctor(&DoctorOptions::default()).unwrap();
		assert_eq!(issue_paths(&default, Check::StalePages), vec!["stale.md"]);
		let issue = default.issues.iter().find(|i| i.check == Check::StalePages).unwrap();
		assert!(issue.detail.contains("200 days"), "got {}", issue.detail);
		// … but fresh under a 400-day threshold.
		let relaxed = vault
			.doctor(&DoctorOptions {
				stale_days: 400,
				checks: None,
			})
			.unwrap();
		assert_eq!(relaxed.counts["stale_pages"], 0);
	}

	#[test]
	fn huge_stale_days_saturates_instead_of_overflowing() {
		// u64::MAX days * 86400 would overflow; the threshold must saturate.
		let (_dir, vault) = test_fixtures::knowledge_vault();
		let report = vault
			.doctor(&DoctorOptions {
				stale_days: u64::MAX,
				checks: Some(vec![Check::StalePages]),
			})
			.unwrap();
		assert_eq!(report.counts["stale_pages"], 0);
	}

	#[test]
	fn oversized_pages_fire_on_line_count_and_byte_size() {
		let (_dir, vault) = test_fixtures::knowledge_vault();
		let report = vault.doctor(&DoctorOptions::default()).unwrap();
		// big.md exceeds 1500 lines while staying under 64 KiB.
		assert_eq!(issue_paths(&report, Check::OversizedPages), vec!["big.md"]);
		// A single-line page over 64 KiB fires on bytes.
		vault.write("huge.md", &"x".repeat(65 * 1024)).unwrap();
		let report = vault.doctor(&DoctorOptions::default()).unwrap();
		assert!(issue_paths(&report, Check::OversizedPages).contains(&"huge.md"));
	}

	#[test]
	fn duplicate_stems_report_one_issue_per_group() {
		let (_dir, vault) = test_fixtures::knowledge_vault();
		let report = vault.doctor(&DoctorOptions::default()).unwrap();
		let dup = report
			.issues
			.iter()
			.find(|i| i.check == Check::DuplicateStems)
			.expect("duplicate stem issue");
		assert_eq!(dup.path, "notes/todo.md");
		assert!(
			dup.detail.contains("notes/todo.md, projects/todo.md"),
			"got {}",
			dup.detail
		);
		// Stems compare case-insensitively.
		vault.write("TODO.md", "# shouting\n").unwrap();
		let report = vault.doctor(&DoctorOptions::default()).unwrap();
		let dup = report.issues.iter().find(|i| i.check == Check::DuplicateStems).unwrap();
		assert!(dup.detail.contains("TODO.md"), "got {}", dup.detail);
	}

	#[test]
	fn binary_attachments_are_invisible_to_content_checks() {
		// logo.png contains wikilink-looking bytes; they must not surface.
		let (_dir, vault) = test_fixtures::knowledge_vault();
		let report = vault.doctor(&DoctorOptions::default()).unwrap();
		assert_eq!(report.counts["broken_links"], 1);
		assert!(report.issues.iter().all(|i| !i.path.contains("logo.png")));
	}

	#[cfg(unix)]
	#[test]
	fn doctor_surfaces_unreadable_pages_as_io_errors() {
		use std::os::unix::fs::PermissionsExt;
		let (dir, vault) = test_fixtures::vault();
		let page = dir.path().join("projects/alpha.md");
		std::fs::set_permissions(&page, std::fs::Permissions::from_mode(0o000)).unwrap();
		let err = vault.doctor(&DoctorOptions::default()).unwrap_err();
		assert!(matches!(err, WikidError::Io(_)), "got {err:?}");
		std::fs::set_permissions(&page, std::fs::Permissions::from_mode(0o644)).unwrap();
	}

	#[cfg(unix)]
	#[test]
	fn doctor_ignores_symlinks_out_of_the_vault() {
		let outside = tempfile::tempdir().unwrap();
		std::fs::write(outside.path().join("secret.md"), "[[no-such-page]]\n").unwrap();
		let (dir, vault) = test_fixtures::vault();
		std::os::unix::fs::symlink(outside.path().join("secret.md"), dir.path().join("escape.md")).unwrap();
		let report = vault.doctor(&DoctorOptions::default()).unwrap();
		assert_eq!(report.counts["broken_links"], 0);
		assert!(
			report.issues.iter().all(|i| i.path != "escape.md"),
			"{:?}",
			report.issues
		);
	}

	#[test]
	fn checks_filter_limits_execution_and_counts() {
		let (_dir, vault) = test_fixtures::knowledge_vault();
		let report = vault
			.doctor(&DoctorOptions {
				stale_days: DEFAULT_STALE_DAYS,
				checks: Some(vec![Check::BrokenLinks, Check::StalePages]),
			})
			.unwrap();
		let expected: BTreeMap<&str, usize> = [("broken_links", 1), ("stale_pages", 1)].into_iter().collect();
		assert_eq!(counts_of(&report), expected);
		assert!(
			report
				.issues
				.iter()
				.all(|i| matches!(i.check, Check::BrokenLinks | Check::StalePages))
		);
		assert_eq!(report.summary, "2 issues across 10 pages: 1 high, 0 medium, 1 low");
	}

	#[test]
	fn empty_vault_reports_no_issues() {
		let dir = tempfile::tempdir().unwrap();
		let vault = Vault::open(dir.path()).unwrap();
		let report = vault.doctor(&DoctorOptions::default()).unwrap();
		assert!(report.issues.is_empty());
		assert_eq!(report.counts.len(), 8);
		assert_eq!(report.summary, "no issues across 0 pages");
	}

	#[test]
	fn check_names_parse_and_reject_unknowns() {
		use std::str::FromStr;
		for check in Check::ALL {
			assert_eq!(Check::from_str(check.name()).unwrap(), check);
		}
		let err = Check::from_str("nope").unwrap_err();
		assert!(matches!(err, WikidError::BadPattern { .. }));
		assert!(err.to_string().contains("broken_links"));
	}

	#[test]
	fn report_round_trips_as_json_with_stable_names() {
		let (_dir, vault) = test_fixtures::knowledge_vault();
		let report = vault.doctor(&DoctorOptions::default()).unwrap();
		let json = serde_json::to_string(&report).unwrap();
		assert!(json.contains("\"check\":\"broken_links\""));
		assert!(json.contains("\"severity\":\"high\""));
		let back: HealthReport = serde_json::from_str(&json).unwrap();
		assert_eq!(report, back);
	}
}
