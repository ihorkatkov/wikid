#!/usr/bin/env bash
set -euo pipefail

# wikid installer
#
# One-line usage:
#   curl -fsSL https://wikid/install.sh | bash
#
# Private/source usage:
#   WIKID_REPO=git@github.com:ihorkatkov/wikid.git curl -fsSL https://wikid/install.sh | bash
#   ./install.sh
#
# Environment:
#   WIKID_REPO          Git repository to install from when not running inside a checkout.
#                       Default: https://github.com/ihorkatkov/wikid.git
#   WIKID_REF           Branch name to install. Default: main
#   WIKID_TAG           Tag to install instead of WIKID_REF.
#   WIKID_REV           Commit SHA to install instead of WIKID_REF/WIKID_TAG.
#   WIKID_LOCAL_PATH    Local repository checkout to install from. Default: inferred from this script when possible.
#   WIKID_BIN_DIR       Expected cargo bin directory. Default: $CARGO_HOME/bin or ~/.cargo/bin.
#   WIKID_NO_RUSTUP     Set to 1 to fail instead of installing Rust when cargo is missing.

log() {
	printf 'wikid-install: %s\n' "$*"
}

fail() {
	printf 'wikid-install: error: %s\n' "$*" >&2
	exit 1
}

need_cmd() {
	command -v "$1" >/dev/null 2>&1 || fail "missing required command: $1"
}

if ! command -v cargo >/dev/null 2>&1; then
	[[ "${WIKID_NO_RUSTUP:-}" != 1 ]] || fail "missing required command: cargo"
	need_cmd curl
	log "cargo not found; installing Rust with rustup"
	curl --proto '=https' --tlsv1.2 -fsSL https://sh.rustup.rs | sh -s -- -y --profile minimal
	# shellcheck source=/dev/null
	. "${CARGO_HOME:-$HOME/.cargo}/env"
fi

need_cmd cargo
need_cmd git

repo=${WIKID_REPO:-https://github.com/ihorkatkov/wikid.git}
ref=${WIKID_REF:-main}
bin_dir=${WIKID_BIN_DIR:-${CARGO_HOME:-$HOME/.cargo}/bin}

script_path=${BASH_SOURCE[0]:-}
script_dir=''
if [[ -n "$script_path" && "$script_path" != bash && "$script_path" != /dev/fd/* && "$script_path" != /proc/* ]]; then
	script_dir=$(cd "$(dirname "$script_path")" && pwd)
fi

local_path=${WIKID_LOCAL_PATH:-}
if [[ -z "$local_path" && -n "$script_dir" && -f "$script_dir/crates/wikid/Cargo.toml" ]]; then
	local_path=$script_dir
fi

install_args=(install --locked)
if [[ -n "$local_path" ]]; then
	[[ -f "$local_path/crates/wikid/Cargo.toml" ]] || fail "WIKID_LOCAL_PATH is not a wikid checkout: $local_path"
	log "installing from local checkout: $local_path"
	install_args+=(--path "$local_path/crates/wikid")
else
	log "installing from git: $repo"
	install_args+=(--git "$repo")
	if [[ -n "${WIKID_REV:-}" ]]; then
		install_args+=(--rev "$WIKID_REV")
	elif [[ -n "${WIKID_TAG:-}" ]]; then
		install_args+=(--tag "$WIKID_TAG")
	else
		install_args+=(--branch "$ref")
	fi
	install_args+=(wikid)
fi

cargo "${install_args[@]}"

[[ -x "$bin_dir/wikid" ]] || fail "wikid was not installed at $bin_dir/wikid"

log "installed wikid: $($bin_dir/wikid --version)"
if [[ -x "$bin_dir/boxd-worktree" ]]; then
	log "installed boxd-worktree: $bin_dir/boxd-worktree"
fi
if [[ ":$PATH:" != *":$bin_dir:"* ]]; then
	log "add this to your shell profile: export PATH=\"$bin_dir:\$PATH\""
fi
log "done"
