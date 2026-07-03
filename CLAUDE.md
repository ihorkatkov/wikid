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

Hooks live in `.hooks/` (pre-commit runs fmt/clippy/build/test/doc with warnings as errors; commit-msg enforces conventional commits). `bd init` copies them into `.beads/hooks/` with beads integration appended and points `core.hooksPath` there — on a fresh clone run `bd init` (or `git config core.hooksPath .hooks` if not using beads). If you edit `.hooks/`, re-apply the change to the `.beads/hooks/` copy. Never bypass with `--no-verify`.

## Design authority

`docs/DESIGN.md` is the implementation blueprint (API shapes, error codes, CLI flags, HTTP routes, locked dependencies). Code deviating from it must update it in the same change. Do not add dependencies beyond its §9 list.

## Conventions

- Tests co-located in `#[cfg(test)]` modules.
- Hard tabs for indentation (rustfmt default here is spaces — this repo uses `rustfmt.toml` with `hard_tabs = true`).
- Conventional commits: `feat:`, `fix:`, `chore:`, `refactor:`, `test:`, `docs:`.


<!-- BEGIN BEADS INTEGRATION v:1 profile:minimal hash:7510c1e2 -->
## Beads Issue Tracker

This project uses **bd (beads)** for issue tracking. Run `bd prime` to see full workflow context and commands.

### Quick Reference

```bash
bd ready              # Find available work
bd show <id>          # View issue details
bd update <id> --claim  # Claim work
bd close <id>         # Complete work
```

### Rules

- Use `bd` for ALL task tracking — do NOT use TodoWrite, TaskCreate, or markdown TODO lists
- Run `bd prime` for detailed command reference and session close protocol
- Use `bd remember` for persistent knowledge — do NOT use MEMORY.md files

**Architecture in one line:** issues live in a local Dolt DB; sync uses `refs/dolt/data` on your git remote; `.beads/issues.jsonl` is a passive export. See https://github.com/gastownhall/beads/blob/main/docs/SYNC_CONCEPTS.md for details and anti-patterns.

## Session Completion

**When ending a work session**, you MUST complete ALL steps below. Work is NOT complete until `git push` succeeds.

**MANDATORY WORKFLOW:**

1. **File issues for remaining work** - Create issues for anything that needs follow-up
2. **Run quality gates** (if code changed) - Tests, linters, builds
3. **Update issue status** - Close finished work, update in-progress items
4. **PUSH TO REMOTE** - This is MANDATORY:
   ```bash
   git pull --rebase
   git push
   git status  # MUST show "up to date with origin"
   ```
5. **Clean up** - Clear stashes, prune remote branches
6. **Verify** - All changes committed AND pushed
7. **Hand off** - Provide context for next session

**CRITICAL RULES:**
- Work is NOT complete until `git push` succeeds
- NEVER stop before pushing - that leaves work stranded locally
- NEVER say "ready to push when you are" - YOU must push
- If push fails, resolve and retry until it succeeds
<!-- END BEADS INTEGRATION -->
