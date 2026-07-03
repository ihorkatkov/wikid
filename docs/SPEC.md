# SPEC: LLM Wiki Runtime — v0.2 (MVP)

**Status:** Draft v0.2 — supersedes v0.1 for MVP scope
**Derived from:** v0.1 + product discussion 2026-07-02
**One-sentence definition:** A single self-hostable binary that exposes one or more plain-Markdown wiki directories to remote agents over CLI and MCP, with a filesystem-feeling, agent-optimized (AXI) surface and structural health checks — no git, no database, no review workflow, no LLM inside.

---

## 1. What changed from v0.1 and why

| v0.1 | v0.2 decision | Rationale |
|---|---|---|
| Proposal / review / publication workflow (§9.5–9.6, §10.4–10.5) | **Cut.** Agents write directly. | Simplicity; trust moves from workflow gates to the user's substrate (their own git/backups). |
| Sessions (§9.4, §10.3) | **Cut.** | An agent can write a notes page directly; no first-class object needed. |
| Sources (§9.3, §10.2) | **Cut.** | Sources are just files in a directory by convention. |
| Summarization/curation (§9.8), semantic search, `wiki.related`, trust levels | **Cut / deferred.** | Dumb runtime. Thinking is layered on later by agents, not embedded. |
| Attribution & activity (§9.9), roles (§9.10) | **Cut from MVP** (auth tokens are named, but actions are not logged). | Deferred to cloud tier. |
| Git-native | **Storage-agnostic.** The runtime never touches git or any DB. Plain files, atomic writes, last-write-wins. | Karpathy plain-files principle. Versioning/undo/backup is the *substrate's* concern — users may git-init or Dropbox the directory themselves; the runtime doesn't know or care. |
| Hosted ambiguity | **Self-hostable daemon first, clear path to cloud.** | Distribution and "runs on whatever VM" is the current core value; managed cloud is the later business model. |

### Consciously accepted risks
- **Last-write-wins**: concurrent agents can clobber each other. Accepted for MVP.
- **No undo in the runtime**: a destroyed page is gone unless the user's substrate (git, backups) has it. Accepted; documented loudly.
- **No attribution**: anonymous writes. Accepted; named tokens keep the door open (identity exists at the auth layer, logging comes later).
- These three are the first things the cloud/team tier must add back. The plain-files core must not be designed in a way that blocks adding an activity log or history layer later.

---

## 2. Product shape

- **One Rust binary** (`wikid` — name TBD) with two roles:
  - `wikid serve` — daemon on the machine that owns the wiki directories.
  - `wikid <command>` — CLI client; talks to a daemon over HTTP, or operates on a local directory directly (same commands, `--dir` / config-selected).
- **MCP**: the daemon also speaks MCP (streamable HTTP) exposing the same operations as tools, so agent harnesses connect natively. CLI and MCP are thin views over one operation core.
- **Multi-wiki**: one daemon serves multiple named wikis. Config maps `name → directory`. Every remote call is scoped by wiki name; "project" from v0.1 is dropped — projects are just paths inside a wiki.
- **Auth**: named bearer tokens in daemon config (`token → actor name`). No accounts, no OAuth. Network exposure is the operator's choice (localhost, tailscale, public + TLS).
- **Distribution**: single static binary per platform, `curl | sh` installer, cargo install. Getting an agent VM connected must be one command.

## 3. Storage model

- A wiki is **any directory of Markdown files**. Zero required setup — pointing at an existing Obsidian vault must just work.
- **Obsidian compatibility is required**: YAML frontmatter, `[[wikilinks]]` (including aliases `[[page|label]]`), ignore `.obsidian/`. The runtime parses these when present; every feature degrades gracefully when absent (convention over enforcement).
- Writes are **atomic** (write temp + rename). No locks, no versions, no journal.
- The runtime holds **no state that isn't derivable from the files** (indexes are rebuildable caches).

## 4. Operation surface

Design principle: **as close to a filesystem as possible.** The surface mirrors the primitives coding agents already use natively (ls / cat / grep / glob / write / surgical edit), because remote agents lose their native file tools and need equivalents. No section-level addressing as a special API — surgical edits are hash-guarded line replacements: read line hashes with `cat --hashes`, then replace lines by number + hash. A stale hash refuses the edit, so concurrent writers can't silently clobber each other. Optimal use is the harness author's responsibility.

Commands (CLI verbs ≙ MCP tools ≙ HTTP endpoints):

| Command | Behavior |
|---|---|
| `status` | Content-first, no-arg default: wikis served, page counts, recently modified, health summary. (AXI #8) |
| `ls [path]` / `tree` | List pages/directories; includes total counts. (AXI #4) |
| `cat <path>` | Read a page. Large files truncated with size hint + `--full`. (AXI #3) |
| `grep <pattern>` | Search content; ranked-lite (title/path matches boosted), match context, total hit count, explicit zero-result message. (AXI #5) |
| `glob <pattern>` | Find pages by path pattern. |
| `write <path>` | Create/overwrite a page (content from stdin or arg). Atomic. |
| `edit <path>` | Hash-guarded line replacement (line number + expected hash → new text; stale hash refuses the whole edit), the safe surgical-write primitive. |
| `mv`, `rm` | Rename/delete. `rm` is the one destructive verb — requires `--force` flag (never an interactive prompt, AXI #6). |
| `links <path>` | Outgoing links + backlinks for a page (from the wikilink graph). |
| `doctor` | Health checks — see §5. |

AXI conformance across all commands: token-efficient compact output (JSON/TOON on `--json`), 3–4 fields per list item by default, structured errors on stdout with exit codes 0/1/2, never prompt interactively, mutations idempotent where possible, contextual next-step hints appended after output, `--help` everywhere.

## 5. Health checks (`doctor`)

Structural only — no LLM, no semantics:
- broken wikilinks / markdown links
- orphan pages (no inbound links, not the index)
- missing frontmatter / missing expected fields (only if the wiki uses frontmatter)
- stale pages (mtime older than threshold)
- oversized pages (should-split hint)
- duplicate titles / conflicting filenames

Output: issue list with severity, path, description, suggested action (as in v0.1 §10.6). Contradiction detection, missing-source detection: deferred to the thinking layer.

## 6. What "clear path to cloud" means

The MVP must not preclude:
- multi-tenant hosting (wikis are already named + token-scoped)
- an activity log / attribution layer (append-only, file-based, per-wiki)
- a history/undo layer
- a search index upgrade (tantivy BM25, embeddings) behind the same `grep`-shaped interface
- a curator/thinking agent using the same public surface

None of these are built now; the operation core is designed so they bolt on without breaking the surface.

## 7. MVP acceptance criteria (revised)

1. **AC1 (kept):** An agent on a fresh VM reaches useful wiki context with one install command + one `grep`/`cat` — no clone, no repo credentials.
2. **AC2/AC15 (kept, strengthened):** The wiki is plain files at all times; the runtime holds nothing that isn't derivable from them.
3. **AC3 (reduced):** `grep` returns ranked, focused matches with counts — not semantic search.
4. **AC9 (kept, reduced):** `doctor` reports the structural issues in §5.
5. **AC10 (kept):** All operations available identically via CLI, HTTP, MCP; no hidden behavior.
6. **AC11 (kept):** An existing Obsidian vault works with zero migration; humans keep editing in Obsidian/editors while agents work remotely.
7. **New — plug-and-play:** `wikid serve` on a directory + one token = a remote agent is productive in under a minute.
8. **Dropped from MVP:** AC4–AC8, AC12–AC14 (sessions, proposals, review, traceability, conflict surfacing, source separation, trust levels, attribution).

## 8. Implementation notes

- **Language:** Rust. CLI via `clap`; server via `axum`; MCP via `rmcp`; search via the `grep`/`ignore` crates (ripgrep internals); frontmatter via `serde_yaml`; TOML config.
- **Crate layout:** `core` (vault model, link graph, file ops, health checks) / `server` (HTTP + MCP + auth) / `cli` (client + local mode) — one workspace, one released binary.
- **Config:** single TOML: bind address, `[wikis]` name→path map, `[tokens]` token→actor map.

## 9. Open items (implementation-time, not blockers)

1. Product name / binary name.
2. Exact compact output format (TOON vs terse JSON vs aligned text) — benchmark on token count.
3. Whether `grep` gets a tiny persistent index in v1 or scans per call (per-call is fine up to thousands of pages; measure).
4. TLS story for public exposure (recommend: document tailscale/caddy, don't build TLS in).
5. Read-only tokens (cheap to add; probably worth it in v1).
