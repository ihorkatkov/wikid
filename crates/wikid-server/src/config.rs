use std::collections::BTreeMap;
use std::ffi::OsString;
use std::path::{Path, PathBuf};

use anyhow::Context as _;
use serde::Deserialize;

/// Daemon configuration, loaded from a single TOML file.
///
/// ```toml
/// bind = "127.0.0.1:7448"
///
/// [wikis]
/// projects = "/home/ihor/Projects/projects-wiki"
///
/// [tokens]
/// "wkd_abc123" = "agent-vm-1"
/// ```
#[derive(Debug, Deserialize)]
pub struct Config {
	#[serde(default = "default_bind")]
	pub bind: String,
	/// Wiki name -> directory path.
	pub wikis: BTreeMap<String, PathBuf>,
	/// Bearer token -> actor name.
	#[serde(default)]
	pub tokens: BTreeMap<String, String>,
}

fn default_bind() -> String {
	"127.0.0.1:7448".to_string()
}

impl Config {
	pub fn from_toml(input: &str) -> Result<Self, toml::de::Error> {
		toml::from_str(input)
	}

	/// Reads and parses the TOML config file at `path`.
	pub fn load(path: &Path) -> anyhow::Result<Self> {
		let text = std::fs::read_to_string(path).with_context(|| format!("read config file {}", path.display()))?;
		Self::from_toml(&text).with_context(|| format!("parse config file {}", path.display()))
	}
}

/// Picks the config file (DESIGN §6): explicit path → `$WIKID_CONFIG` →
/// `./wikid.toml` → `~/.config/wikid/config.toml`. The first two express
/// explicit intent and are returned without an existence check so a typo
/// fails loudly at load time; the two defaults apply only when present.
pub fn discover(explicit: Option<&Path>) -> Option<PathBuf> {
	discover_in(
		explicit,
		std::env::var_os("WIKID_CONFIG"),
		Path::new("."),
		std::env::home_dir(),
	)
}

fn discover_in(explicit: Option<&Path>, env: Option<OsString>, cwd: &Path, home: Option<PathBuf>) -> Option<PathBuf> {
	if let Some(path) = explicit {
		return Some(path.to_path_buf());
	}
	if let Some(path) = env.filter(|v| !v.is_empty()) {
		return Some(PathBuf::from(path));
	}
	let local = cwd.join("wikid.toml");
	if local.is_file() {
		return Some(local);
	}
	let global = home?.join(".config").join("wikid").join("config.toml");
	global.is_file().then_some(global)
}

#[cfg(test)]
mod tests {
	use super::*;

	#[test]
	fn parses_minimal_config() {
		let config = Config::from_toml(
			r#"
			[wikis]
			projects = "/tmp/wiki"

			[tokens]
			"wkd_test" = "agent-vm-1"
			"#,
		)
		.unwrap();
		assert_eq!(config.bind, "127.0.0.1:7448");
		assert_eq!(config.wikis["projects"], PathBuf::from("/tmp/wiki"));
		assert_eq!(config.tokens["wkd_test"], "agent-vm-1");
	}

	#[test]
	fn rejects_config_without_wikis() {
		assert!(Config::from_toml("bind = \"127.0.0.1:7448\"").is_err());
	}

	#[test]
	fn load_reads_and_parses_the_file() {
		let dir = tempfile::tempdir().unwrap();
		let path = dir.path().join("wikid.toml");
		std::fs::write(&path, "[wikis]\nmain = \"/tmp/wiki\"\n").unwrap();
		let config = Config::load(&path).unwrap();
		assert_eq!(config.wikis["main"], PathBuf::from("/tmp/wiki"));
		assert!(config.tokens.is_empty());

		let missing = Config::load(&dir.path().join("nope.toml")).unwrap_err();
		assert!(missing.to_string().contains("nope.toml"));
		std::fs::write(&path, "not toml [").unwrap();
		assert!(Config::load(&path).is_err());
	}

	#[test]
	fn discover_prefers_explicit_then_env() {
		let cwd = tempfile::tempdir().unwrap();
		std::fs::write(cwd.path().join("wikid.toml"), "").unwrap();
		let explicit = Path::new("/etc/wikid.toml");
		assert_eq!(
			discover_in(Some(explicit), Some("env.toml".into()), cwd.path(), None),
			Some(explicit.to_path_buf())
		);
		assert_eq!(
			discover_in(None, Some("env.toml".into()), cwd.path(), None),
			Some(PathBuf::from("env.toml"))
		);
		// An empty env var is treated as unset.
		assert_eq!(
			discover_in(None, Some(OsString::new()), cwd.path(), None),
			Some(cwd.path().join("wikid.toml"))
		);
	}

	#[test]
	fn discover_falls_back_to_cwd_then_home() {
		let cwd = tempfile::tempdir().unwrap();
		let home = tempfile::tempdir().unwrap();
		let global = home.path().join(".config").join("wikid").join("config.toml");
		std::fs::create_dir_all(global.parent().unwrap()).unwrap();
		std::fs::write(&global, "").unwrap();
		assert_eq!(
			discover_in(None, None, cwd.path(), Some(home.path().to_path_buf())),
			Some(global)
		);

		let empty_home = tempfile::tempdir().unwrap();
		assert_eq!(
			discover_in(None, None, cwd.path(), Some(empty_home.path().to_path_buf())),
			None
		);
		assert_eq!(discover_in(None, None, cwd.path(), None), None);
	}
}
