# CLAUDE.md

wikid: a single Rust binary that exposes plain-Markdown wiki directories to remote agents over CLI and MCP. Read `docs/SPEC.md` (v0.2) before making design decisions — it records what was deliberately cut from v0.1 and why. Do not reintroduce cut features (git integration, databases, proposals/review, sessions, attribution) without an explicit product decision.

## Core invariants

- A wiki is any directory of Markdown files. An existing Obsidian vault must work with zero setup or migration.
- The runtime holds no state that isn't derivable from the files. Indexes are rebuildable caches.
- Writes are atomic (temp file + rename), last-write-wins. No locks, no versions, no git, no DB.
- Every operation is available identically via CLI, HTTP, and MCP — thin views over one operation core in `wikid-core`.
- CLI output follows the AXI principles (https://axi.md/): content-first no-arg default, token-efficient output, 3–4 fields per list item, truncation with size hints, structured errors on stdout, exit codes 0/1/2, never prompt interactively, contextual next-step hints.

## Workspace

- `crates/wikid-core` — vault model, wikilink graph, file ops, health checks. No I/O framework deps.
- `crates/wikid-server` — axum HTTP daemon, bearer-token auth, MCP (later).
- `crates/wikid` — the `wikid` binary: clap CLI, client for remote daemons, local mode, `serve`.

## Commands

- Build: `cargo build`
- Test: `cargo test`
- Lint: `cargo clippy --workspace --all-targets --all-features -- -D warnings`
- Format: `cargo fmt --all`
- Docs check: `RUSTDOCFLAGS="-D warnings" cargo doc --workspace --no-deps --document-private-items`

## Git hooks

Hooks live in `.hooks/` (pre-commit runs fmt/clippy/build/test/doc with warnings as errors; commit-msg enforces conventional commits). Install once per clone: `git config core.hooksPath .hooks`. Never bypass with `--no-verify`.

## Design authority

`docs/DESIGN.md` is the implementation blueprint (API shapes, error codes, CLI flags, HTTP routes, locked dependencies). Code deviating from it must update it in the same change. Do not add dependencies beyond its §9 list.

## Conventions

- Tests co-located in `#[cfg(test)]` modules.
- Hard tabs for indentation (rustfmt default here is spaces — this repo uses `rustfmt.toml` with `hard_tabs = true`).
- Conventional commits: `feat:`, `fix:`, `chore:`, `refactor:`, `test:`, `docs:`.
