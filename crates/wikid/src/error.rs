//! CLI-surface errors (DESIGN §6): one structured shape for core errors and
//! CLI-level conditions. Errors always go to stdout with exit code 1, as
//! `error[<code>]: <message>` plus an optional `hint:` line, or as
//! `{"error":{"code","message","hint"}}` under `--json`.

use wikid_core::WikidError;

/// A renderable CLI error.
#[derive(Debug)]
pub struct CliError {
	/// Stable machine-readable code (core `WikidError::code()` or CLI-level).
	pub code: String,
	/// Human-readable message.
	pub message: String,
	/// Optional next-step suggestion.
	pub hint: Option<String>,
}

impl CliError {
	pub fn new(code: &str, message: impl Into<String>, hint: Option<String>) -> Self {
		Self {
			code: code.to_string(),
			message: message.into(),
			hint,
		}
	}

	/// Remote mode was selected but no wiki name was given (DESIGN §6).
	pub fn no_wiki() -> Self {
		Self::new(
			"no_wiki",
			"remote mode needs a wiki name: pass --wiki <name> or set $WIKID_WIKI",
			Some("wiki names are the keys under [wikis] in the daemon's config".to_string()),
		)
	}

	/// `wikid serve` found no config file anywhere in the discovery chain.
	pub fn no_config() -> Self {
		Self::new(
			"no_config",
			"no config file found: pass --config <path>, set $WIKID_CONFIG, \
			 or create ./wikid.toml or ~/.config/wikid/config.toml",
			Some("see docs/wikid.example.toml for the format".to_string()),
		)
	}

	/// Neither a local directory nor a remote server was targeted.
	pub fn no_target() -> Self {
		Self::new(
			"no_target",
			"no wiki targeted: pass --dir <path> (or set $WIKID_DIR) for a local wiki, \
			 or --server <url> --token <t> --wiki <name> (or $WIKID_SERVER/$WIKID_TOKEN/$WIKID_WIKI) for a remote one",
			Some("wikid --dir . status — inspect the current directory as a wiki".to_string()),
		)
	}

	/// `rm` without `--force`: the refusal is a structured error, never an
	/// interactive question (AXI checklist item 6).
	pub fn force_required(path: &str) -> Self {
		Self::new(
			"force_required",
			format!("rm is destructive: refusing to delete {path} without --force"),
			Some("re-run with --force to delete it permanently (there is no undo)".to_string()),
		)
	}

	/// Renders `error[<code>]: <message>` plus the optional `hint:` line.
	pub fn human(&self) -> String {
		match &self.hint {
			Some(hint) => format!("error[{}]: {}\nhint: {}", self.code, self.message, hint),
			None => format!("error[{}]: {}", self.code, self.message),
		}
	}

	/// Renders the `{"error":{...}}` object; `hint` is omitted when absent.
	pub fn json(&self) -> String {
		let mut error = serde_json::Map::new();
		error.insert("code".to_string(), self.code.clone().into());
		error.insert("message".to_string(), self.message.clone().into());
		if let Some(hint) = &self.hint {
			error.insert("hint".to_string(), hint.clone().into());
		}
		serde_json::json!({ "error": error }).to_string()
	}
}

impl From<WikidError> for CliError {
	fn from(err: WikidError) -> Self {
		Self {
			code: err.code().to_string(),
			message: err.to_string(),
			hint: err.hint(),
		}
	}
}

#[cfg(test)]
mod tests {
	use super::*;

	#[test]
	fn human_format_with_and_without_hint() {
		let err = CliError::new("boom", "it broke", Some("try again".to_string()));
		assert_eq!(err.human(), "error[boom]: it broke\nhint: try again");
		let err = CliError::new("boom", "it broke", None);
		assert_eq!(err.human(), "error[boom]: it broke");
	}

	#[test]
	fn json_format_omits_absent_hint() {
		let err = CliError::new("boom", "it broke", None);
		let value: serde_json::Value = serde_json::from_str(&err.json()).unwrap();
		assert_eq!(value["error"]["code"], "boom");
		assert_eq!(value["error"]["message"], "it broke");
		assert!(value["error"].get("hint").is_none());

		let err = CliError::new("boom", "it broke", Some("try again".to_string()));
		let value: serde_json::Value = serde_json::from_str(&err.json()).unwrap();
		assert_eq!(value["error"]["hint"], "try again");
	}

	#[test]
	fn core_errors_carry_code_message_and_hint() {
		let err = CliError::from(WikidError::NotFound {
			path: "a.md".to_string(),
		});
		assert_eq!(err.code, "not_found");
		assert_eq!(err.message, "not found: a.md");
		assert!(err.hint.is_some());
	}

	#[test]
	fn cli_level_errors_have_stable_codes() {
		assert_eq!(CliError::no_target().code, "no_target");
		assert_eq!(CliError::no_wiki().code, "no_wiki");
		assert_eq!(CliError::no_config().code, "no_config");
		assert_eq!(CliError::force_required("a.md").code, "force_required");
	}
}
