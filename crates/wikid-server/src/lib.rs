//! wikid-server: the daemon. Serves one or more named vaults over HTTP
//! (and later MCP), authenticated with named bearer tokens.
//!
//! [`fn@app`] builds the full router without binding a port (tests drive it via
//! `tower::ServiceExt::oneshot`); [`serve`] performs startup validation,
//! binds, and runs until stopped.

mod app;
pub mod config;
mod error;

pub use app::{AppState, WikiList, WikiSummary, app};
pub use config::Config;
pub use error::ApiError;

use anyhow::Context as _;

/// Validates the config, binds `config.bind`, and serves until stopped.
///
/// Startup validation (DESIGN §7): every wiki directory must exist (fail
/// fast), and an empty token map is refused on a non-loopback bind —
/// auth-less serving is allowed on loopback only, with a loud warning.
pub async fn serve(config: Config) -> anyhow::Result<()> {
	let state = AppState::from_config(&config)?;
	if state.auth_less() {
		if !bind_is_loopback(&config.bind)? {
			anyhow::bail!(
				"refusing to serve {} with no tokens configured: add a [tokens] entry or bind to loopback",
				config.bind
			);
		}
		tracing::warn!(bind = %config.bind, "no tokens configured — serving without authentication (loopback only)");
	}
	let listener = tokio::net::TcpListener::bind(&config.bind)
		.await
		.with_context(|| format!("bind {}", config.bind))?;
	tracing::info!(bind = %config.bind, wikis = config.wikis.len(), "wikid-server listening");
	axum::serve(listener, app(state)).await.context("serve")?;
	Ok(())
}

/// True when every address the bind string resolves to is loopback.
fn bind_is_loopback(bind: &str) -> anyhow::Result<bool> {
	use std::net::ToSocketAddrs as _;
	let addrs: Vec<_> = bind
		.to_socket_addrs()
		.with_context(|| format!("resolve bind address {bind:?}"))?
		.collect();
	if addrs.is_empty() {
		anyhow::bail!("bind address {bind:?} resolved to no addresses");
	}
	Ok(addrs.iter().all(|addr| addr.ip().is_loopback()))
}

#[cfg(test)]
mod tests {
	use std::collections::BTreeMap;
	use std::path::PathBuf;

	use super::*;

	#[test]
	fn loopback_binds_are_recognized() {
		assert!(bind_is_loopback("127.0.0.1:7448").unwrap());
		assert!(bind_is_loopback("[::1]:7448").unwrap());
		assert!(!bind_is_loopback("0.0.0.0:7448").unwrap());
		assert!(bind_is_loopback("not an address").is_err());
	}

	#[tokio::test]
	async fn serve_refuses_non_loopback_bind_without_tokens() {
		let dir = tempfile::tempdir().unwrap();
		let config = Config {
			bind: "0.0.0.0:0".to_owned(),
			default_wiki: None,
			wikis: BTreeMap::from([("main".to_owned(), dir.path().to_path_buf())]),
			tokens: BTreeMap::new(),
		};
		let err = serve(config).await.unwrap_err();
		assert!(err.to_string().contains("refusing to serve"), "got: {err:#}");
	}

	#[tokio::test]
	async fn serve_fails_fast_when_a_wiki_dir_is_missing() {
		let config = Config {
			bind: "127.0.0.1:0".to_owned(),
			default_wiki: None,
			wikis: BTreeMap::from([("ghost".to_owned(), PathBuf::from("/nonexistent/wiki"))]),
			tokens: BTreeMap::from([("t".to_owned(), "actor".to_owned())]),
		};
		let err = serve(config).await.unwrap_err();
		assert!(format!("{err:#}").contains("ghost"), "got: {err:#}");
	}
}
