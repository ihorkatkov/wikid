//! Explicit self-update for the `wikid` binary.
//!
//! This is intentionally a CLI concern, not `wikid-core`: it manages the
//! installed executable, queries GitHub release metadata, verifies the release
//! checksum, and atomically replaces the current binary. It keeps no cache or
//! hidden state; every update check is an explicit command invocation.

use std::cmp::Ordering;
use std::fs::{self, File};
use std::io::{Read, Write};
use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};

use crate::error::CliError;

const OWNER: &str = "ihorkatkov";
const REPO: &str = "wikid";
const GITHUB_API: &str = "https://api.github.com";
const MAX_BINARY_BYTES: u64 = 64 * 1024 * 1024;
const USER_AGENT: &str = concat!("wikid/", env!("CARGO_PKG_VERSION"));

#[derive(Debug, Clone, Serialize)]
pub struct UpdateResult {
	pub current: String,
	pub target: String,
	pub action: UpdateAction,
	pub updated: bool,
	#[serde(skip_serializing_if = "Option::is_none")]
	pub asset: Option<String>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum UpdateAction {
	UpToDate,
	WouldUpdate,
	Updated,
	Reinstalled,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum UpdatePlan {
	UpToDate {
		tag: String,
	},
	Update {
		tag: String,
		asset: ReleaseAsset,
		checksum: ReleaseAsset,
	},
}

#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
pub struct GitHubRelease {
	pub tag_name: String,
	#[serde(default)]
	pub assets: Vec<ReleaseAsset>,
}

#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
pub struct ReleaseAsset {
	pub name: String,
	pub browser_download_url: String,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
struct Version {
	major: u64,
	minor: u64,
	patch: u64,
}

pub fn run(check: bool, force: bool, requested: Option<&str>) -> Result<UpdateResult, CliError> {
	let current = env!("CARGO_PKG_VERSION");
	let target = target_triple()?;
	let release = fetch_release(requested)?;
	let plan = resolve_plan(current, target, &release, force, requested)?;
	match plan {
		UpdatePlan::UpToDate { tag } => Ok(UpdateResult {
			current: current.to_owned(),
			target: tag,
			action: UpdateAction::UpToDate,
			updated: false,
			asset: None,
		}),
		UpdatePlan::Update {
			tag,
			asset,
			checksum: _,
		} if check => Ok(UpdateResult {
			current: current.to_owned(),
			target: tag,
			action: UpdateAction::WouldUpdate,
			updated: false,
			asset: Some(asset.name),
		}),
		UpdatePlan::Update { tag, asset, checksum } => {
			let exe = current_exe()?;
			ensure_parent_writable(&exe)?;
			let bytes = download_bytes(&asset.browser_download_url, MAX_BINARY_BYTES)?;
			let checksum_text = String::from_utf8(download_bytes(&checksum.browser_download_url, 16 * 1024)?)
				.map_err(|err| CliError::new("update", format!("release checksum is not valid UTF-8: {err}"), None))?;
			verify_checksum(&bytes, &checksum_text)?;
			replace_executable(&exe, &bytes)?;
			let action = if normalize_tag(current) == tag {
				UpdateAction::Reinstalled
			} else {
				UpdateAction::Updated
			};
			Ok(UpdateResult {
				current: current.to_owned(),
				target: tag,
				action,
				updated: true,
				asset: Some(asset.name),
			})
		}
	}
}

pub fn target_triple() -> Result<&'static str, CliError> {
	match (std::env::consts::OS, std::env::consts::ARCH) {
		("linux", "x86_64") => Ok("x86_64-unknown-linux-gnu"),
		("linux", "aarch64") => Ok("aarch64-unknown-linux-gnu"),
		("macos", "x86_64") => Ok("x86_64-apple-darwin"),
		("macos", "aarch64") => Ok("aarch64-apple-darwin"),
		(os, arch) => Err(CliError::new(
			"unsupported_target",
			format!("no wikid release asset is published for {os}/{arch}"),
			Some("install from source with the installer or cargo install".to_owned()),
		)),
	}
}

pub fn resolve_plan(
	current: &str,
	target_triple: &str,
	release: &GitHubRelease,
	force: bool,
	requested: Option<&str>,
) -> Result<UpdatePlan, CliError> {
	let current_version = parse_version(current)?;
	let release_version = parse_version(&release.tag_name)?;
	let tag = normalize_tag(&release.tag_name);
	if !force && current_version == release_version {
		return Ok(UpdatePlan::UpToDate { tag });
	}
	if requested.is_none() && !force {
		match current_version.cmp(&release_version) {
			Ordering::Equal => unreachable!("equal versions returned above"),
			Ordering::Greater => {
				return Err(CliError::new(
					"version_mismatch",
					format!(
						"current version {current} is newer than latest release {}",
						release.tag_name
					),
					Some(format!(
						"use `wikid update --force --version {}` to reinstall or downgrade",
						release.tag_name
					)),
				));
			}
			Ordering::Less => {}
		}
	}
	let asset_name = format!("wikid-{target_triple}");
	let checksum_name = format!("{asset_name}.sha256");
	let asset = find_asset(release, &asset_name)?;
	let checksum = find_asset(release, &checksum_name)?;
	Ok(UpdatePlan::Update { tag, asset, checksum })
}

fn find_asset(release: &GitHubRelease, name: &str) -> Result<ReleaseAsset, CliError> {
	release
		.assets
		.iter()
		.find(|asset| asset.name == name)
		.cloned()
		.ok_or_else(|| {
			CliError::new(
				"missing_release_asset",
				format!("release {} has no asset named {name}", release.tag_name),
				Some("open the GitHub releases page or install from source".to_owned()),
			)
		})
}

fn fetch_release(requested: Option<&str>) -> Result<GitHubRelease, CliError> {
	let path = match requested {
		Some(tag) => format!("/repos/{OWNER}/{REPO}/releases/tags/{}", normalize_tag(tag)),
		None => format!("/repos/{OWNER}/{REPO}/releases/latest"),
	};
	let url = format!("{GITHUB_API}{path}");
	let request = ureq::get(&url)
		.set("User-Agent", USER_AGENT)
		.set("Accept", "application/vnd.github+json");
	parse_github_json(request.call())
}

fn parse_github_json(result: Result<ureq::Response, ureq::Error>) -> Result<GitHubRelease, CliError> {
	match result {
		Ok(response) => response.into_json::<GitHubRelease>().map_err(|err| {
			CliError::new(
				"update",
				format!("invalid release metadata from GitHub: {err}"),
				Some("try again later or install from source".to_owned()),
			)
		}),
		Err(ureq::Error::Status(status, _)) => Err(CliError::new(
			"update",
			format!("GitHub release lookup failed with HTTP {status}"),
			Some("check the requested version tag or try again later".to_owned()),
		)),
		Err(ureq::Error::Transport(err)) => Err(CliError::new(
			"transport",
			format!("cannot reach GitHub releases: {err}"),
			Some("check network access or install from source".to_owned()),
		)),
	}
}

fn download_bytes(url: &str, max_bytes: u64) -> Result<Vec<u8>, CliError> {
	let response = ureq::get(url)
		.set("User-Agent", USER_AGENT)
		.call()
		.map_err(|err| match err {
			ureq::Error::Status(status, _) => CliError::new(
				"update",
				format!("download failed with HTTP {status}: {url}"),
				Some("try again later or install from source".to_owned()),
			),
			ureq::Error::Transport(err) => CliError::new(
				"transport",
				format!("download failed: {err}"),
				Some("check network access or install from source".to_owned()),
			),
		})?;
	if let Some(len) = response.header("Content-Length").and_then(|v| v.parse::<u64>().ok())
		&& len > max_bytes
	{
		return Err(CliError::new(
			"update",
			format!("download is too large ({len} bytes; limit {max_bytes})"),
			None,
		));
	}
	let mut reader = response.into_reader().take(max_bytes + 1);
	let mut bytes = Vec::new();
	reader
		.read_to_end(&mut bytes)
		.map_err(|err| CliError::new("io", format!("failed to read download: {err}"), None))?;
	if bytes.len() as u64 > max_bytes {
		return Err(CliError::new(
			"update",
			format!("download exceeded maximum size of {max_bytes} bytes"),
			None,
		));
	}
	Ok(bytes)
}

pub fn verify_checksum(bytes: &[u8], checksum_text: &str) -> Result<(), CliError> {
	let expected = checksum_text
		.split_whitespace()
		.next()
		.ok_or_else(|| CliError::new("update", "release checksum file is empty", None))?
		.to_ascii_lowercase();
	if expected.len() != 64 || !expected.chars().all(|c| c.is_ascii_hexdigit()) {
		return Err(CliError::new(
			"update",
			"release checksum file does not start with a SHA-256 hex digest",
			None,
		));
	}
	let actual = hex_sha256(bytes);
	if actual != expected {
		return Err(CliError::new(
			"checksum_mismatch",
			"downloaded wikid binary does not match the release checksum",
			Some("refusing to replace the current binary; try again later".to_owned()),
		));
	}
	Ok(())
}

fn hex_sha256(bytes: &[u8]) -> String {
	let digest = Sha256::digest(bytes);
	let mut out = String::with_capacity(64);
	for byte in digest {
		out.push_str(&format!("{byte:02x}"));
	}
	out
}

fn current_exe() -> Result<PathBuf, CliError> {
	std::env::current_exe()
		.and_then(|path| path.canonicalize())
		.map_err(|err| CliError::new("io", format!("cannot locate current wikid executable: {err}"), None))
}

fn ensure_parent_writable(exe: &Path) -> Result<(), CliError> {
	let parent = exe.parent().ok_or_else(|| {
		CliError::new(
			"update",
			format!("cannot determine install directory for {}", exe.display()),
			None,
		)
	})?;
	let probe = parent.join(format!(".wikid-update-probe-{}", std::process::id()));
	match File::create(&probe).and_then(|mut file| file.write_all(b"probe")) {
		Ok(()) => {
			let _ = fs::remove_file(&probe);
			Ok(())
		}
		Err(err) => Err(CliError::new(
			"permission_denied",
			format!("install directory is not writable: {} ({err})", parent.display()),
			Some("run the installer with appropriate permissions or install into a user-writable directory".to_owned()),
		)),
	}
}

fn replace_executable(exe: &Path, bytes: &[u8]) -> Result<(), CliError> {
	let parent = exe.parent().ok_or_else(|| {
		CliError::new(
			"update",
			format!("cannot determine install directory for {}", exe.display()),
			None,
		)
	})?;
	let tmp = parent.join(format!(".wikid-update-{}", std::process::id()));
	let write_result = (|| -> std::io::Result<()> {
		let mut file = File::create(&tmp)?;
		file.write_all(bytes)?;
		file.sync_all()?;
		#[cfg(unix)]
		{
			use std::os::unix::fs::PermissionsExt as _;
			fs::set_permissions(&tmp, fs::Permissions::from_mode(0o755))?;
		}
		fs::rename(&tmp, exe)?;
		Ok(())
	})();
	if let Err(err) = write_result {
		let _ = fs::remove_file(&tmp);
		return Err(CliError::new(
			"io",
			format!("failed to replace {}: {err}", exe.display()),
			Some("current binary was left untouched if replacement did not complete".to_owned()),
		));
	}
	Ok(())
}

fn parse_version(input: &str) -> Result<Version, CliError> {
	let normalized = input.strip_prefix('v').unwrap_or(input);
	let mut parts = normalized.split('.');
	let major = parse_part(parts.next(), input)?;
	let minor = parse_part(parts.next(), input)?;
	let patch = parse_part(parts.next(), input)?;
	if parts.next().is_some() {
		return Err(bad_version(input));
	}
	Ok(Version { major, minor, patch })
}

fn parse_part(part: Option<&str>, original: &str) -> Result<u64, CliError> {
	let Some(part) = part else {
		return Err(bad_version(original));
	};
	if part.is_empty() || !part.chars().all(|c| c.is_ascii_digit()) {
		return Err(bad_version(original));
	}
	part.parse::<u64>().map_err(|_| bad_version(original))
}

fn bad_version(version: &str) -> CliError {
	CliError::new(
		"bad_version",
		format!("version {version:?} is not a simple vMAJOR.MINOR.PATCH tag"),
		Some("use a release tag like v0.2.0".to_owned()),
	)
}

fn normalize_tag(tag: &str) -> String {
	if tag.starts_with('v') {
		tag.to_owned()
	} else {
		format!("v{tag}")
	}
}

pub fn render(result: &UpdateResult) -> String {
	match result.action {
		UpdateAction::UpToDate => format!(
			"wikid is up to date: {}\nhint: wikid update --check --json — machine-readable update check",
			result.current
		),
		UpdateAction::WouldUpdate => format!(
			"update available: {} -> {}\nhint: wikid update — download and install {}",
			result.current, result.target, result.target
		),
		UpdateAction::Updated => format!(
			"updated wikid: {} -> {}\nhint: wikid --version — confirm the installed version",
			result.current, result.target
		),
		UpdateAction::Reinstalled => format!(
			"installed wikid: {}\nhint: wikid --version — confirm the installed version",
			result.target
		),
	}
}

#[cfg(test)]
mod tests {
	use super::*;

	fn asset(name: &str) -> ReleaseAsset {
		ReleaseAsset {
			name: name.to_owned(),
			browser_download_url: format!("https://example.test/{name}"),
		}
	}

	fn release(tag: &str, target: &str) -> GitHubRelease {
		let asset_name = format!("wikid-{target}");
		GitHubRelease {
			tag_name: tag.to_owned(),
			assets: vec![asset(&asset_name), asset(&format!("{asset_name}.sha256"))],
		}
	}

	#[test]
	fn parses_and_orders_versions() {
		assert_eq!(
			parse_version("v1.2.3").unwrap(),
			Version {
				major: 1,
				minor: 2,
				patch: 3
			}
		);
		assert_eq!(
			parse_version("0.1.0").unwrap(),
			Version {
				major: 0,
				minor: 1,
				patch: 0
			}
		);
		assert!(parse_version("v1.2").is_err());
		assert!(parse_version("v1.2.x").is_err());
		assert!(parse_version("v1.2.3.4").is_err());
		assert!(parse_version("0.2.0").unwrap() > parse_version("0.1.9").unwrap());
	}

	#[test]
	fn target_triple_is_one_of_published_assets_on_supported_hosts() {
		let target = target_triple().unwrap();
		assert!(
			[
				"x86_64-unknown-linux-gnu",
				"aarch64-unknown-linux-gnu",
				"x86_64-apple-darwin",
				"aarch64-apple-darwin",
			]
			.contains(&target)
		);
	}

	#[test]
	fn resolve_plan_handles_up_to_date_update_and_newer_current() {
		let target = "x86_64-apple-darwin";
		assert_eq!(
			resolve_plan("0.1.0", target, &release("v0.1.0", target), false, None).unwrap(),
			UpdatePlan::UpToDate {
				tag: "v0.1.0".to_owned()
			}
		);
		match resolve_plan("0.1.0", target, &release("v0.2.0", target), false, None).unwrap() {
			UpdatePlan::Update { tag, asset, checksum } => {
				assert_eq!(tag, "v0.2.0");
				assert_eq!(asset.name, "wikid-x86_64-apple-darwin");
				assert_eq!(checksum.name, "wikid-x86_64-apple-darwin.sha256");
			}
			other => panic!("unexpected plan: {other:?}"),
		}
		let err = resolve_plan("0.3.0", target, &release("v0.2.0", target), false, None).unwrap_err();
		assert_eq!(err.code, "version_mismatch");
	}

	#[test]
	fn requested_version_allows_downgrade_or_reinstall() {
		let target = "x86_64-apple-darwin";
		assert!(matches!(
			resolve_plan("0.3.0", target, &release("v0.2.0", target), false, Some("v0.2.0")).unwrap(),
			UpdatePlan::Update { .. }
		));
		assert_eq!(
			resolve_plan("0.2.0", target, &release("v0.2.0", target), false, Some("v0.2.0")).unwrap(),
			UpdatePlan::UpToDate {
				tag: "v0.2.0".to_owned()
			}
		);
		assert!(matches!(
			resolve_plan("0.2.0", target, &release("v0.2.0", target), true, None).unwrap(),
			UpdatePlan::Update { .. }
		));
	}

	#[test]
	fn missing_asset_is_structured_error() {
		let release = GitHubRelease {
			tag_name: "v0.2.0".to_owned(),
			assets: vec![],
		};
		let err = resolve_plan("0.1.0", "x86_64-apple-darwin", &release, false, None).unwrap_err();
		assert_eq!(err.code, "missing_release_asset");
	}

	#[test]
	fn verifies_sha256_files() {
		let bytes = b"wikid";
		let checksum = format!("{}  wikid-x86_64-apple-darwin\n", hex_sha256(bytes));
		verify_checksum(bytes, &checksum).unwrap();
		let err = verify_checksum(
			bytes,
			"0000000000000000000000000000000000000000000000000000000000000000  wikid\n",
		)
		.unwrap_err();
		assert_eq!(err.code, "checksum_mismatch");
		assert_eq!(verify_checksum(bytes, "not-a-hash").unwrap_err().code, "update");
	}
}
