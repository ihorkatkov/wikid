# Changelog

All notable user-facing changes to `wikid` are documented here.

## [0.2.0] - 2026-07-08

### Added

- Embedded agent usage guides via `wikid skills`, including version-matched `core` and `llm-wiki` guides that can be printed, materialized, and wired into Claude Code skills.
- `wikid update` for explicit local binary updates from verified GitHub release assets, plus `--check` support.
- Hash-guarded line editing with `cat --hashes`, `edit`, and `edit-batch` so agents can make surgical edits without silently clobbering stale lines.
- Line-window reads with `cat --lines START-END` for oversized pages and logs.
- Obsidian-compatible link improvements: aliases, embeds, heading fragments, block references, markdown anchor links, relative/root markdown paths, attachment-folder config, and links inside callouts.
- `tags` command for frontmatter and inline tag discovery, including implied nested-tag ancestors.
- Callout metadata extraction and shared markdown fence scanning for links, tags, and doctor checks.
- LLM-wiki oriented doctor policy improvements for authored pages, raw captures, meta pages, duplicate stems, fragments, oversized pages, and malformed frontmatter.

### Changed

- Bare `wikid` / `wikid status` output is more agent-friendly and remote status now includes the server-side root path in JSON so clients do not mistake it for a local filesystem path.
- `init` and config targeting now honor explicit destination directories more consistently and avoid dirtying the caller's current directory accidentally.
- CLI help now points agents to `wikid skills get core` before the flag list.
- `README.md`, `AGENTS.md`, `docs/SPEC.md`, and `docs/DESIGN.md` now describe the v0.2 operating model: plain files, dumb runtime, remote filesystem-feeling operations, and version-matched agent guides.
- Release packaging now builds current Linux/macOS targets and the installer overwrites existing binaries safely.

### Fixed

- Doctor false positives from raw/meta wiki areas and Obsidian constructs are reduced.
- Markdown links inside fenced and inline code are ignored correctly.
- Fragment reads, link resolution, tag counting, and duplicate-stem reporting better match real Obsidian vault behavior.
- Skills status rendering is compact for humans while preserving absolute paths in JSON.

### Notes

- The runtime still intentionally uses plain files, atomic writes, and last-write-wins semantics. History, backup, and undo remain the substrate's responsibility.
- MCP remains the next milestone; CLI, HTTP, and operation-core behavior are the stable base for that adapter.

## [0.1.0] - 2026-07-05

Initial public MVP release:

- Local and remote access to plain-Markdown wiki directories.
- `serve`, `status`, `ls`, `tree`, `cat`, `grep`, `glob`, `write`, `mv`, `rm`, `links`, and `doctor`.
- Multi-wiki daemon config with named bearer tokens.
- Atomic writes and structural health checks for LLM-wiki style vaults.
- GitHub release workflow and source installer.
