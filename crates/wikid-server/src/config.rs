use std::collections::BTreeMap;
use std::path::PathBuf;

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
}
