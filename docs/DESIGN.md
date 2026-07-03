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
| `cat(path, limit: Option<ReadLimit>)` | Returns `Document { path, content, truncated, total_lines, total_bytes, modified }`. Default limit 400 lines or 32 KiB (whichever first); `None` = full. |
| `grep(pattern, opts)` | Regex search (`regex` crate) over pages + UTF-8 text files. Options: `ignore_case`, `files_only`, `context` (lines), `limit` (default 50 matches). Result: `matches: [{path, line, text}]` (+ `context_before/after` when requested), `total_matches`, `matched_files` (files with ≥1 match), `total_files` (files searched), `truncated`. Files whose path stem matches the pattern are ranked first. |
| `glob(pattern)` | `globset` match over relative paths, e.g. `**/*.md`. Sorted by path. Returns entries + `total`. |
| `write(path, content)` | Create or overwrite. Creates parent dirs. **Atomic**: `tempfile::NamedTempFile::new_in(parent)` + `persist`. Returns `{path, created: bool, bytes}`. |
| `edit(path, old, new, all: bool)` | Literal (non-regex) string replacement. `all=false`: `old` must match exactly once — 0 matches → `NoMatch` (error includes a best-effort nearest-line hint), >1 → `Ambiguous { count }`. `all=true`: replace every occurrence, return count. Write is atomic as above. |
| `mv(from, to, force)` | Rename file (not dirs). Creates parent dirs at destination. Destination exists and `!force` → `AlreadyExists`. |
| `rm(path)` | Delete file (not dirs). The `--force` gate is CLI/HTTP-level, not core. |
| `links(path)` | `LinkReport { outgoing: [{raw, target, resolved: Option<path>, kind: wikilink/markdown}], backlinks: [path] }`. Backlinks = scan all pages for links resolving to `path`. |
| `doctor(opts)` | See §5. |
| `status()` | `VaultStatus { root, total_pages, total_files, total_bytes, recent: [{path, modified}] (5 most recent pages), doctor_summary: {high, medium, low} }`. |

**Error model** (`thiserror` enum `WikidError`): `NotFound`, `InvalidPath`, `AlreadyExists`, `NoMatch`, `Ambiguous`, `NotUtf8`, `BadPattern`, `Io`. Each maps to a stable string `code()` and an optional `hint()` (used verbatim by CLI and HTTP error bodies).

## 4. Core: link model (Obsidian-compatible)

- Extract `[[Target]]`, `[[Target|alias]]`, `[[Target#Heading]]` (heading part ignored for resolution) and markdown links `[text](relative/path.md)` (skip `http(s)://`, `mailto:`, anchors).
- Resolution, in order: (1) exact relative path from root (with/without `.md`); (2) unique file-stem match anywhere in the vault (case-insensitive); (3) unique path-suffix match (`folder/Note`). Multiple stem candidates → unresolved + flagged `ambiguous` (doctor reports it). No match → broken link.
- Frontmatter: leading `---\n…\n---` block parsed with `serde_yaml` into a string-keyed map. Absence is normal. Malformed YAML → treated as no frontmatter, doctor flags it.
- Page title: frontmatter `title` → first `# heading` → file stem.

## 5. Core: doctor checks

All structural, no LLM. Each issue: `{check, severity (low/medium/high), path, detail, suggested_action}`. Report includes per-check counts and a one-line summary. Exit code 0 even with findings (it's a report; `--fail-on <severity>` can gate CI later — not in MVP).

| Check | Trigger | Severity |
|---|---|---|
| `broken_links` | link resolves to nothing | high |
| `ambiguous_links` | stem matches >1 file | medium |
| `orphan_pages` | page with no inbound links, excluding root-level `index.md`/`README.md` | low |
| `missing_frontmatter` | only when ≥50% of pages have frontmatter (the vault "uses" it) — pages without it | low |
| `malformed_frontmatter` | `---` block present but YAML parse fails | medium |
| `stale_pages` | mtime older than `stale_days` (default 90) | low |
| `oversized_pages` | > 64 KiB or > 1500 lines | medium |
| `duplicate_stems` | same case-insensitive stem at multiple paths (breaks wikilink resolution) | medium |

## 6. CLI

**Targeting.** Local mode: `--dir <path>` or `$WIKID_DIR`. Remote mode: `--server <url>` + `--token <t>` + `--wiki <name>` or `$WIKID_SERVER`/`$WIKID_TOKEN`/`$WIKID_WIKI`. Both given → usage error. Neither → structured error explaining both options. Remote mode maps commands 1:1 onto the HTTP API and renders identically (deserializes the shared core structs). `--wiki` is required in remote mode (missing → structured `no_wiki` error); `--token` is optional so auth-less loopback daemons work. HTTP error bodies re-render verbatim as the same `error[<code>]` with exit 1; connection/decode failures use the CLI-level `transport` code, unrecognized error bodies `http`. `serve` with no discoverable config → `no_config`; an unloadable config → `config`.

**Output.** Human-readable compact text by default; `--json` on every command emits the core result struct as JSON (one object, stdout). Errors (both modes): stdout, exit 1, format `error[<code>]: <message>` + optional `hint: …` line; `--json` errors: `{"error":{"code","message","hint"}}`. Usage errors: clap default (exit 2).

**AXI conformance checklist** (each item is a test):
1. `wikid` with no args = `status` — live data, never help text.
2. List items show ≤4 fields; totals always present.
3. `cat` truncates by default with `… truncated (N lines / M bytes total) — use --full`.
4. Zero results are explicit: `no matches for "…" in N files`.
5. Structured errors on stdout; exit codes 0/1/2; **grep exits 1 on zero matches** (coreutils-faithful).
6. Never prompts interactively; `rm` requires `--force` (refusal is a structured error, not a question).
7. Human output ends with 1–2 `hint:` lines suggesting next commands with `<placeholders>` (e.g. after `grep`: `hint: wikid cat <path> — read a match`). No hints in `--json`.

**Commands and flags** (coreutils-faithful; unknown flags fail with exit 2 and clap's suggestion):

- `status`
- `ls [path]` (depth 1), `tree [path]` (`--depth`, default 3)
- `cat <path>` `--full`
- `grep <pattern>` `-i` `-l` `-C <n>` `--limit <n>`
- `glob <pattern>`
- `write <path>` (content from stdin; `-m <text>` for one-liners)
- `edit <path> --old <s> --new <s>` `--all`
- `mv <from> <to>` `--force`
- `rm <path> --force`
- `links <path>`
- `doctor` `--stale-days <n>` `--checks <a,b,c>`
- `serve` `--config <path>` (default: `$WIKID_CONFIG` → `./wikid.toml` → `~/.config/wikid/config.toml`)

## 7. HTTP API (`wikid-server`)

- `GET /health` — unauthenticated `{"status":"ok"}`.
- Everything else requires `Authorization: Bearer <token>`; unknown token → 401 `{"error":{"code":"unauthorized",…}}`.
- Routes (all under a named wiki; unknown wiki → 404 `unknown_wiki` listing available names):
  - `GET  /v1/wikis` → `{wikis:[{name, pages}]}`
  - `GET  /v1/wikis/{wiki}/status`
  - `GET  /v1/wikis/{wiki}/ls?path=&depth=`
  - `GET  /v1/wikis/{wiki}/cat?path=&full=`
  - `GET  /v1/wikis/{wiki}/grep?pattern=&ignore_case=&files_only=&context=&limit=`
  - `GET  /v1/wikis/{wiki}/glob?pattern=`
  - `GET  /v1/wikis/{wiki}/links?path=`
  - `GET  /v1/wikis/{wiki}/doctor?stale_days=&checks=`
  - `PUT  /v1/wikis/{wiki}/pages` body `{path, content}`
  - `POST /v1/wikis/{wiki}/edit` body `{path, old, new, all}`
  - `POST /v1/wikis/{wiki}/mv` body `{from, to, force}`
  - `DELETE /v1/wikis/{wiki}/pages?path=&force=true` (`force` missing → 400 `force_required`, the same code and shape as the CLI's `rm` refusal with wording adapted to the wire: `force=true` instead of `--force`)
- Success bodies are the core structs serialized directly. `WikidError` → status mapping: `NotFound`/`unknown_wiki` 404, `InvalidPath`/`BadPattern`/usage 400, `AlreadyExists`/`Ambiguous` 409, `NoMatch` 404, `NotUtf8` 415, `Io` 500. Body always `{"error":{"code","message","hint"}}`.
- Config (TOML, see `wikid-server::config`): `bind` (default `127.0.0.1:7448`), `[wikis] name = "/path"`, `[tokens] "token" = "actor-name"`. Actor names are logged (`tracing`) but not otherwise used in MVP — attribution is deferred by SPEC.
- Startup validates every wiki dir exists (fail fast) and warns loudly when `tokens` is empty and bind is non-loopback (auth-less non-local serving is refused).

## 8. Testing strategy

- **Core**: co-located unit tests. A `test-fixtures` helper (`#[cfg(test)]` + `pub` test-util module) builds a temp vault: nested pages with wikilinks (incl. alias, heading, ambiguous, broken), frontmatter'd + frontmatter-less pages, `.obsidian/` dir (must be invisible), a binary attachment, an oversized page, a stale page (set mtime via `filetime`-free trick: `File::set_times`). Must cover: path escapes (`../x`, absolute, symlink out), atomic write behavior, edit 0/1/n matches, grep flags + binary skip, link resolution order, every doctor check firing and not firing.
- **CLI**: integration tests in `crates/wikid/tests/` with `assert_cmd` + `predicates` against temp vaults. Cover the AXI checklist §6 item by item, plus `--json` validity (parse with `serde_json`) and exit codes.
- **Server**: in-crate tests via `tower::ServiceExt::oneshot` (no port binding): auth 401, unknown wiki 404, happy paths for every route, path-escape 400, delete without force 400.
- **End-to-end** (in `crates/wikid/tests/`): spawn `wikid serve` on an ephemeral port with a temp config, run CLI in remote mode against it, assert parity with local-mode output on the same vault.

## 9. Dependencies (locked)

Workspace-level, already declared in root `Cargo.toml`. Do not add others without updating this section: `anyhow`, `thiserror`, `serde`, `serde_json`, `serde_yaml`, `toml`, `clap` (derive), `tokio`, `axum`, `regex`, `globset`, `ignore`, `tempfile`, `humantime` (RFC3339 formatting of `SystemTime`), `ureq` (json feature — sync client keeps the CLI runtime-free), `tracing`, `tracing-subscriber`; dev: `assert_cmd`, `predicates`, `tower` (util), `http-body-util`.

## 10. Deferred (do not build now)

MCP server (next milestone, `rmcp`, thin adapter over the same core), search index (tantivy behind the same `grep` shape), activity log/attribution, history/undo, read-only tokens, TLS (document reverse-proxy/tailscale instead), `--fail-on` for doctor, TOON output format (measure token cost first).
