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
	let path = match name {
		Some(name) => version_dir.join(name),
		None => version_dir,
	};
	Ok(SkillPathResult {
		path: path.display().to_string(),
		version: version.to_owned(),
	})
}

pub fn render_list(result: &SkillListResult) -> String {
	render_list_at_width(result, catalog_width())
}

fn render_list_at_width(result: &SkillListResult, width: usize) -> String {
	let mut lines = Vec::new();
	for skill in &result.skills {
		let prefix = format!("{} — ", skill.name);
		lines.extend(wrap_with_prefix(&prefix, &skill.description, width));
	}
	let guide_word = if result.skills.len() == 1 { "guide" } else { "guides" };
	lines.push(format!("total: {} {guide_word}", result.skills.len()));
	lines.push("hint: `wikid skills get core` to read one; add --full for the complete reference".to_owned());
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
	let indent = " ".repeat(prefix.len());
	let mut lines = Vec::new();
	let mut line = prefix.to_owned();
	for word in text.split_whitespace() {
		let sep = if line.trim().is_empty() || line == prefix {
			""
		} else {
			" "
		};
		if line.len() + sep.len() + word.len() > width && line != prefix {
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
	fn wrap_with_prefix_respects_width_and_hanging_indent() {
		let text = "one two three four five six seven";
		assert_eq!(
			wrap_with_prefix("core — ", text, 20),
			vec![
				"core — one two",
				"         three four",
				"         five six",
				"         seven"
			]
		);
		assert_eq!(
			wrap_with_prefix("core — ", text, 40),
			vec!["core — one two three four five six", "         seven"]
		);
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
