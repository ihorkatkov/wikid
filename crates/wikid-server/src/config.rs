use std::collections::BTreeMap;
use std::ffi::OsString;
use std::path::{Path, PathBuf};

use anyhow::Context as _;
use serde::{Deserialize, Serialize};

/// Daemon configuration, loaded from a single TOML file.
///
/// ```toml
/// bind = "127.0.0.1:7448"
/// default_wiki = "projects"
///
/// [wikis]
/// projects = "/home/ihor/Projects/projects-wiki"
///
/// [tokens]
/// "wkd_abc123" = "agent-vm-1"
/// ```
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Config {
	#[serde(default = "default_bind")]
	pub bind: String,
	/// Optional default wiki for zero-target CLI commands when multiple wikis are registered.
	#[serde(default, skip_serializing_if = "Option::is_none")]
	pub default_wiki: Option<String>,
	/// Wiki name -> directory path.
	#[serde(default)]
	pub wikis: BTreeMap<String, PathBuf>,
	/// Bearer token -> actor name.
	#[serde(default)]
	pub tokens: BTreeMap<String, String>,
	/// Client-only named remote targets. The daemon ignores this table.
	#[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
	pub remotes: BTreeMap<String, RemoteProfile>,
}

/// A complete client connection target under `[remotes.<name>]`.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RemoteProfile {
	pub server: String,
	#[serde(default, skip_serializing_if = "Option::is_none")]
	pub token: Option<String>,
	/// Wiki name on the daemon; defaults to the profile name when omitted.
	#[serde(default, skip_serializing_if = "Option::is_none")]
	pub wiki: Option<String>,
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

	pub fn empty() -> Self {
		Self {
			bind: default_bind(),
			default_wiki: None,
			wikis: BTreeMap::new(),
			tokens: BTreeMap::new(),
			remotes: BTreeMap::new(),
		}
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

/// Picks the config file to create or mutate. Explicit path and `$WIKID_CONFIG`
/// are honored even when absent; otherwise an existing `./wikid.toml` wins;
/// if no config exists, the global config path is returned.
pub fn write_target(explicit: Option<&Path>) -> Option<PathBuf> {
	write_target_in(
		explicit,
		std::env::var_os("WIKID_CONFIG"),
		Path::new("."),
		std::env::home_dir(),
	)
}

pub fn global_path() -> Option<PathBuf> {
	std::env::home_dir().map(|home| home.join(".config").join("wikid").join("config.toml"))
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

fn write_target_in(
	explicit: Option<&Path>,
	env: Option<OsString>,
	cwd: &Path,
	home: Option<PathBuf>,
) -> Option<PathBuf> {
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
	home.map(|home| home.join(".config").join("wikid").join("config.toml"))
}

pub fn save(path: &Path, config: &Config) -> anyhow::Result<()> {
	if let Some(parent) = path.parent() {
		std::fs::create_dir_all(parent).with_context(|| format!("create config directory {}", parent.display()))?;
	}
	let text = toml::to_string_pretty(config).context("serialize config")?;
	let tmp = path.with_extension(format!("{}.tmp", std::process::id()));
	std::fs::write(&tmp, text).with_context(|| format!("write temp config {}", tmp.display()))?;
	set_owner_only_permissions(&tmp)?;
	std::fs::rename(&tmp, path).with_context(|| format!("replace config {}", path.display()))?;
	set_owner_only_permissions(path)?;
	Ok(())
}

#[cfg(unix)]
fn set_owner_only_permissions(path: &Path) -> anyhow::Result<()> {
	use std::os::unix::fs::PermissionsExt as _;
	std::fs::set_permissions(path, std::fs::Permissions::from_mode(0o600))
		.with_context(|| format!("set 0600 permissions on {}", path.display()))
}

#[cfg(not(unix))]
fn set_owner_only_permissions(_path: &Path) -> anyhow::Result<()> {
	Ok(())
}

#[cfg(test)]
mod tests {
	use super::*;

	#[test]
	fn parses_named_remote_profiles() {
		let config = Config::from_toml(
			r#"
			default_wiki = "projects"

			[remotes.projects]
			server = "https://projects-wiki.example"
			token = "wkd_client_secret"
			wiki = "shared"
			"#,
		)
		.unwrap();
		let remote = &config.remotes["projects"];
		assert_eq!(remote.server, "https://projects-wiki.example");
		assert_eq!(remote.token.as_deref(), Some("wkd_client_secret"));
		assert_eq!(remote.wiki.as_deref(), Some("shared"));
	}

	#[test]
	fn remote_token_and_wiki_are_optional() {
		let config = Config::from_toml("[remotes.public]\nserver = \"http://127.0.0.1:7448\"\n").unwrap();
		let remote = &config.remotes["public"];
		assert_eq!(remote.token, None);
		assert_eq!(remote.wiki, None);
	}

	#[test]
	fn serialization_preserves_profiles_but_omits_an_empty_remotes_table() {
		let empty = toml::to_string_pretty(&Config::empty()).unwrap();
		assert!(!empty.contains("remotes"));
		let config = Config::from_toml(
			"default_wiki = \"team\"\n[wikis]\nlocal = \"/tmp/wiki\"\n[remotes.team]\nserver = \"https://wiki.example\"\n",
		)
		.unwrap();
		let serialized = toml::to_string_pretty(&config).unwrap();
		let reparsed = Config::from_toml(&serialized).unwrap();
		assert_eq!(reparsed.default_wiki.as_deref(), Some("team"));
		assert!(reparsed.wikis.contains_key("local"));
		assert_eq!(reparsed.remotes["team"].server, "https://wiki.example");
	}

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
		assert_eq!(config.default_wiki, None);
		assert_eq!(config.wikis["projects"], PathBuf::from("/tmp/wiki"));
		assert_eq!(config.tokens["wkd_test"], "agent-vm-1");
	}

	#[test]
	fn config_without_wikis_is_an_empty_config() {
		let config = Config::from_toml("bind = \"127.0.0.1:7448\"").unwrap();
		assert!(config.wikis.is_empty());
		assert_eq!(config.bind, "127.0.0.1:7448");
	}

	#[test]
	fn save_preserves_remote_profiles_during_real_config_rewrite() {
		let dir = tempfile::tempdir().unwrap();
		let path = dir.path().join("wikid.toml");
		std::fs::write(
			&path,
			"default_wiki = \"team\"\n[remotes.team]\nserver = \"https://wiki.example\"\ntoken = \"remote-secret\"\nwiki = \"shared\"\n",
		)
		.unwrap();
		let mut config = Config::load(&path).unwrap();
		config.tokens.insert("server-secret".to_owned(), "admin".to_owned());
		save(&path, &config).unwrap();
		let rewritten = Config::load(&path).unwrap();
		let remote = &rewritten.remotes["team"];
		assert_eq!(remote.server, "https://wiki.example");
		assert_eq!(remote.token.as_deref(), Some("remote-secret"));
		assert_eq!(remote.wiki.as_deref(), Some("shared"));
		assert_eq!(rewritten.tokens["server-secret"], "admin");
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

	#[test]
	fn write_target_uses_existing_local_before_global() {
		let cwd = tempfile::tempdir().unwrap();
		let home = tempfile::tempdir().unwrap();
		std::fs::write(cwd.path().join("wikid.toml"), "").unwrap();
		assert_eq!(
			write_target_in(None, None, cwd.path(), Some(home.path().to_path_buf())),
			Some(cwd.path().join("wikid.toml"))
		);
		let cwd2 = tempfile::tempdir().unwrap();
		assert_eq!(
			write_target_in(None, None, cwd2.path(), Some(home.path().to_path_buf())),
			Some(home.path().join(".config").join("wikid").join("config.toml"))
		);
	}
}
