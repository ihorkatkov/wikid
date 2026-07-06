# wikid DESIGN — MVP implementation blueprint

Companion to [SPEC.md](SPEC.md). SPEC says *what*; this locks *how*. Implementation follows this document; deviations require updating this document in the same change.

## 1. Crate responsibilities

- **`wikid-core`** — everything that touches files: vault model, path safety, file operations, wikilink graph, health checks, status aggregation. Sync, no async runtime, no HTTP. All public result types derive `Serialize + Deserialize` — they ARE the wire format (CLI `--json`, HTTP responses, remote client parsing share these structs).
- **`wikid-server`** — axum router + bearer-token auth + TOML config. A thin HTTP view over `wikid-core`. Exposes `pub fn app(state) -> axum::Router` so tests drive it with `tower::ServiceExt::oneshot` without binding a port.
- **`wikid`** (binary) — clap CLI. Two modes sharing one command surface: **local** (calls `wikid-core` directly against a directory) and **remote** (HTTP client via `ureq` against a daemon). Also hosts `wikid serve` (starts `wikid-server` on tokio).

## 2. Core: path rules and safety

- User-facing paths are always **relative to the vault root**, forward slashes, e.g. `projects/alpha.md`.
- Rejected with `InvalidPath`: absolute paths, `..` components (checked after lexical normalization), empty paths, paths whose normalized form escapes the root.
- Vault root is canonicalized at `Vault::open`. Operation targets are lexically joined; when the target exists, a defensive `canonicalize().starts_with(root)` check also applies (symlinks pointing outside the vault are refused).
- **Ignore rules** (all read operations: ls/tree/grep/glob/links/doctor/status): skip hidden files and dot-directories at any depth (`.obsidian/`, `.git/`, `.trash/`, …) via the `ignore` crate (`WalkBuilder`, hidden-filter on, gitignore respected). `cat`/`write`/`edit`/`mv`/`rm` accept explicit non-hidden paths only (same `InvalidPath` for dotted components).
- A **page** is a `.md` file. Other files (attachments) appear in `ls`/`glob` and can be `cat`ed if valid UTF-8. `grep` searches pages plus UTF-8 text attachments; binary files (sniff: NUL byte in first 8 KiB) are skipped by `grep` and by `links`/`doctor` content checks (which only scan pages).
- Symlinked entries encountered by the walker that resolve outside the vault root (or dangle) are skipped by all read operations — the walker never follows or lists content beyond the root. In-vault symlinks are listed normally.

## 3. Core: operations

Public API (`Vault` methods; all return `Result<T, WikidError>`):

| Method | Semantics |
|---|---|
| `ls(path: Option<&str>, depth: usize)` | Listing of dirs (trailing `/`) + files at `path` (default root), recursing to `depth` (default 1; `tree` = depth 3). Entries: `path`, `kind` (`dir`/`file`/`page`), `size`, `modified` (RFC3339 UTC). Includes `total_dirs`, `total_files`, `total_pages` for the whole subtree regardless of depth (AXI: pre-computed aggregates). |
| `cat(path, limit: Option<ReadLimit>)` / `cat_with_range(path, limit, range)` | Returns `Document { path, content, truncated, range_start?, range_end?, total_lines, total_bytes, modified }`. Default limit 400 lines or 32 KiB (whichever first); `None` = full. A line range is 1-based inclusive, disables truncation, clamps end to EOF, and preserves whole-file totals. Paths may include `#Heading` (heading section through the next same-or-higher ATX heading) or `#^block-id` (anchor line only); missing fragments return `fragment_not_found` rather than page-not-found. |
| `cat_hashes(path, limit: Option<ReadLimit>)` / `cat_hashes_with_range(path, limit, range)` | Same read (same limits/truncation/range), returned as `HashlinesResult { path, lines: [{line, hash, text}], truncated, range_start?, range_end?, total_lines, total_bytes, modified }`. `hash` = first 12 hex chars of SHA-256 of the line text (no EOL). Windowed and fragment reads keep absolute file line numbers so this remains the read half of the edit protocol. |
| `grep(pattern, opts)` | Regex search (`regex` crate) over pages + UTF-8 text files. Options: `ignore_case`, `files_only`, `context` (lines), `limit` (default 50 matches). Result: `matches: [{path, line, text}]` (+ `context_before/after` when requested), `total_matches`, `matched_files` (files with ≥1 match), `total_files` (files searched), `truncated`. Files whose path stem matches the pattern are ranked first. |
| `glob(pattern)` | `globset` match over relative paths, e.g. `**/*.md`. Sorted by path. Returns entries + `total`. |
| `write(path, content)` | Create or overwrite. Creates parent dirs. **Atomic**: `tempfile::NamedTempFile::new_in(parent)` + `persist`. Returns `{path, created: bool, bytes}`. |
| `edit(path, edits: &[LineEdit])` | Hash-guarded line replacement (`LineEdit { line, expected_hash, new_text }`, 1-based lines). Structural problems (empty batch, out-of-range line, duplicate line) → `BadEdit`. Every `expected_hash` must match the current line's hash (comparison is ASCII-case-insensitive); any mismatch → `StaleEdit` naming every stale line, and the whole batch is refused — all-or-nothing. `new_text` may contain `\n` (one line expands into many). EOL style (LF/CRLF) and trailing-newline presence are preserved. Write is atomic as above; returns `{path, replacements, bytes}` where `replacements` counts lines replaced. CLI exposes single-line `edit` and JSON-stdin `edit-batch`; both call this same method. |
| `mv(from, to, force)` | Rename file (not dirs). Creates parent dirs at destination. Destination exists and `!force` → `AlreadyExists`. |
| `rm(path)` | Delete file (not dirs). The `--force` gate is CLI/HTTP-level, not core. |
| `links(path)` | `LinkReport { outgoing: [{raw, target, resolved: Option<path>, kind: wikilink/markdown, embed: bool, fragment?}], backlinks: [path] }`. Backlinks = scan all pages for links resolving to `path`. |
| `tags()` | `TagReport { tags: [{tag, count, pages, implied?}] }` across visible Markdown pages. Inline tags use Obsidian's `#tag` grammar (letters/digits/`_`/`-`/`/`, not preceded by a word char), excluding wikilink fragments, ATX headings, fenced code blocks, inline code spans, and numeric-only tags. Frontmatter `tags` accepts a string or list; leading `#` is stripped; dedupe is case-insensitive while preserving first-authored case. Counts are total occurrences (each inline occurrence and each frontmatter entry counts once). Nested tags imply ancestor entries whose counts/pages aggregate all descendant and literal occurrences; `implied: true` is present only when the ancestor never appears literally. |
| `doctor(opts)` | See §5. |
| `status()` | `VaultStatus { version, root, total_pages, total_files, total_bytes, recent: [{path, modified}] (5 most recent pages), doctor_summary: {high, medium, low} }`. |

**Error model** (`thiserror` enum `WikidError`): `NotFound`, `InvalidPath`, `AlreadyExists`, `StaleEdit`, `BadEdit`, `NotUtf8`, `BadPattern`, `Io`. Each maps to a stable string `code()` and an optional `hint()` (used verbatim by CLI and HTTP error bodies).

## 4. Core: link model (Obsidian-compatible)

- Extract `[[Target]]`, `[[Target|alias]]`, `[[Target#Heading]]`, `[[Target#^block-id]]`, embeds (`![[Target]]`, `![alt](relative/path)`), and markdown links `[text](relative/path.md)` (skip `http(s)://`, `mailto:`). Fragments are retained on links but ignored for target resolution.
- Fragment-only links are self-links (matches Obsidian): `[[#Heading]]`, `[[#^block-id]]`, `![[#Heading]]`, and markdown anchors `[text](#Heading)` / `[text](#^block-id)` emit `target: ""`, `resolved` = the containing page, fragment kept (percent-decoded for markdown anchors); doctor validates the fragment against the containing page.
- Markdown link paths resolve like Obsidian: `./x.md` and `../x.md` are relative to the containing file's directory, `/x.md` is from the vault root, and a bare `x.md` is vault-wide with a root-level match winning over deeper ones. `../` escaping the vault root → unresolved.
- Wikilink resolution, in order: (1) exact relative path from root (with/without `.md`); (2) if `.obsidian/app.json` has `attachmentFolderPath`, an exact file under that configured folder for the target; (3) unique file-stem match anywhere in the vault (case-insensitive); (4) unique path-suffix match (`folder/Note`); (5) unique frontmatter alias match from `aliases` (string or list, case-insensitive). Real root path matches win first; configured attachment-folder matches can disambiguate duplicate attachment filenames; file/path/stem matches win before aliases. Multiple candidates at a stage → unresolved + flagged `ambiguous` (doctor reports it). No match → broken link. Alias resolution (5) is a deliberate wikid extension — real Obsidian leaves bare `[[Alias]]` unresolved.
- Frontmatter: leading `---\n…\n---` block parsed with `serde_yaml` into a string-keyed map. Absence is normal. Malformed YAML → treated as no frontmatter; doctor flags it with a sanitized one-line YAML detail and line number when available.
- Obsidian config: only `.obsidian/app.json`'s `attachmentFolderPath` is honored, via a targeted read of that known file. Missing `.obsidian/`, missing app.json, malformed JSON, or invalid paths degrade to no config; `.obsidian/` remains hidden from normal ls/tree/grep/page walks.
- Page title: frontmatter `title` → first `# heading` → file stem.

## 5. Core: doctor checks

All structural, no LLM. Each issue: `{check, severity (low/medium/high), category, path, detail, suggested_action}`. Report includes per-check counts and a one-line summary. Exit code 0 even with findings (it's a report; `--fail-on <severity>` can gate CI later — not in MVP).

`DoctorOptions.profile` defaults to `llm-wiki`, an opinionated LLM Wiki policy: authored paths are `entities/**`, `concepts/**`, `questions/**`, `syntheses/**`, `queries/**`, and `meetings/**`; root meta/navigation pages are `SCHEMA.md`, `index.md`, and `log.md`; raw captures are `raw/**`; assets are `raw/assets/**`. When an LLM Wiki layout is detected, the default profile lints authored pages plus root meta/navigation pages only. Non-authored notes (`raw/**`, assets, generated exports, and other non-authored subtrees) are excluded from default linting. Root meta pages are excluded from missing-frontmatter adoption and findings. Duplicate-stem findings are reported only when the duplicate group contains an authored/root page; page/page remains medium, page/asset is low, and asset/asset is suppressed. `--profile strict` disables those scope rules while preserving engine-level URL parsing behavior. Human output groups issues as `authored_pages`, `raw_source`, `asset_hygiene`, `graph_navigation`, and `size_performance`; JSON carries full issues without human truncation.

| Check | Trigger | Severity |
|---|---|---|
| `broken_links` | link resolves to nothing | high |
| `ambiguous_links` | link target matches >1 file/stem/suffix; duplicate-alias ambiguity is reported by `duplicate_aliases` instead | medium |
| `duplicate_aliases` | two or more pages claim the same case-insensitive frontmatter alias, even if no link references it | low |
| `orphan_pages` | page with no inbound links, excluding root-level `index.md`/`README.md` | low |
| `broken_block_reference` | resolved page link has a `#^block-id` fragment, but the target page has no trailing `^block-id` anchor | medium |
| `broken_heading_reference` | resolved page link has a `#heading` fragment, but the target page has no matching ATX heading (case-insensitive trimmed text) | medium |
| `missing_frontmatter` | only when ≥50% of pages have frontmatter (the vault "uses" it) — pages without it | low |
| `malformed_frontmatter` | `---` block present but YAML parse/type-check fails; issue detail is a sanitized one-liner like `invalid YAML frontmatter (line N): <reason>` | medium |
| `stale_pages` | mtime older than `stale_days` (default 90) | low |
| `oversized_pages` | > 64 KiB or > 1500 lines | medium |
| `duplicate_stems` | same case-insensitive stem at multiple paths (breaks wikilink resolution); default `llm-wiki` profile keeps page/page as medium, page/asset as low, and suppresses asset/asset noise | medium/low |

## 6. CLI

**Targeting.** Local mode: `--dir <path>` or `$WIKID_DIR`. Remote mode: `--server <url>` + `--token <t>` + `--wiki <name>` or `$WIKID_SERVER`/`$WIKID_TOKEN`/`$WIKID_WIKI`. Explicit flags win over env; opposite-mode env vars are ignored when an explicit local/remote flag is present. Env-only local+remote targets → usage error. If neither local nor remote targeting is selected, commands fall back to config discovery (`--config`, `$WIKID_CONFIG`, `./wikid.toml`, `~/.config/wikid/config.toml`) and use local mode by choosing the registered wiki whose canonical path contains cwd (longest prefix), else the only registered wiki, else `default_wiki`. Multiple registered wikis with no valid default → structured `ambiguous_wiki` listing names and hinting to set `default_wiki` or pass a target. Remote mode maps commands 1:1 onto the HTTP API and renders identically (deserializes the shared core structs). `--wiki` is required in remote mode (missing → structured `no_wiki` error); `--token` is optional so auth-less loopback daemons work. HTTP error bodies re-render verbatim as the same `error[<code>]` with exit 1; connection/decode failures use the CLI-level `transport` code, unrecognized error bodies `http`.

**Output.** Human-readable compact text by default; `--json` on every command emits the core result struct as JSON (one object, stdout). Client-side CLI-only commands such as `skills` emit their own stable structs. Errors (both modes): stdout, exit 1, format `error[<code>]: <message>` + optional `hint: …` line; `--json` errors: `{"error":{"code","message","hint"}}`. Usage errors: clap default (exit 2).

**AXI conformance checklist** (each item is a test):
1. `wikid` with no args = `status` — live data, never help text.
2. List items show ≤4 fields; totals always present.
3. `cat` truncates by default with `… truncated (N lines / M bytes total) — use --full`.
4. Zero results are explicit: `no matches for "…" in N files`.
5. Structured errors on stdout; exit codes 0/1/2; **grep exits 1 on zero matches** (coreutils-faithful).
6. Never prompts interactively; `rm` requires `--force` (refusal is a structured error, not a question).
7. Human output ends with 1–2 `hint:` lines suggesting next commands with `<placeholders>` (e.g. after `grep`: `hint: wikid cat <path> — read a match`). No hints in `--json`.

**Commands and flags** (coreutils-faithful; unknown flags fail with exit 2 and clap's suggestion):

- `skills [list]` — client-side catalog of embedded agent usage guides. Human output is `name — description`, wrapped near 72 columns, followed by `total: N guides` and the hint line ``hint: `wikid skills get core` to read one; add --full for the complete reference``. JSON shape: `{skills:[{name, description}]}`. This command never targets a wiki and does not contact the server.
- `skills get <name> [--full]` — print the embedded `SKILL.md` verbatim, including YAML frontmatter. `--full` appends each reference document after the body, introduced by `# Reference: <title>`. JSON shape: `{name, full, content}` where `content` is the exact human text that would have been printed. Unknown names return structured `not_found` with a did-you-mean hint.
- `skills path [<name>]` — materialize all embedded skills to `$XDG_DATA_HOME/wikid/skills/<version>/`, falling back to `~/.local/share/wikid/skills/<version>/`; on macOS, honor XDG only when set and otherwise use the same fallback for stable cross-platform symlinks. The layout mirrors repo `skills/` (`core/SKILL.md`, `core/references/*.md`, `llm-wiki/SKILL.md`). Writes are idempotent: if the version dir exists with `.complete`, no files are rewritten. Otherwise the CLI writes a temp dir and renames it into place. It also maintains a `current` symlink in `.../wikid/skills/` pointing at the version dir and leaves old version dirs in place. Human output is the `current` path, or `current/<name>` when named, so symlink wiring follows future wikid updates. JSON shape: `{path, version, versioned_path}` where `versioned_path` preserves the concrete version dir.
- `skills status` — read-only installation report for embedded guide count/version, materialized cache state, and Claude skill symlink wiring. It exits 0 even when nothing is materialized or wired. Human output abbreviates local `$HOME` paths as `~`, renders wiring targets relative to the skills data dir when possible (`current/core`, `<version>/llm-wiki`), aligns state words in a fixed column, and wraps at the same width as the skills catalog. JSON shape stays absolute and machine-oriented: `{embedded:{version,guides}, materialized:{path,current,version_present,stale_versions}, wiring:[{link,target,state}]}`.
- `init [path]` — create a blank LLM Wiki scaffold and register it in config. Target precedence is explicit positional `path`, then global `--dir`, then `$WIKID_DIR`, then cwd. It creates `index.md`, `log.md`, `AGENTS.md`, and `raw/`, `raw/assets/`, `concepts/`, `entities/`, `questions/`, `syntheses/`; never overwrites existing files; works in non-empty directories and Obsidian vaults; writes/updates config idempotently. Generated `AGENTS.md` points agents at `wikid skills get core` instead of inlining CLI usage prose.
- `token show [actor]` — explicit secret-revealing local command. Defaults to `admin`; prints exactly one matching token or errors on none/multiple. JSON shape: `{actor, token, config_path}`.
- `update` `--check` `--force` `--version <vX.Y.Z>` — explicit self-update for the installed `wikid` binary. It queries GitHub releases, selects the raw `wikid-<target>` asset for the current supported target, verifies the sibling `.sha256`, writes a temp file next to the current executable, chmods it executable, and atomically renames it over the current binary. No background checks, no cache, no prompts, no remote daemon update. JSON shape: `{current, target, action, updated, asset?}`.
- `status`
- `ls [path]` (depth 1), `tree [path]` (`--depth`, default 3)
- `cat <path>` `--full` `--lines <START-END>` `--hashes` (emit `line:hash: text` per line — the read step before `edit`; `--lines` is 1-based inclusive and conflicts with `--full`)
- `grep <pattern>` `-i` `-l` `-C <n>` `--limit <n>`
- `glob <pattern>`
- `write <path>` (content from stdin; `-m <text>` for one-liners)
- `edit <path> --line <n> --hash <h> --new=<s>` (single line edit; use the `=` form when `<s>` starts with `-`)
- `edit-batch <path>` (reads JSON array of `{line, expected_hash, new_text}` from stdin; all-or-nothing through the same hash-guarded core edit)
- `mv <from> <to>` `--force`
- `rm <path> --force`
- `links <path>`
- `tags`
- `doctor` `--stale-days <n>` `--checks <a,b,c>` `--profile <llm-wiki|strict>`
- `serve` `--config <path>` (discovery: `--config` → `$WIKID_CONFIG` → `./wikid.toml` → `~/.config/wikid/config.toml`; write target for bootstrap/mutation: explicit/env path even if absent, else existing `./wikid.toml`, else global config)

**Embedded skills.** Source files live in the top-level repo `skills/` directory and are compiled into the `wikid` binary with an `include_str!` static registry in `crates/wikid/src/skills.rs`:

```rust
pub struct Skill {
    pub name: &'static str,
    pub body: &'static str,
    pub references: &'static [(&'static str, &'static str)],
}
```

Tier-1 descriptions are parsed from each `SKILL.md` YAML frontmatter at runtime with `serde_yaml`; descriptions are not duplicated in Rust. Root clap help uses `before_help` to show the AI-agent start-here block, and `skills` has the first display order in the command list. Guard tests require valid frontmatter for every registered skill and require every clap subcommand name to appear in the core skill body plus references. A living-document integration test executes the `skills/core/SKILL.md` examples for hashes, stale edits, truncation, tags, and links so documented excerpt formats stay enforced.

## 7. HTTP API (`wikid-server`)

- `GET /health` — unauthenticated `{"status":"ok","version":"<CARGO_PKG_VERSION>"}` so clients can surface client/server version mismatches without blocking compatible operations.
- Everything else requires `Authorization: Bearer <token>`; unknown token → 401 `{"error":{"code":"unauthorized",…}}`.
- Routes (all under a named wiki; unknown wiki → 404 `unknown_wiki` listing available names). In remote mode, `status.root` is the server-side filesystem path; it is not a path the client machine can read directly. Human CLI rendering labels it `root (server): …`, while `--json` preserves the shared `VaultStatus` struct unchanged:
  - `GET  /v1/wikis` → `{wikis:[{name, pages}]}`
  - `GET  /v1/wikis/{wiki}/status`
  - `GET  /v1/wikis/{wiki}/ls?path=&depth=`
  - `GET  /v1/wikis/{wiki}/cat?path=&full=&lines=&hashes=` (`lines=START-END`; `hashes=true` → `HashlinesResult` instead of `Document`)
  - `GET  /v1/wikis/{wiki}/grep?pattern=&ignore_case=&files_only=&context=&limit=`
  - `GET  /v1/wikis/{wiki}/glob?pattern=`
  - `GET  /v1/wikis/{wiki}/links?path=`
  - `GET  /v1/wikis/{wiki}/tags`
  - `GET  /v1/wikis/{wiki}/doctor?stale_days=&checks=&profile=`
  - `PUT  /v1/wikis/{wiki}/pages` body `{path, content}`
  - `POST /v1/wikis/{wiki}/edit` body `{path, edits: [{line, expected_hash, new_text}]}`
  - `POST /v1/wikis/{wiki}/mv` body `{from, to, force}`
  - `DELETE /v1/wikis/{wiki}/pages?path=&force=true` (`force` missing → 400 `force_required`, the same code and shape as the CLI's `rm` refusal with wording adapted to the wire: `force=true` instead of `--force`)
- Success bodies are the core structs serialized directly. `WikidError` → status mapping: `NotFound`/`unknown_wiki` 404, `InvalidPath`/`BadPattern`/`BadEdit`/usage 400, `AlreadyExists`/`StaleEdit` 409, `NotUtf8` 415, `Io` 500. Body always `{"error":{"code","message","hint"}}`.
- Config (TOML, see `wikid-server::config`): `bind` (default `127.0.0.1:7448`), optional `default_wiki`, `[wikis] name = "/path"`, `[tokens] "actual_secret" = "actor-name"`. Actor names are logged (`tracing`) but not otherwise used in MVP — attribution is deferred by SPEC. Config rewrites preserve keys/values (comments may be lost), use temp+rename where feasible, and set Unix mode `0600`.
- `serve` with no config found bootstraps the write target, registers cwd under its directory name (collision suffix `name-2`, `name-3`, …), generates one `admin` token from 32 bytes of `/dev/urandom` hex-encoded with `wkd_`, prints startup info without the token value, flushes stdout, then serves immediately. `serve --json` prints exactly one startup object to stdout before serving; logs must not corrupt stdout.
- Startup validates every wiki dir exists (fail fast) and warns loudly when `tokens` is empty and bind is non-loopback (auth-less non-local serving is refused).

## 8. Testing strategy

- **Core**: co-located unit tests. A `test-fixtures` helper (`#[cfg(test)]` + `pub` test-util module) builds a temp vault: nested pages with wikilinks (incl. alias, heading, ambiguous, broken), frontmatter'd + frontmatter-less pages, `.obsidian/` dir (must be invisible), a binary attachment, an oversized page, a stale page (set mtime via `filetime`-free trick: `File::set_times`). Must cover: path escapes (`../x`, absolute, symlink out), atomic write behavior, edit hash matching (fresh/stale/case-insensitive), all-or-nothing batch refusal, structural batch validation, EOL/trailing-newline preservation, grep flags + binary skip, link resolution order, every doctor check firing and not firing.
- **CLI**: integration tests in `crates/wikid/tests/` with `assert_cmd` + `predicates` against temp vaults. Cover the AXI checklist §6 item by item, plus `--json` validity (parse with `serde_json`) and exit codes.
- **Server**: in-crate tests via `tower::ServiceExt::oneshot` (no port binding): auth 401, unknown wiki 404, happy paths for every route, path-escape 400, delete without force 400.
- **End-to-end** (in `crates/wikid/tests/`): spawn `wikid serve` on an ephemeral port with a temp config, run CLI in remote mode against it, assert parity with local-mode output on the same vault.

## 9. Dependencies (locked)

Workspace-level, already declared in root `Cargo.toml`. Do not add others without updating this section: `anyhow`, `thiserror`, `serde`, `serde_json`, `serde_yaml`, `toml`, `clap` (derive), `tokio`, `axum`, `regex`, `globset`, `ignore`, `sha2` (line hashes for the edit protocol), `tempfile`, `humantime` (RFC3339 formatting of `SystemTime`), `ureq` (json feature — sync client keeps the CLI runtime-free), `tracing`, `tracing-subscriber`; dev: `assert_cmd`, `predicates`, `tower` (util), `http-body-util`.

## 10. Deferred (do not build now)

MCP server (next milestone, `rmcp`, thin adapter over the same core), search index (tantivy behind the same `grep` shape), activity log/attribution, history/undo, read-only tokens, TLS (document reverse-proxy/tailscale instead), `--fail-on` for doctor, TOON output format (measure token cost first).
