//! Structural health checks (DESIGN §5). Everything is derived from the
//! files — no LLM, no semantics. Doctor is a report: findings never fail
//! the operation.

use std::collections::{BTreeMap, HashSet};
use std::path::PathBuf;
use std::time::{Duration, SystemTime};

use serde::{Deserialize, Serialize};

use crate::error::WikidError;
use crate::frontmatter::{self, Frontmatter};
use crate::links::{ExtractedLink, LinkIndex, Resolution, block_anchors, extract_links};
use crate::markdown::FenceTracker;
use crate::obsidian_config::ObsidianConfig;
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

/// Built-in doctor policy profile.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum DoctorProfile {
	/// Opinionated defaults for LLM wikis: authored pages stay actionable,
	/// source captures/assets/meta pages are downgraded or ignored.
	#[default]
	#[serde(alias = "llm-wiki")]
	LlmWiki,
	/// Raw structural lint with no LLM-wiki suppressions.
	Strict,
}

impl DoctorProfile {
	pub fn name(self) -> &'static str {
		match self {
			Self::LlmWiki => "llm-wiki",
			Self::Strict => "strict",
		}
	}
}

impl std::str::FromStr for DoctorProfile {
	type Err = WikidError;

	fn from_str(s: &str) -> Result<Self, Self::Err> {
		match s {
			"llm-wiki" | "llm_wiki" | "default" => Ok(Self::LlmWiki),
			"strict" => Ok(Self::Strict),
			_ => Err(WikidError::BadPattern {
				pattern: s.to_string(),
				reason: "unknown doctor profile; expected llm-wiki or strict".to_string(),
			}),
		}
	}
}

/// Human-oriented grouping for doctor findings.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum IssueCategory {
	/// Authored wiki pages where findings are normally actionable.
	AuthoredPages,
	/// Raw source captures and extraction artifacts.
	RawSource,
	/// Attachment/asset hygiene.
	AssetHygiene,
	/// Link graph and navigation structure.
	GraphNavigation,
	/// Size and freshness warnings.
	SizePerformance,
}

/// The structural checks, in report order.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Check {
	/// A link resolves to nothing.
	BrokenLinks,
	/// A link's stem, suffix, or alias matches more than one file.
	AmbiguousLinks,
	/// The same case-insensitive frontmatter alias appears on multiple pages.
	DuplicateAliases,
	/// A page no other page links to (root `index.md`/`README.md` excluded).
	OrphanPages,
	/// A `#^block-id` fragment points at a page without that block anchor.
	BrokenBlockReference,
	/// A `#heading` fragment points at a page without that ATX heading.
	BrokenHeadingReference,
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
	pub const ALL: [Check; 11] = [
		Check::BrokenLinks,
		Check::AmbiguousLinks,
		Check::DuplicateAliases,
		Check::OrphanPages,
		Check::BrokenBlockReference,
		Check::BrokenHeadingReference,
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
			Check::DuplicateAliases => "duplicate_aliases",
			Check::OrphanPages => "orphan_pages",
			Check::BrokenBlockReference => "broken_block_reference",
			Check::BrokenHeadingReference => "broken_heading_reference",
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
			Check::DuplicateAliases => Severity::Low,
			Check::OrphanPages => Severity::Low,
			Check::BrokenBlockReference => Severity::Medium,
			Check::BrokenHeadingReference => Severity::Medium,
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
	/// Which checks to run; `None` runs all checks.
	pub checks: Option<Vec<Check>>,
	/// Lint policy profile.
	#[serde(default)]
	pub profile: DoctorProfile,
}

impl Default for DoctorOptions {
	fn default() -> Self {
		Self {
			stale_days: DEFAULT_STALE_DAYS,
			checks: None,
			profile: DoctorProfile::default(),
		}
	}
}

/// One doctor finding.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Issue {
	/// The check that produced this finding.
	pub check: Check,
	/// Per-check severity, optionally adjusted by the active doctor profile.
	pub severity: Severity,
	/// Output grouping for humans and agents.
	pub category: IssueCategory,
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
#[derive(Clone)]
struct PageScan {
	rel: String,
	frontmatter: Frontmatter,
	links: Vec<(ExtractedLink, Resolution)>,
	bytes: u64,
	lines: usize,
	modified: SystemTime,
	headings: Vec<String>,
	block_anchors: Vec<String>,
}

impl Vault {
	/// Runs the structural health checks (DESIGN §5). Attachments and binary
	/// or non-UTF-8 `.md` files are skipped by the content checks; every
	/// visible file participates in `duplicate_stems`.
	pub fn doctor(&self, opts: &DoctorOptions) -> Result<HealthReport, WikidError> {
		let files = self.visible_files()?;
		let config = ObsidianConfig::load(self.root());
		let mut page_inputs = Vec::new();
		let mut aliases = Vec::new();
		for (i, (rel, abs)) in files.iter().enumerate() {
			if !is_page(rel) {
				continue;
			}
			// IO failures must surface (a silently dropped page would skew
			// every check and the page count); binary/non-UTF-8 pages are
			// deliberately skipped.
			let Some(text) = read_text(abs)? else { continue };
			let frontmatter = frontmatter::parse(&text);
			let page_aliases = frontmatter::aliases(&frontmatter);
			if !page_aliases.is_empty() {
				aliases.push((i, page_aliases));
			}
			let meta = std::fs::metadata(abs)?;
			page_inputs.push((
				rel.clone(),
				frontmatter,
				extract_links(&text),
				meta.len(),
				text.lines().count(),
				meta.modified()?,
				extract_atx_headings(&text),
				block_anchors(&text),
			));
		}
		let index = LinkIndex::build(
			files.iter().map(|(rel, _)| rel.clone()).collect(),
			&aliases,
			config.attachment_folder,
		);
		let pages: Vec<PageScan> = page_inputs
			.into_iter()
			.map(
				|(rel, frontmatter, extracted, bytes, lines, modified, headings, block_anchors)| {
					let links = extracted
						.into_iter()
						.map(|link| {
							let resolution = index.resolve_from(&rel, &link);
							(link, resolution)
						})
						.collect();
					PageScan {
						rel,
						frontmatter,
						links,
						bytes,
						lines,
						modified,
						headings,
						block_anchors,
					}
				},
			)
			.collect();
		let enabled: Vec<Check> = match &opts.checks {
			Some(filter) => Check::ALL.iter().copied().filter(|c| filter.contains(c)).collect(),
			None => Check::ALL.to_vec(),
		};
		let use_llm_scope = opts.profile == DoctorProfile::LlmWiki && has_llm_wiki_layout(&pages);
		let scoped_pages: Vec<PageScan> = if use_llm_scope {
			pages
				.iter()
				.filter(|page| is_llm_wiki_linted_page(&page.rel))
				.cloned()
				.collect()
		} else {
			pages.clone()
		};
		let mut issues = Vec::new();
		for check in &enabled {
			let mut found = match check {
				Check::BrokenLinks => broken_links(&scoped_pages, opts.profile),
				Check::AmbiguousLinks => ambiguous_links(&scoped_pages, &duplicate_alias_paths(&scoped_pages)),
				Check::DuplicateAliases => duplicate_aliases(&scoped_pages),
				Check::OrphanPages => orphan_pages(&scoped_pages),
				Check::BrokenBlockReference => broken_block_references(&scoped_pages, &pages),
				Check::BrokenHeadingReference => broken_heading_references(&scoped_pages, &pages),
				Check::MissingFrontmatter => missing_frontmatter(&scoped_pages, opts.profile),
				Check::MalformedFrontmatter => malformed_frontmatter(&scoped_pages),
				Check::StalePages => stale_pages(&scoped_pages, opts.stale_days),
				Check::OversizedPages => oversized_pages(&scoped_pages),
				Check::DuplicateStems => duplicate_stems(&files, opts.profile, use_llm_scope),
			};
			found.sort_by(|a, b| a.path.cmp(&b.path));
			issues.extend(found);
		}
		let mut counts: BTreeMap<String, usize> = enabled.iter().map(|c| (c.name().to_string(), 0)).collect();
		for issue in &issues {
			*counts.get_mut(issue.check.name()).expect("issue from enabled check") += 1;
		}
		let summary = summarize(&issues, scoped_pages.len());
		Ok(HealthReport {
			issues,
			counts,
			summary,
		})
	}
}

fn issue(check: Check, path: &str, detail: String, action: &str) -> Issue {
	issue_with_severity(check, check.severity(), path, detail, action)
}

fn issue_with_severity(check: Check, severity: Severity, path: &str, detail: String, action: &str) -> Issue {
	Issue {
		check,
		severity,
		category: category_for(check, path),
		path: path.to_string(),
		detail,
		suggested_action: action.to_string(),
	}
}

fn category_for(check: Check, path: &str) -> IssueCategory {
	if is_asset_path(path) {
		return IssueCategory::AssetHygiene;
	}
	if is_raw_path(path) {
		return IssueCategory::RawSource;
	}
	if is_authored_page(path) {
		return IssueCategory::AuthoredPages;
	}
	match check {
		Check::BrokenLinks
		| Check::AmbiguousLinks
		| Check::DuplicateAliases
		| Check::OrphanPages
		| Check::BrokenBlockReference
		| Check::BrokenHeadingReference
		| Check::DuplicateStems => IssueCategory::GraphNavigation,
		Check::StalePages | Check::OversizedPages => IssueCategory::SizePerformance,
		Check::MissingFrontmatter | Check::MalformedFrontmatter => IssueCategory::AuthoredPages,
	}
}

fn is_raw_path(path: &str) -> bool {
	path == "raw" || path.starts_with("raw/")
}

fn is_asset_path(path: &str) -> bool {
	path.starts_with("raw/assets/")
}

fn is_root_meta_page(path: &str) -> bool {
	!path.contains('/')
		&& (path.eq_ignore_ascii_case("SCHEMA.md")
			|| path.eq_ignore_ascii_case("index.md")
			|| path.eq_ignore_ascii_case("log.md"))
}

fn has_llm_wiki_layout(pages: &[PageScan]) -> bool {
	pages
		.iter()
		.any(|page| is_authored_page(&page.rel) || is_raw_path(&page.rel))
}

fn is_llm_wiki_linted_page(path: &str) -> bool {
	is_authored_page(path) || is_root_meta_page(path)
}

fn is_authored_page(path: &str) -> bool {
	[
		"entities/",
		"concepts/",
		"questions/",
		"syntheses/",
		"queries/",
		"meetings/",
	]
	.iter()
	.any(|prefix| path.starts_with(prefix))
}

fn is_source_capture_wikilink(target: &str) -> bool {
	matches!(target, "P" | "FIGCAPTION" | "H1" | "H2" | "H3" | "H4" | "H5" | "H6")
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

fn extract_atx_headings(content: &str) -> Vec<String> {
	let mut headings = Vec::new();
	let mut fences = FenceTracker::new();
	for line in content.lines() {
		if fences.observe(line) || fences.in_fence() {
			continue;
		}
		let trimmed_start = line.trim_start();
		let Some(rest) = trimmed_start.strip_prefix('#') else {
			continue;
		};
		let level = trimmed_start.bytes().take_while(|b| *b == b'#').count();
		if !(1..=6).contains(&level) {
			continue;
		}
		let rest = &rest[level - 1..];
		if !rest.starts_with(char::is_whitespace) {
			continue;
		}
		let heading = rest.trim().trim_end_matches('#').trim();
		if !heading.is_empty() {
			headings.push(heading.to_string());
		}
	}
	headings
}

fn broken_block_references(source_pages: &[PageScan], target_pages: &[PageScan]) -> Vec<Issue> {
	let targets: BTreeMap<&str, &PageScan> = target_pages.iter().map(|page| (page.rel.as_str(), page)).collect();
	fragment_issues(source_pages, &targets, FragmentKind::Block)
}

fn broken_heading_references(source_pages: &[PageScan], target_pages: &[PageScan]) -> Vec<Issue> {
	let targets: BTreeMap<&str, &PageScan> = target_pages.iter().map(|page| (page.rel.as_str(), page)).collect();
	fragment_issues(source_pages, &targets, FragmentKind::Heading)
}

#[derive(Clone, Copy)]
enum FragmentKind {
	Block,
	Heading,
}

fn fragment_issues(source_pages: &[PageScan], targets: &BTreeMap<&str, &PageScan>, kind: FragmentKind) -> Vec<Issue> {
	source_pages
		.iter()
		.flat_map(|page| {
			page.links.iter().filter_map(move |(link, resolution)| {
				let Resolution::Resolved(target) = resolution else {
					return None;
				};
				let fragment = link.fragment.as_deref()?;
				let target_page = targets.get(target.as_str())?;
				match kind {
					FragmentKind::Block => {
						let block_id = fragment.strip_prefix('^')?;
						if target_page.block_anchors.iter().any(|anchor| anchor == block_id) {
							return None;
						}
						Some(issue(
							Check::BrokenBlockReference,
							&page.rel,
							format!("{} points to missing block ^{} in {}", link.raw, block_id, target),
							"add the block anchor to the target page or fix the fragment",
						))
					}
					FragmentKind::Heading => {
						if fragment.starts_with('^') {
							return None;
						}
						let wanted = fragment.rsplit('#').next().unwrap_or(fragment).trim();
						if wanted.is_empty()
							|| target_page
								.headings
								.iter()
								.any(|heading| heading.trim().eq_ignore_ascii_case(wanted))
						{
							return None;
						}
						Some(issue(
							Check::BrokenHeadingReference,
							&page.rel,
							format!("{} points to missing heading '{}' in {}", link.raw, wanted, target),
							"add the heading to the target page or fix the fragment",
						))
					}
				}
			})
		})
		.collect()
}

fn broken_links(pages: &[PageScan], profile: DoctorProfile) -> Vec<Issue> {
	pages
		.iter()
		.flat_map(|page| {
			page.links
				.iter()
				.filter(|(_, resolution)| matches!(resolution, Resolution::Broken))
				.filter(move |(link, _)| {
					!matches!(profile, DoctorProfile::LlmWiki)
						|| !is_raw_path(&page.rel)
						|| !is_source_capture_wikilink(&link.target)
				})
				.map(|(link, _)| {
					let severity = if matches!(profile, DoctorProfile::LlmWiki) && is_raw_path(&page.rel) {
						Severity::Low
					} else {
						Check::BrokenLinks.severity()
					};
					issue_with_severity(
						Check::BrokenLinks,
						severity,
						&page.rel,
						format!("{} resolves to nothing", link.raw),
						"create the target page or fix the link",
					)
				})
		})
		.collect()
}

fn ambiguous_links(pages: &[PageScan], duplicate_aliases: &BTreeMap<String, Vec<String>>) -> Vec<Issue> {
	pages
		.iter()
		.flat_map(|page| {
			page.links.iter().filter_map(|(link, resolution)| match resolution {
				Resolution::Ambiguous(candidates) if !is_duplicate_alias_link(link, candidates, duplicate_aliases) => {
					Some(issue(
						Check::AmbiguousLinks,
						&page.rel,
						format!(
							"{} matches {} files: {}",
							link.raw,
							candidates.len(),
							candidates.join(", ")
						),
						"qualify the link with its folder, e.g. [[folder/name]]",
					))
				}
				_ => None,
			})
		})
		.collect()
}

fn duplicate_aliases(pages: &[PageScan]) -> Vec<Issue> {
	duplicate_alias_paths(pages)
		.into_iter()
		.map(|(alias, paths)| {
			issue(
				Check::DuplicateAliases,
				&paths[0],
				format!("alias '{alias}' is shared by {}", paths.join(", ")),
				"remove or rename duplicate aliases so alias wikilinks resolve uniquely",
			)
		})
		.collect()
}

fn duplicate_alias_paths(pages: &[PageScan]) -> BTreeMap<String, Vec<String>> {
	let mut by_alias: BTreeMap<String, Vec<String>> = BTreeMap::new();
	for page in pages {
		for alias in frontmatter::aliases(&page.frontmatter) {
			by_alias.entry(alias.to_lowercase()).or_default().push(page.rel.clone());
		}
	}
	by_alias
		.into_iter()
		.filter_map(|(alias, mut paths)| {
			paths.sort();
			paths.dedup();
			(paths.len() > 1).then_some((alias, paths))
		})
		.collect()
}

fn is_duplicate_alias_link(
	link: &ExtractedLink,
	candidates: &[String],
	duplicate_aliases: &BTreeMap<String, Vec<String>>,
) -> bool {
	let Some(alias_paths) = duplicate_aliases.get(&link.target.to_lowercase()) else {
		return false;
	};
	let mut candidates = candidates.to_vec();
	candidates.sort();
	&candidates == alias_paths
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

fn missing_frontmatter(pages: &[PageScan], profile: DoctorProfile) -> Vec<Issue> {
	// Only meaningful when the vault "uses" frontmatter: at least half of the
	// eligible pages carry a block (well-formed or not). In the LLM-wiki
	// profile, raw captures and root meta pages do not influence adoption.
	let eligible: Vec<&PageScan> = pages
		.iter()
		.filter(|page| {
			!matches!(profile, DoctorProfile::LlmWiki) || (!is_raw_path(&page.rel) && !is_root_meta_page(&page.rel))
		})
		.collect();
	let with_block = eligible
		.iter()
		.filter(|page| !matches!(page.frontmatter, Frontmatter::Absent))
		.count();
	if eligible.is_empty()
		|| match profile {
			DoctorProfile::LlmWiki => with_block * 2 < eligible.len(),
			DoctorProfile::Strict => with_block == 0,
		} {
		return Vec::new();
	}
	eligible
		.into_iter()
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
		.filter_map(|page| {
			let detail = page.frontmatter.malformed_detail()?;
			Some(issue(
				Check::MalformedFrontmatter,
				&page.rel,
				clean_yaml_error(detail),
				"fix the YAML between the --- markers or remove the block",
			))
		})
		.collect()
}

fn clean_yaml_error(detail: &str) -> String {
	let detail = detail.trim().replace('\n', " ");
	let Some((reason, location)) = detail.rsplit_once(" at line ") else {
		return format!("invalid YAML frontmatter: {detail}");
	};
	let line = location
		.split(|ch: char| !ch.is_ascii_digit())
		.next()
		.filter(|line| !line.is_empty());
	let reason = strip_yaml_locations(reason.trim()).trim_end_matches('.').to_string();
	match line {
		Some(line) => format!("invalid YAML frontmatter (line {line}): {reason}"),
		None => format!("invalid YAML frontmatter: {reason}"),
	}
}

fn strip_yaml_locations(reason: &str) -> String {
	let mut cleaned = reason.to_string();
	while let Some(start) = cleaned.find(" at line ") {
		let tail = &cleaned[start + " at line ".len()..];
		let Some(comma) = tail.find(',') else {
			cleaned.truncate(start);
			break;
		};
		cleaned.replace_range(start..start + " at line ".len() + comma + 1, "");
	}
	cleaned.trim().to_string()
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

fn duplicate_stems(files: &[(String, PathBuf)], profile: DoctorProfile, use_llm_scope: bool) -> Vec<Issue> {
	let mut by_stem: BTreeMap<String, Vec<&str>> = BTreeMap::new();
	for (rel, _) in files {
		let name = rel.rsplit('/').next().unwrap_or(rel);
		let stem = name.rsplit_once('.').map_or(name, |(stem, _)| stem);
		by_stem.entry(stem.to_lowercase()).or_default().push(rel);
	}
	by_stem
		.into_iter()
		.filter(|(_, paths)| paths.len() > 1)
		.filter_map(|(stem, mut paths)| {
			paths.sort_unstable();
			let page_count = paths.iter().filter(|path| is_page(path)).count();
			let scoped_pages: Vec<&str> = paths
				.iter()
				.copied()
				.filter(|path| is_page(path) && (!use_llm_scope || is_llm_wiki_linted_page(path)))
				.collect();
			if matches!(profile, DoctorProfile::LlmWiki) && scoped_pages.is_empty() {
				return None;
			}
			let scoped_page_count = if use_llm_scope { scoped_pages.len() } else { page_count };
			let severity = match profile {
				DoctorProfile::Strict => Severity::Medium,
				DoctorProfile::LlmWiki if scoped_page_count >= 2 => Severity::Medium,
				DoctorProfile::LlmWiki if scoped_page_count == 1 => Severity::Low,
				DoctorProfile::LlmWiki => return None,
			};
			let action = if scoped_page_count >= 2 {
				"rename one page so wikilink stems stay unique"
			} else {
				"rename assets only if the shared stem is confusing"
			};
			Some(issue_with_severity(
				Check::DuplicateStems,
				severity,
				scoped_pages.first().copied().unwrap_or(paths[0]),
				format!("stem '{stem}' is shared by {}", paths.join(", ")),
				action,
			))
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
			("duplicate_aliases", 0),
			("orphan_pages", 3),
			("broken_block_reference", 0),
			("broken_heading_reference", 0),
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
		// The base ops fixture is clean for all non-orphan checks; the
		// orphan (notes/unicode.md) is covered separately below.
		let (_dir, vault) = test_fixtures::vault();
		let report = vault.doctor(&DoctorOptions::default()).unwrap();
		for check in [
			"broken_links",
			"ambiguous_links",
			"duplicate_aliases",
			"broken_block_reference",
			"broken_heading_reference",
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
	fn llm_wiki_profile_reports_broken_links_in_scaffold_questions() {
		let dir = tempfile::tempdir().unwrap();
		std::fs::create_dir_all(dir.path().join("questions")).unwrap();
		std::fs::write(
			dir.path().join("questions/refunds.md"),
			"# Refunds\n\n[[missing-answer]]\n",
		)
		.unwrap();
		let vault = Vault::open(dir.path()).unwrap();

		let report = vault.doctor(&DoctorOptions::default()).unwrap();

		assert_eq!(report.counts["broken_links"], 1, "issues: {:?}", report.issues);
		assert_eq!(issue_paths(&report, Check::BrokenLinks), vec!["questions/refunds.md"]);
	}

	#[test]
	fn llm_wiki_profile_ignores_raw_and_meta_frontmatter_policy_noise() {
		let dir = tempfile::tempdir().unwrap();
		std::fs::create_dir_all(dir.path().join("entities")).unwrap();
		std::fs::create_dir_all(dir.path().join("raw")).unwrap();
		std::fs::write(dir.path().join("entities/a.md"), "---\ntitle: A\n---\n[[b]]\n").unwrap();
		std::fs::write(dir.path().join("entities/b.md"), "# B\n").unwrap();
		std::fs::write(dir.path().join("entities/c.md"), "---\ntitle: C\n---\n[[a]]\n").unwrap();
		std::fs::write(dir.path().join("entities/d.md"), "---\ntitle: D\n---\n[[a]]\n").unwrap();
		std::fs::write(dir.path().join("raw/capture.md"), "[[P]] [[H1]] [[FIGCAPTION]]\n").unwrap();
		std::fs::write(dir.path().join("SCHEMA.md"), "# Schema\n").unwrap();
		std::fs::write(dir.path().join("log.md"), "# Log\n").unwrap();
		let vault = Vault::open(dir.path()).unwrap();

		let default = vault.doctor(&DoctorOptions::default()).unwrap();
		assert_eq!(issue_paths(&default, Check::BrokenLinks), Vec::<&str>::new());
		assert_eq!(issue_paths(&default, Check::MissingFrontmatter), vec!["entities/b.md"]);

		let strict = vault
			.doctor(&DoctorOptions {
				profile: DoctorProfile::Strict,
				..Default::default()
			})
			.unwrap();
		assert_eq!(strict.counts["broken_links"], 3);
		assert!(issue_paths(&strict, Check::MissingFrontmatter).contains(&"SCHEMA.md"));
		assert!(issue_paths(&strict, Check::MissingFrontmatter).contains(&"log.md"));
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
				..Default::default()
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
				..Default::default()
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
	fn llm_wiki_profile_classifies_duplicate_stems_by_page_and_asset_kind() {
		let dir = tempfile::tempdir().unwrap();
		std::fs::create_dir_all(dir.path().join("raw/assets")).unwrap();
		std::fs::write(dir.path().join("page.md"), "# Page\n").unwrap();
		std::fs::write(dir.path().join("page.png"), b"png").unwrap();
		std::fs::write(dir.path().join("raw/assets/logo.png"), b"png").unwrap();
		std::fs::write(dir.path().join("raw/assets/logo.svg"), b"svg").unwrap();
		let vault = Vault::open(dir.path()).unwrap();

		let default = vault.doctor(&DoctorOptions::default()).unwrap();
		let duplicates: Vec<_> = default
			.issues
			.iter()
			.filter(|issue| issue.check == Check::DuplicateStems)
			.collect();
		assert_eq!(duplicates.len(), 1, "issues: {:?}", default.issues);
		assert_eq!(duplicates[0].severity, Severity::Low);
		assert!(duplicates[0].detail.contains("page.md, page.png"));

		let strict = vault
			.doctor(&DoctorOptions {
				profile: DoctorProfile::Strict,
				..Default::default()
			})
			.unwrap();
		assert_eq!(strict.counts["duplicate_stems"], 2);
	}

	#[test]
	fn binary_attachments_are_invisible_to_content_checks() {
		// logo.png contains wikilink-looking bytes; they must not surface.
		let (_dir, vault) = test_fixtures::knowledge_vault();
		let report = vault.doctor(&DoctorOptions::default()).unwrap();
		assert_eq!(report.counts["broken_links"], 1);
		assert!(report.issues.iter().all(|i| !i.path.contains("logo.png")));
	}

	#[test]
	fn doctor_uses_obsidian_attachment_folder_for_duplicate_attachment_links() {
		let (dir, vault) = test_fixtures::vault();
		std::fs::write(
			dir.path().join(".obsidian/app.json"),
			r#"{"attachmentFolderPath":"assets"}"#,
		)
		.unwrap();
		vault.write("assets/logo.png", "configured").unwrap();
		vault.write("other/logo.png", "duplicate").unwrap();
		vault.write("source.md", "![[logo.png]]\n").unwrap();

		let report = vault
			.doctor(&DoctorOptions {
				checks: Some(vec![Check::BrokenLinks, Check::AmbiguousLinks]),
				profile: DoctorProfile::Strict,
				..Default::default()
			})
			.unwrap();
		assert_eq!(report.counts["broken_links"], 0, "issues: {:?}", report.issues);
		assert_eq!(report.counts["ambiguous_links"], 0, "issues: {:?}", report.issues);
	}

	#[test]
	fn doctor_resolves_frontmatter_aliases_without_a_broken_link() {
		let (_dir, vault) = test_fixtures::vault();
		vault
			.write("entities/acme.md", "---\naliases: [ACME Corp, Acme]\n---\n# Acme\n")
			.unwrap();
		vault.write("linker.md", "[[Acme]] [[ACME Corp]]\n").unwrap();
		let report = vault
			.doctor(&DoctorOptions {
				checks: Some(vec![Check::BrokenLinks, Check::AmbiguousLinks]),
				profile: DoctorProfile::Strict,
				..Default::default()
			})
			.unwrap();
		assert_eq!(report.counts["broken_links"], 0, "issues: {:?}", report.issues);
		assert_eq!(report.counts["ambiguous_links"], 0, "issues: {:?}", report.issues);
	}

	#[test]
	fn doctor_uses_obsidian_markdown_path_modes_for_broken_links() {
		let (_dir, vault) = test_fixtures::vault();
		vault.write("Home.md", "# Home\n").unwrap();
		vault.write("Sibling.md", "# Root sibling\n").unwrap();
		vault.write("Sub/Child.md", "[a](../Home.md)\n").unwrap();
		vault.write("Sub/Sibling.md", "# Sub sibling\n").unwrap();
		vault.write("Sub/P1.md", "[a](Sibling.md)\n").unwrap();
		vault.write("Sub/P2.md", "[a](./Sibling.md)\n").unwrap();
		vault.write("Sub/P3.md", "[a](/Sibling.md)\n").unwrap();
		vault.write("Sub/Escape.md", "[a](../../Home.md)\n").unwrap();
		let report = vault
			.doctor(&DoctorOptions {
				checks: Some(vec![Check::BrokenLinks]),
				profile: DoctorProfile::Strict,
				..Default::default()
			})
			.unwrap();
		assert_eq!(report.counts["broken_links"], 1, "issues: {:?}", report.issues);
		assert_eq!(report.issues[0].path, "Sub/Escape.md");
	}

	#[test]
	fn malformed_frontmatter_detail_is_sanitized() {
		let dir = tempfile::tempdir().unwrap();
		std::fs::write(dir.path().join("bad.md"), "---\ntitle: [bad\n---\n# Bad\n").unwrap();
		let vault = Vault::open(dir.path()).unwrap();
		let report = vault
			.doctor(&DoctorOptions {
				checks: Some(vec![Check::MalformedFrontmatter]),
				profile: DoctorProfile::Strict,
				..Default::default()
			})
			.unwrap();
		let issue = report.issues.first().unwrap();
		assert!(
			issue.detail.starts_with("invalid YAML frontmatter (line "),
			"{}",
			issue.detail
		);
		assert!(!issue.detail.contains("column"), "{}", issue.detail);
	}

	#[test]
	fn doctor_reports_alias_collisions_as_duplicate_aliases() {
		let (_dir, vault) = test_fixtures::vault();
		vault
			.write("entities/acme.md", "---\naliases: Client\n---\n# Acme\n")
			.unwrap();
		vault
			.write("entities/other.md", "---\naliases: CLIENT\n---\n# Other\n")
			.unwrap();
		vault.write("linker.md", "[[client]]\n").unwrap();
		let report = vault
			.doctor(&DoctorOptions {
				checks: Some(vec![Check::BrokenLinks, Check::AmbiguousLinks, Check::DuplicateAliases]),
				profile: DoctorProfile::Strict,
				..Default::default()
			})
			.unwrap();
		assert_eq!(report.counts["broken_links"], 0, "issues: {:?}", report.issues);
		assert_eq!(
			report.counts["ambiguous_links"], 0,
			"duplicate alias should own this finding"
		);
		assert_eq!(report.counts["duplicate_aliases"], 1);
		let issue = report
			.issues
			.iter()
			.find(|issue| issue.check == Check::DuplicateAliases)
			.unwrap();
		assert_eq!(issue.severity, Severity::Low);
		assert!(issue.detail.contains("entities/acme.md, entities/other.md"));
	}

	#[test]
	fn duplicate_aliases_fire_without_referencing_link() {
		let dir = tempfile::tempdir().unwrap();
		std::fs::write(dir.path().join("a.md"), "---\naliases: Same\n---\n# A\n").unwrap();
		std::fs::write(dir.path().join("b.md"), "---\naliases: same\n---\n# B\n").unwrap();
		let vault = Vault::open(dir.path()).unwrap();
		let report = vault
			.doctor(&DoctorOptions {
				checks: Some(vec![Check::DuplicateAliases]),
				profile: DoctorProfile::Strict,
				..Default::default()
			})
			.unwrap();
		assert_eq!(report.counts["duplicate_aliases"], 1, "issues: {:?}", report.issues);
	}

	#[test]
	fn doctor_reports_missing_block_references() {
		let dir = tempfile::tempdir().unwrap();
		std::fs::write(dir.path().join("source.md"), "[[note#^missing]]\n").unwrap();
		std::fs::write(dir.path().join("note.md"), "Some text ^abc\n").unwrap();
		let vault = Vault::open(dir.path()).unwrap();
		let report = vault
			.doctor(&DoctorOptions {
				checks: Some(vec![Check::BrokenBlockReference]),
				profile: DoctorProfile::Strict,
				..Default::default()
			})
			.unwrap();
		assert_eq!(report.counts["broken_block_reference"], 1);
		let issue = report.issues.first().unwrap();
		assert_eq!(issue.severity, Severity::Medium);
		assert_eq!(issue.path, "source.md");
		assert!(issue.detail.contains("^missing"));
	}

	#[test]
	fn doctor_accepts_existing_block_references() {
		let dir = tempfile::tempdir().unwrap();
		std::fs::write(dir.path().join("source.md"), "[[note#^abc]]\n").unwrap();
		std::fs::write(dir.path().join("note.md"), "Some text ^abc\n").unwrap();
		let vault = Vault::open(dir.path()).unwrap();
		let report = vault
			.doctor(&DoctorOptions {
				checks: Some(vec![Check::BrokenBlockReference]),
				profile: DoctorProfile::Strict,
				..Default::default()
			})
			.unwrap();
		assert_eq!(
			report.counts["broken_block_reference"], 0,
			"issues: {:?}",
			report.issues
		);
	}

	#[test]
	fn doctor_validates_fragment_only_wikilinks_against_containing_page() {
		let dir = tempfile::tempdir().unwrap();
		std::fs::write(
			dir.path().join("source.md"),
			"# Child Section\n\n[[#Child Section]] [[#Missing]] ![[#^block]] ![[#^absent]]\nanchor ^block\n",
		)
		.unwrap();
		let vault = Vault::open(dir.path()).unwrap();
		let report = vault
			.doctor(&DoctorOptions {
				checks: Some(vec![Check::BrokenHeadingReference, Check::BrokenBlockReference]),
				profile: DoctorProfile::Strict,
				..Default::default()
			})
			.unwrap();
		assert_eq!(
			report.counts["broken_heading_reference"], 1,
			"issues: {:?}",
			report.issues
		);
		assert_eq!(
			report.counts["broken_block_reference"], 1,
			"issues: {:?}",
			report.issues
		);
		assert!(report.issues.iter().any(|issue| issue.detail.contains("Missing")));
		assert!(report.issues.iter().any(|issue| issue.detail.contains("^absent")));
	}

	#[test]
	fn doctor_accepts_fragment_only_nested_heading_final_segment() {
		let dir = tempfile::tempdir().unwrap();
		std::fs::write(
			dir.path().join("source.md"),
			"# Top\n\n## Sub\n\n[[#Top#Sub]] [[#Top#Missing]]\n",
		)
		.unwrap();
		let vault = Vault::open(dir.path()).unwrap();
		let report = vault
			.doctor(&DoctorOptions {
				checks: Some(vec![Check::BrokenHeadingReference]),
				profile: DoctorProfile::Strict,
				..Default::default()
			})
			.unwrap();
		assert_eq!(
			report.counts["broken_heading_reference"], 1,
			"issues: {:?}",
			report.issues
		);
		assert!(report.issues[0].detail.contains("Missing"));
	}

	#[test]
	fn doctor_validates_markdown_fragment_only_links_against_containing_page() {
		let dir = tempfile::tempdir().unwrap();
		std::fs::write(
			dir.path().join("source.md"),
			"# Heading One\n\n[x](#Heading%20One) [bad](#Missing) [block](#^b1) [bad-block](#^missing)\nanchor ^b1\n",
		)
		.unwrap();
		let vault = Vault::open(dir.path()).unwrap();
		let report = vault
			.doctor(&DoctorOptions {
				checks: Some(vec![Check::BrokenHeadingReference, Check::BrokenBlockReference]),
				profile: DoctorProfile::Strict,
				..Default::default()
			})
			.unwrap();
		assert_eq!(
			report.counts["broken_heading_reference"], 1,
			"issues: {:?}",
			report.issues
		);
		assert_eq!(
			report.counts["broken_block_reference"], 1,
			"issues: {:?}",
			report.issues
		);
		assert!(report.issues.iter().any(|issue| issue.detail.contains("Missing")));
		assert!(report.issues.iter().any(|issue| issue.detail.contains("^missing")));
	}

	#[test]
	fn doctor_accepts_heading_and_block_after_mismatched_fence_inside_code_block() {
		let dir = tempfile::tempdir().unwrap();
		std::fs::write(
			dir.path().join("source.md"),
			"[[target#Real Heading]] [[target#^blk1]]\n",
		)
		.unwrap();
		std::fs::write(
			dir.path().join("target.md"),
			"```\n~~~\nstill code ^ignored\n```\n## Real Heading\nanchored line ^blk1\n",
		)
		.unwrap();
		let vault = Vault::open(dir.path()).unwrap();
		let report = vault
			.doctor(&DoctorOptions {
				checks: Some(vec![Check::BrokenHeadingReference, Check::BrokenBlockReference]),
				profile: DoctorProfile::Strict,
				..Default::default()
			})
			.unwrap();
		assert_eq!(
			report.counts["broken_heading_reference"], 0,
			"issues: {:?}",
			report.issues
		);
		assert_eq!(
			report.counts["broken_block_reference"], 0,
			"issues: {:?}",
			report.issues
		);
	}

	#[test]
	fn doctor_reports_missing_heading_references_case_insensitively() {
		let dir = tempfile::tempdir().unwrap();
		std::fs::write(dir.path().join("source.md"), "[[note#setup]] [[note#Missing|Alias]]\n").unwrap();
		std::fs::write(dir.path().join("note.md"), "# Note\n\n## Setup\n").unwrap();
		let vault = Vault::open(dir.path()).unwrap();
		let report = vault
			.doctor(&DoctorOptions {
				checks: Some(vec![Check::BrokenHeadingReference]),
				profile: DoctorProfile::Strict,
				..Default::default()
			})
			.unwrap();
		assert_eq!(report.counts["broken_heading_reference"], 1);
		let issue = report.issues.first().unwrap();
		assert_eq!(issue.severity, Severity::Medium);
		assert!(issue.detail.contains("Missing"));
	}

	#[test]
	fn doctor_validates_nested_heading_fragments_by_final_segment() {
		let dir = tempfile::tempdir().unwrap();
		std::fs::write(dir.path().join("source.md"), "[[t#Top#Sub]] [[t#Top#Missing]]\n").unwrap();
		std::fs::write(dir.path().join("t.md"), "# Top\n\n## Sub\n").unwrap();
		let vault = Vault::open(dir.path()).unwrap();
		let report = vault
			.doctor(&DoctorOptions {
				checks: Some(vec![Check::BrokenHeadingReference]),
				profile: DoctorProfile::Strict,
				..Default::default()
			})
			.unwrap();
		assert_eq!(
			report.counts["broken_heading_reference"], 1,
			"issues: {:?}",
			report.issues
		);
		assert!(report.issues[0].detail.contains("Missing"));
	}

	#[test]
	fn llm_wiki_profile_scopes_fragment_reference_sources() {
		let dir = tempfile::tempdir().unwrap();
		std::fs::create_dir_all(dir.path().join("entities")).unwrap();
		std::fs::create_dir_all(dir.path().join("raw")).unwrap();
		std::fs::write(dir.path().join("entities/source.md"), "[[target#^missing]]\n").unwrap();
		std::fs::write(dir.path().join("raw/source.md"), "[[target#^missing]]\n").unwrap();
		std::fs::write(dir.path().join("target.md"), "# Target\n").unwrap();
		let vault = Vault::open(dir.path()).unwrap();
		let default = vault
			.doctor(&DoctorOptions {
				checks: Some(vec![Check::BrokenBlockReference]),
				..Default::default()
			})
			.unwrap();
		assert_eq!(
			issue_paths(&default, Check::BrokenBlockReference),
			vec!["entities/source.md"]
		);

		let strict = vault
			.doctor(&DoctorOptions {
				checks: Some(vec![Check::BrokenBlockReference]),
				profile: DoctorProfile::Strict,
				..Default::default()
			})
			.unwrap();
		assert_eq!(strict.counts["broken_block_reference"], 2);
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
				..Default::default()
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
		assert_eq!(report.counts.len(), 11);
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
