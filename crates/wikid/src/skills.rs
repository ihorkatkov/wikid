//! Embedded agent usage guides for `wikid skills`.

use std::fs;
use std::io::IsTerminal;
use std::path::{Path, PathBuf};

use serde::Serialize;

use crate::error::CliError;

#[derive(Debug)]
pub struct Skill {
	pub name: &'static str,
	pub body: &'static str,
	pub references: &'static [(&'static str, &'static str)],
}

pub const SKILLS: &[Skill] = &[
	Skill {
		name: "core",
		body: include_str!("../../../skills/core/SKILL.md"),
		references: &[
			(
				"json-shapes",
				include_str!("../../../skills/core/references/json-shapes.md"),
			),
			(
				"doctor-checks",
				include_str!("../../../skills/core/references/doctor-checks.md"),
			),
			(
				"link-resolution",
				include_str!("../../../skills/core/references/link-resolution.md"),
			),
		],
	},
	Skill {
		name: "llm-wiki",
		body: include_str!("../../../skills/llm-wiki/SKILL.md"),
		references: &[],
	},
];

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct SkillSummary {
	pub name: String,
	pub description: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct SkillListResult {
	pub skills: Vec<SkillSummary>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct SkillGetResult {
	pub name: String,
	pub full: bool,
	pub content: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct SkillPathResult {
	pub path: String,
	pub version: String,
	pub versioned_path: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct SkillStatusResult {
	pub embedded: EmbeddedStatus,
	pub materialized: MaterializedStatus,
	pub wiring: Vec<WiringStatus>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct EmbeddedStatus {
	pub version: String,
	pub guides: usize,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct MaterializedStatus {
	pub path: String,
	pub current: Option<String>,
	pub version_present: bool,
	pub stale_versions: usize,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct WiringStatus {
	pub link: String,
	pub target: String,
	pub state: WiringState,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum WiringState {
	Ok,
	Pinned,
	Broken,
}

#[derive(Debug, serde::Deserialize)]
struct Frontmatter {
	#[serde(rename = "name")]
	_name: String,
	description: String,
}

pub fn list() -> Result<SkillListResult, CliError> {
	let mut skills = Vec::new();
	for skill in SKILLS {
		skills.push(SkillSummary {
			name: skill.name.to_owned(),
			description: frontmatter(skill)?.description,
		});
	}
	Ok(SkillListResult { skills })
}

pub fn get(name: &str, full: bool) -> Result<SkillGetResult, CliError> {
	let skill = find(name)?;
	Ok(SkillGetResult {
		name: skill.name.to_owned(),
		full,
		content: content(skill, full),
	})
}

pub fn materialize(name: Option<&str>) -> Result<SkillPathResult, CliError> {
	if let Some(name) = name {
		find(name)?;
	}
	let version = env!("CARGO_PKG_VERSION");
	let base = skills_data_dir()?;
	let version_dir = base.join(version);
	let complete = version_dir.join(".complete");
	if !complete.is_file() {
		fs::create_dir_all(&base).map_err(io_error)?;
		let tmp = base.join(format!(".{version}.tmp-{}", std::process::id()));
		if tmp.exists() {
			fs::remove_dir_all(&tmp).map_err(io_error)?;
		}
		fs::create_dir_all(&tmp).map_err(io_error)?;
		write_all(&tmp)?;
		fs::write(tmp.join(".complete"), b"ok\n").map_err(io_error)?;
		if version_dir.exists() {
			fs::remove_dir_all(&version_dir).map_err(io_error)?;
		}
		fs::rename(&tmp, &version_dir).map_err(io_error)?;
	}
	update_current_symlink(&base, &version_dir)?;
	let current_dir = base.join("current");
	let path = match name {
		Some(name) => current_dir.join(name),
		None => current_dir,
	};
	let versioned_path = match name {
		Some(name) => version_dir.join(name),
		None => version_dir,
	};
	Ok(SkillPathResult {
		path: path.display().to_string(),
		version: version.to_owned(),
		versioned_path: versioned_path.display().to_string(),
	})
}

pub fn render_list(result: &SkillListResult) -> String {
	render_list_at_width(result, catalog_width())
}

fn render_list_at_width(result: &SkillListResult, width: usize) -> String {
	let mut lines = Vec::new();
	let name_width = result
		.skills
		.iter()
		.map(|skill| skill.name.chars().count())
		.max()
		.unwrap_or(0);
	for skill in &result.skills {
		let prefix = format!("{:<name_width$} — ", skill.name);
		lines.extend(wrap_with_prefix(&prefix, &skill.description, width));
	}
	let guide_word = if result.skills.len() == 1 { "guide" } else { "guides" };
	lines.extend(wrap_with_prefix(
		"total: ",
		&format!("{} {guide_word}", result.skills.len()),
		width,
	));
	lines.extend(wrap_with_prefix(
		"hint: ",
		"`wikid skills get core` to read one; add --full for the complete reference",
		width,
	));
	lines.join("\n")
}

fn catalog_width() -> usize {
	if std::io::stdout().is_terminal() {
		std::env::var("COLUMNS")
			.ok()
			.and_then(|value| value.parse::<usize>().ok())
			.unwrap_or(100)
			.clamp(40, 120)
	} else {
		72
	}
}

pub fn status() -> Result<SkillStatusResult, CliError> {
	let version = env!("CARGO_PKG_VERSION").to_owned();
	let base = skills_data_dir()?;
	let version_dir = base.join(&version);
	let current = current_target(&base)?;
	let version_present = version_dir.join(".complete").is_file();
	let stale_versions = stale_version_count(&base, &version)?;
	Ok(SkillStatusResult {
		embedded: EmbeddedStatus {
			version,
			guides: SKILLS.len(),
		},
		materialized: MaterializedStatus {
			path: base.display().to_string(),
			current: current.map(|path| path.display().to_string()),
			version_present,
			stale_versions,
		},
		wiring: scan_claude_wiring(&base)?,
	})
}

pub fn render_status(result: &SkillStatusResult) -> String {
	render_status_at_width(result, catalog_width(), env_nonempty("HOME").as_deref().map(Path::new))
}

fn render_status_at_width(result: &SkillStatusResult, width: usize, home: Option<&Path>) -> String {
	let mut lines = Vec::new();
	lines.extend(wrap_with_prefix_hard(
		"embedded: ",
		&format!("version {}  guides {}", result.embedded.version, result.embedded.guides),
		width,
	));
	let base = Path::new(&result.materialized.path);
	lines.extend(wrap_with_prefix_hard(
		"materialized: ",
		&abbreviate_home(&result.materialized.path, home),
		width,
	));
	lines.extend(wrap_with_prefix_hard(
		"  version_present: ",
		if result.materialized.version_present {
			"yes"
		} else {
			"no"
		},
		width,
	));
	let current = result
		.materialized
		.current
		.as_deref()
		.map(|path| abbreviate_home(path, home))
		.unwrap_or_else(|| "(missing)".to_owned());
	lines.extend(wrap_with_prefix_hard("  current: ", &current, width));
	lines.extend(wrap_with_prefix_hard(
		"  stale_versions: ",
		&result.materialized.stale_versions.to_string(),
		width,
	));
	lines.push("wiring:".to_owned());
	if result.wiring.is_empty() {
		lines.push("  none".to_owned());
		lines.extend(wrap_with_prefix_hard(
			"hint: ",
			"ln -s \"$(wikid skills path core)\" ~/.claude/skills/wikid-core",
			width,
		));
	} else {
		let rows = result
			.wiring
			.iter()
			.map(|wire| {
				let link = abbreviate_home(&wire.link, home);
				let target =
					relativize_target(&wire.target, base).unwrap_or_else(|| abbreviate_home(&wire.target, home));
				let left = format!("{link} -> {target}");
				(left, wire.state.as_str())
			})
			.collect::<Vec<_>>();
		let left_width = rows.iter().map(|(left, _)| left.chars().count()).max().unwrap_or(0);
		for (left, state) in rows {
			lines.extend(render_wiring_line(&left, state, left_width, width));
		}
		if result.wiring.iter().any(|wire| wire.state == WiringState::Pinned) {
			lines.extend(wrap_with_prefix_hard(
				"hint: ",
				"relink pinned skills with `wikid skills path <name>` so updates self-heal",
				width,
			));
		}
	}
	if !result.materialized.version_present {
		lines.extend(wrap_with_prefix_hard(
			"hint: ",
			"wikid skills path — materialize the current version",
			width,
		));
	}
	lines.join("\n")
}

impl WiringState {
	fn as_str(&self) -> &'static str {
		match self {
			WiringState::Ok => "ok",
			WiringState::Pinned => "pinned",
			WiringState::Broken => "broken",
		}
	}
}

fn current_target(base: &Path) -> Result<Option<PathBuf>, CliError> {
	let current = base.join("current");
	match fs::read_link(&current) {
		Ok(target) => Ok(Some(target)),
		Err(err) if err.kind() == std::io::ErrorKind::NotFound => Ok(None),
		Err(err) => Err(io_error(err)),
	}
}

fn stale_version_count(base: &Path, version: &str) -> Result<usize, CliError> {
	let Ok(entries) = fs::read_dir(base) else {
		return Ok(0);
	};
	let mut count = 0;
	for entry in entries {
		let entry = entry.map_err(io_error)?;
		let file_name = entry.file_name();
		let name = file_name.to_string_lossy();
		if name == version || name == "current" || name.starts_with('.') {
			continue;
		}
		if entry.file_type().map_err(io_error)?.is_dir() && entry.path().join(".complete").is_file() {
			count += 1;
		}
	}
	Ok(count)
}

fn scan_claude_wiring(base: &Path) -> Result<Vec<WiringStatus>, CliError> {
	let Some(home) = env_nonempty("HOME") else {
		return Ok(Vec::new());
	};
	let skills_dir = PathBuf::from(home).join(".claude").join("skills");
	let Ok(entries) = fs::read_dir(&skills_dir) else {
		return Ok(Vec::new());
	};
	let base_canon = canonicalize_lossy(base);
	let current_path = base.join("current");
	let mut wiring = Vec::new();
	for entry in entries {
		let entry = entry.map_err(io_error)?;
		let link_path = entry.path();
		let meta = fs::symlink_metadata(&link_path).map_err(io_error)?;
		if !meta.file_type().is_symlink() {
			continue;
		}
		let raw_target = fs::read_link(&link_path).map_err(io_error)?;
		let abs_target = if raw_target.is_absolute() {
			raw_target.clone()
		} else {
			skills_dir.join(&raw_target)
		};
		let target_text = raw_target.display().to_string();
		let resolved = fs::canonicalize(&abs_target);
		let state = match resolved {
			Ok(resolved) => {
				if !canonicalize_lossy(&resolved).starts_with(&base_canon) {
					continue;
				}
				if path_routes_through_current(&abs_target, &current_path) {
					WiringState::Ok
				} else {
					WiringState::Pinned
				}
			}
			Err(_) => WiringState::Broken,
		};
		wiring.push(WiringStatus {
			link: link_path.display().to_string(),
			target: target_text,
			state,
		});
	}
	wiring.sort_by(|a, b| a.link.cmp(&b.link));
	Ok(wiring)
}

fn canonicalize_lossy(path: &Path) -> PathBuf {
	fs::canonicalize(path).unwrap_or_else(|_| path.to_path_buf())
}

fn path_routes_through_current(target: &Path, current: &Path) -> bool {
	target == current || target.starts_with(current)
}

fn write_all(root: &Path) -> Result<(), CliError> {
	for skill in SKILLS {
		let skill_dir = root.join(skill.name);
		fs::create_dir_all(&skill_dir).map_err(io_error)?;
		fs::write(skill_dir.join("SKILL.md"), skill.body).map_err(io_error)?;
		if !skill.references.is_empty() {
			let refs_dir = skill_dir.join("references");
			fs::create_dir_all(&refs_dir).map_err(io_error)?;
			for (title, body) in skill.references {
				fs::write(refs_dir.join(format!("{title}.md")), body).map_err(io_error)?;
			}
		}
	}
	Ok(())
}

fn update_current_symlink(base: &Path, version_dir: &Path) -> Result<(), CliError> {
	let current = base.join("current");
	if current.exists() || current.symlink_metadata().is_ok() {
		let meta = current.symlink_metadata().map_err(io_error)?;
		if meta.file_type().is_dir() && !meta.file_type().is_symlink() {
			fs::remove_dir_all(&current).map_err(io_error)?;
		} else {
			fs::remove_file(&current).map_err(io_error)?;
		}
	}
	create_symlink(version_dir, &current)
}

#[cfg(unix)]
fn create_symlink(target: &Path, link: &Path) -> Result<(), CliError> {
	std::os::unix::fs::symlink(target, link).map_err(io_error)
}

#[cfg(windows)]
fn create_symlink(target: &Path, link: &Path) -> Result<(), CliError> {
	std::os::windows::fs::symlink_dir(target, link).map_err(io_error)
}

fn skills_data_dir() -> Result<PathBuf, CliError> {
	if let Some(xdg) = env_nonempty("XDG_DATA_HOME") {
		return Ok(PathBuf::from(xdg).join("wikid").join("skills"));
	}
	let home = env_nonempty("HOME").ok_or_else(|| {
		CliError::new(
			"io",
			"cannot determine home directory for skills path",
			Some("set XDG_DATA_HOME or HOME".to_owned()),
		)
	})?;
	Ok(PathBuf::from(home)
		.join(".local")
		.join("share")
		.join("wikid")
		.join("skills"))
}

fn env_nonempty(name: &str) -> Option<String> {
	std::env::var(name).ok().filter(|value| !value.is_empty())
}

pub fn skill_name_help() -> String {
	format!("Guide to print [possible values: {}]", skill_names().join(", "))
}

fn skill_names() -> Vec<&'static str> {
	SKILLS.iter().map(|skill| skill.name).collect()
}

pub fn find(name: &str) -> Result<&'static Skill, CliError> {
	SKILLS
		.iter()
		.find(|skill| skill.name == name)
		.ok_or_else(|| unknown(name))
}

pub fn content(skill: &Skill, full: bool) -> String {
	let mut text = skill.body.to_owned();
	if full {
		for (title, reference) in skill.references {
			if !text.ends_with('\n') {
				text.push('\n');
			}
			text.push('\n');
			text.push_str(&format!("# Reference: {title}\n\n"));
			text.push_str(reference_without_title(reference));
			if !text.ends_with('\n') {
				text.push('\n');
			}
		}
	}
	text
}

fn reference_without_title(reference: &str) -> &str {
	reference
		.split_once('\n')
		.map(|(_, rest)| rest.trim_start())
		.unwrap_or(reference)
}

fn frontmatter(skill: &Skill) -> Result<Frontmatter, CliError> {
	let yaml = frontmatter_text(skill.body).ok_or_else(|| {
		CliError::new(
			"skill_frontmatter",
			format!("skill {} has no YAML frontmatter", skill.name),
			None,
		)
	})?;
	serde_yaml::from_str::<Frontmatter>(yaml).map_err(|err| {
		CliError::new(
			"skill_frontmatter",
			format!("skill {} frontmatter is invalid: {err}", skill.name),
			None,
		)
	})
}

pub(crate) fn frontmatter_text(body: &str) -> Option<&str> {
	let rest = body.strip_prefix("---\n")?;
	let (yaml, _) = rest.split_once("\n---")?;
	Some(yaml)
}

fn unknown(name: &str) -> CliError {
	let hint = nearest(name)
		.map(|candidate| format!("did you mean {candidate}?"))
		.unwrap_or_else(|| "run `wikid skills` to list available guides".to_owned());
	CliError::new("not_found", format!("skill not found: {name}"), Some(hint))
}

fn nearest(name: &str) -> Option<&'static str> {
	SKILLS
		.iter()
		.map(|skill| (skill.name, levenshtein(name, skill.name)))
		.min_by_key(|(_, distance)| *distance)
		.and_then(|(candidate, distance)| (distance <= 4).then_some(candidate))
}

fn levenshtein(a: &str, b: &str) -> usize {
	let mut prev: Vec<usize> = (0..=b.chars().count()).collect();
	let mut curr = vec![0; prev.len()];
	for (i, ca) in a.chars().enumerate() {
		curr[0] = i + 1;
		for (j, cb) in b.chars().enumerate() {
			let cost = usize::from(ca != cb);
			curr[j + 1] = (curr[j] + 1).min(prev[j + 1] + 1).min(prev[j] + cost);
		}
		std::mem::swap(&mut prev, &mut curr);
	}
	prev[b.chars().count()]
}

fn wrap_with_prefix(prefix: &str, text: &str, width: usize) -> Vec<String> {
	let indent = " ".repeat(prefix.chars().count());
	let mut lines = Vec::new();
	let mut line = prefix.to_owned();
	for word in text.split_whitespace() {
		let sep = if line.trim().is_empty() || line == prefix {
			""
		} else {
			" "
		};
		if line.chars().count() + sep.chars().count() + word.chars().count() > width && line != prefix {
			lines.push(line);
			line = format!("{indent}{word}");
		} else {
			line.push_str(sep);
			line.push_str(word);
		}
	}
	lines.push(line);
	lines
}

fn abbreviate_home(path: &str, home: Option<&Path>) -> String {
	let Some(home) = home else {
		return path.to_owned();
	};
	let path_ref = Path::new(path);
	if path_ref == home {
		return "~".to_owned();
	}
	path_ref
		.strip_prefix(home)
		.ok()
		.map(|rest| format!("~/{}", rest.to_string_lossy()))
		.unwrap_or_else(|| path.to_owned())
}

fn relativize_target(target: &str, base: &Path) -> Option<String> {
	Path::new(target)
		.strip_prefix(base)
		.ok()
		.map(|path| path.to_string_lossy().to_string())
		.filter(|path| !path.is_empty())
}

fn render_wiring_line(left: &str, state: &str, left_width: usize, width: usize) -> Vec<String> {
	let padded_left = pad_to_chars(left, left_width);
	let line = format!("  {padded_left}  {state}");
	if line.chars().count() <= width {
		return vec![line];
	}
	let mut lines = wrap_hard_with_indent("  ", left, width);
	let last = lines.pop().unwrap_or_else(|| "  ".to_owned());
	let state_line = format!("{last}  {state}");
	if state_line.chars().count() <= width {
		lines.push(state_line);
	} else {
		lines.push(last);
		lines.extend(wrap_hard_with_indent("  ", state, width));
	}
	lines
}

fn pad_to_chars(text: &str, width: usize) -> String {
	let len = text.chars().count();
	if len >= width {
		text.to_owned()
	} else {
		format!("{}{}", text, " ".repeat(width - len))
	}
}

fn wrap_with_prefix_hard(prefix: &str, text: &str, width: usize) -> Vec<String> {
	let mut lines = Vec::new();
	let indent = " ".repeat(prefix.chars().count());
	let mut line = prefix.to_owned();
	for word in text.split_whitespace() {
		let sep = if line == prefix { "" } else { " " };
		if line.chars().count() + sep.chars().count() + word.chars().count() <= width {
			line.push_str(sep);
			line.push_str(word);
			continue;
		}
		if line != prefix {
			lines.push(line);
			line = indent.clone();
		}
		if line.chars().count() + word.chars().count() <= width {
			line.push_str(word);
		} else {
			let mut chunks = wrap_hard_with_indent(&line, word, width);
			line = chunks.pop().unwrap_or_else(|| indent.clone());
			lines.extend(chunks);
		}
	}
	lines.push(line);
	lines
}

fn wrap_hard_with_indent(indent: &str, text: &str, width: usize) -> Vec<String> {
	let indent_len = indent.chars().count();
	let available = width.saturating_sub(indent_len).max(1);
	let mut lines = Vec::new();
	let mut current = indent.to_owned();
	for ch in text.chars() {
		if current.chars().count() >= width {
			lines.push(current);
			current = indent.to_owned();
		}
		if current.chars().count() - indent_len >= available {
			lines.push(current);
			current = indent.to_owned();
		}
		current.push(ch);
	}
	lines.push(current);
	lines
}

fn io_error(err: std::io::Error) -> CliError {
	CliError::new("io", err.to_string(), None)
}

#[cfg(test)]
mod tests {
	use super::*;

	#[test]
	fn descriptions_come_from_frontmatter() {
		let result = list().unwrap();
		assert_eq!(result.skills.len(), SKILLS.len());
		assert!(result.skills.iter().any(|skill| skill.name == "core"));
	}

	#[test]
	fn unknown_skill_gets_did_you_mean_hint() {
		let err = find("cor").unwrap_err();
		assert_eq!(err.code, "not_found");
		assert_eq!(err.hint.as_deref(), Some("did you mean core?"));
	}

	#[test]
	fn rendered_catalog_uses_a_fixed_description_column() {
		let result = SkillListResult {
			skills: vec![
				SkillSummary {
					name: "core".to_owned(),
					description: "one two three four five six".to_owned(),
				},
				SkillSummary {
					name: "llm-wiki".to_owned(),
					description: "alpha beta gamma delta".to_owned(),
				},
			],
		};
		let rendered = render_list_at_width(&result, 28);
		assert_eq!(
			rendered.lines().take(4).collect::<Vec<_>>(),
			vec![
				"core     — one two three",
				"           four five six",
				"llm-wiki — alpha beta gamma",
				"           delta",
			]
		);
	}

	#[test]
	fn rendered_catalog_respects_width_for_every_line() {
		let result = list().unwrap();
		for width in [40, 72, 120] {
			let rendered = render_list_at_width(&result, width);
			for line in rendered.lines() {
				let len = line.chars().count();
				assert!(
					len <= width,
					"line is {len} chars at width {width}: {line:?}\n{rendered}"
				);
			}
		}
	}

	#[test]
	fn rendered_status_abbreviates_relativizes_and_aligns_state_column() {
		let home = Path::new("/tmp/fake-home");
		let result = SkillStatusResult {
			embedded: EmbeddedStatus {
				version: "0.1.0".to_owned(),
				guides: 2,
			},
			materialized: MaterializedStatus {
				path: "/tmp/fake-home/.local/share/wikid/skills".to_owned(),
				current: Some("/tmp/fake-home/.local/share/wikid/skills/0.1.0".to_owned()),
				version_present: true,
				stale_versions: 1,
			},
			wiring: vec![
				WiringStatus {
					link: "/tmp/fake-home/.claude/skills/wikid-core".to_owned(),
					target: "/tmp/fake-home/.local/share/wikid/skills/current/core".to_owned(),
					state: WiringState::Ok,
				},
				WiringStatus {
					link: "/tmp/fake-home/.claude/skills/wikid-llm-wiki".to_owned(),
					target: "/tmp/fake-home/.local/share/wikid/skills/0.1.0/llm-wiki".to_owned(),
					state: WiringState::Pinned,
				},
				WiringStatus {
					link: "/tmp/fake-home/.claude/skills/wikid-old".to_owned(),
					target: "/tmp/fake-home/.local/share/wikid/skills/0.0.9/core".to_owned(),
					state: WiringState::Broken,
				},
			],
		};

		let rendered = render_status_at_width(&result, 120, Some(home));
		assert!(rendered.contains("materialized: ~/.local/share/wikid/skills"));
		assert!(rendered.contains("  current: ~/.local/share/wikid/skills/0.1.0"));
		assert!(rendered.contains("~/.claude/skills/wikid-core -> current/core"));
		assert!(rendered.contains("~/.claude/skills/wikid-llm-wiki -> 0.1.0/llm-wiki"));
		assert!(rendered.contains("~/.claude/skills/wikid-old -> 0.0.9/core"));
		assert!(!rendered.contains("/tmp/fake-home"));

		let state_columns = ["ok", "pinned", "broken"]
			.into_iter()
			.map(|state| {
				let line = rendered.lines().find(|line| line.ends_with(state)).unwrap();
				line.chars().count() - state.chars().count()
			})
			.collect::<Vec<_>>();
		assert_eq!(state_columns[0], state_columns[1]);
		assert_eq!(state_columns[1], state_columns[2]);
	}

	#[test]
	fn rendered_status_respects_width_for_every_line() {
		let home = Path::new("/tmp/fake-home");
		let result = SkillStatusResult {
			embedded: EmbeddedStatus {
				version: "0.1.0".to_owned(),
				guides: 2,
			},
			materialized: MaterializedStatus {
				path: "/tmp/fake-home/.local/share/wikid/skills".to_owned(),
				current: Some("/tmp/fake-home/.local/share/wikid/skills/current".to_owned()),
				version_present: false,
				stale_versions: 0,
			},
			wiring: vec![WiringStatus {
				link: "/tmp/fake-home/.claude/skills/wikid-very-deep-nested-long-name".to_owned(),
				target: "/tmp/fake-home/.local/share/wikid/skills/current/very-deep-nested-long-name".to_owned(),
				state: WiringState::Ok,
			}],
		};

		for width in [40, 72, 120] {
			let rendered = render_status_at_width(&result, width, Some(home));
			for line in rendered.lines() {
				let len = line.chars().count();
				assert!(
					len <= width,
					"line is {len} chars at width {width}: {line:?}\n{rendered}"
				);
			}
		}
	}

	#[test]
	fn registered_skill_frontmatter_is_valid() {
		for skill in SKILLS {
			let yaml = frontmatter_text(skill.body).expect("frontmatter");
			let value: serde_yaml::Value = serde_yaml::from_str(yaml).expect("valid yaml");
			let name = value.get("name").and_then(serde_yaml::Value::as_str).expect("name");
			let description = value
				.get("description")
				.and_then(serde_yaml::Value::as_str)
				.expect("description");
			assert!(!description.is_empty());
			assert!(description.len() <= 1024);
			assert!(name.len() <= 64);
			assert!(
				name.chars()
					.all(|ch| ch.is_ascii_lowercase() || ch.is_ascii_digit() || ch == '-')
			);
			assert!(!name.contains("anthropic"));
			assert!(!name.contains("claude"));
			assert!(skill.body.lines().count() <= 500);
		}
	}
}
