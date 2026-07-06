---
date: 2026-07-06T00:00:00Z
researcher: mnemosyne
git_commit: 493e6adeb853adf12f6b6acdffb2bb636e70cbb2
branch: main
repository: wikid
topic: "What does wikid need for Open Knowledge Format (OKF) compatibility?"
scope: "wikid Rust workspace (crates: wikid, wikid-core, wikid-server) vs OKF v0.1 draft spec"
query_type: map
tags: [research, okf, frontmatter, links, doctor, compatibility]
status: complete
confidence: high
sources_scanned:
  files: 8
  thoughts_docs: 0
  beads_tasks: 1
---

# Research: What does wikid need for Open Knowledge Format (OKF) compatibility?

**Date**: 2026-07-06
**Commit**: 493e6adeb853adf12f6b6acdffb2bb636e70cbb2
**Branch**: main
**Confidence**: high — the raw OKF `SPEC.md` was fetched and read end-to-end (451 lines), and every wikid claim below is cited to a specific file:line at the researched commit.

## Query
> What does wikid need for Open Knowledge Format (OKF) compatibility? (bead wikid-kre)

## Summary
The OKF v0.1 draft (`okf/SPEC.md` in GoogleCloudPlatform/knowledge-catalog) defines a knowledge bundle as a directory of Markdown files with YAML frontmatter, where **`type` is the only REQUIRED frontmatter field** and consumers must tolerate missing optional fields, unknown keys, unknown types, broken links, and a missing `index.md`. wikid already satisfies most of the *consumer* obligations: it parses YAML frontmatter, resolves both OKF-sanctioned link forms (bundle-relative absolute `/…` and directory-relative `./`/`../`) exactly, tolerates broken links as non-fatal, parses `tags`, derives `title`, excludes `index.md` from orphan checks, and its `init` scaffold already emits `index.md`/`log.md`. The gaps are narrow: no `type`-presence health check, no surfacing of frontmatter (`type`/`title`/`tags`) in `ls`/`status`/`grep`, no OKF bundle detection via `okf_version`, and a small `log.md` heading-format divergence. **The bead `wikid-kre` description is factually wrong** where it lists required keys as "type, resource, tags, timestamp" — the spec requires only `type`; the other three are optional/recommended.

## OKF Spec Summary (Section 1)

**Source**: `okf/SPEC.md`, Open Knowledge Format v0.1 (draft), GoogleCloudPlatform/knowledge-catalog, fetched raw 2026-07-06 (local copy read at `/tmp/okf_spec.md`, 451 lines). All section numbers below refer to that file.

### What a bundle is
- **§2 Concept / Concept ID**: A bundle is a directory tree of `.md` files. A concept document's ID is its file path relative to the bundle root with the `.md` extension removed (e.g. `tables/orders.md` → `tables/orders`).
- **§3.1 Reserved filenames**: `index.md` and `log.md` are **reserved at any directory level** and MUST NOT be treated as concept documents.

### Frontmatter (§4) — the core of the spec
Every non-reserved `.md` file MUST begin with a YAML frontmatter block delimited by `---`.

| Key | Requirement | Type | Semantics |
|-----|-------------|------|-----------|
| `type` | **REQUIRED** | non-empty string | The kind of concept (e.g. `table`, `dataset`, `metric`). The only mandatory field. |
| `title` | Recommended (optional) | string | Human-readable name. |
| `description` | Recommended (optional) | string | Short summary. |
| `resource` | Recommended (optional) | string (URI) | Pointer to the external resource the concept describes. |
| `tags` | Recommended (optional) | YAML list of strings | Free-form labels. |
| `timestamp` | Recommended (optional) | string (ISO 8601) | Time of last *meaningful* change to the concept. |

- **§4.1**: `type` is the only required field. All others are recommended but optional.
- **§4.2 Extensibility**: Producers MAY add arbitrary additional keys. Consumers **SHOULD preserve unknown keys** and **MUST NOT reject** a document for unrecognized fields or unrecognized `type` values.

### Links (§5)
- **§5.1 Absolute (bundle-relative)**: Links beginning with `/` are resolved from the bundle root (e.g. `/tables/orders.md`). **Recommended form.**
- **§5.2 Relative**: Links beginning with `./` or `../` resolve relative to the linking document's directory. Bare relative paths resolve relative to the current directory.
- Links are untyped, directed edges. **§5.3**: Consumers **MUST tolerate broken links** (a link whose target does not exist is not an error).

### index.md (§6)
- A navigation aid, NOT a concept document. Contains **no frontmatter** (see §11 exception for the bundle root).
- Body is sections of Markdown bullets in the form `* [Title](url) - description`.
- Producers MAY auto-generate it; consumers MAY synthesize one if absent (its absence is not an error).

### log.md (§7)
- Chronological change log. Body is `## YYYY-MM-DD` date headings, **newest-first**, with optional bold prefixes inside entries.

### Conformance (§9)
- **Producer**: every non-reserved `.md` has a parseable YAML frontmatter block; every such block has a non-empty `type`; reserved files follow §6/§7.
- **Consumer**: MUST NOT reject a bundle for: missing optional fields, unknown `type` values, unknown frontmatter keys, broken links, or a missing `index.md`.

### Versioning / detection (§11)
- The **bundle-root `index.md`** MAY carry a single frontmatter key `okf_version: "0.1"`. This is the **only** place frontmatter is permitted inside an `index.md`, and the only spec-sanctioned marker that a directory is an OKF bundle. OKF is otherwise deliberately manifest-free and unmarked.

### Reference bundles (confirmed via GitHub API)
`okf/bundles/{crypto_bitcoin, ga4, stackoverflow}` exist in the repo, each organized into subdirectories (`tables/`, `datasets/`, `references/`, `metrics/`, `joins/`) with a per-directory `index.md`.

### Bead validation
The bead `wikid-kre` states required frontmatter keys are "type, resource, tags, timestamp." **This is incorrect against §4.1**: only `type` is required. `resource`, `tags`, and `timestamp` are recommended/optional. A consumer that *rejected* documents lacking those three would violate §9's consumer conformance rules.

## What wikid Already Does (Section 2)

Mapping to OKF requirements, with file:line at commit `493e6ad`.

### Frontmatter parsing — mostly conformant
| OKF requirement | wikid behavior | Location |
|-----------------|----------------|----------|
| Parse `---`-delimited YAML frontmatter | `Frontmatter` enum {Absent, Malformed, Present(BTreeMap)}; leading-block detection | `crates/wikid-core/src/frontmatter.rs:12-121` |
| Preserve unknown keys (§4.2) | Present variant stores the **entire** map as `BTreeMap<String, serde_yaml::Value>`; no key is dropped or rejected | `crates/wikid-core/src/frontmatter.rs:19-20`, `frontmatter.rs:117` |
| `title` recommended field | `page_title` precedence: frontmatter `title` → first `# ` heading → file stem | `crates/wikid-core/src/frontmatter.rs:69-84` |
| `tags` recommended field | `frontmatter_tags` reads `tags` as string or sequence, strips leading `#` | `crates/wikid-core/src/tags.rs:92`; merged with inline in `page_tags` at `tags.rs:111` |
| Empty frontmatter tolerated | Empty block parses to `Present(empty map)` (Obsidian writes these) | `crates/wikid-core/src/frontmatter.rs:113-116` |

Not yet handled as first-class: `type`, `resource`, `timestamp`, `description` are stored (they land in the `Present` map) but are never read, surfaced, or validated anywhere.

### Links — OKF's two sanctioned forms already resolve exactly
| OKF link form | wikid behavior | Location |
|---------------|----------------|----------|
| Absolute bundle-relative `/tables/x.md` (§5.1) | `strip_prefix('/')` → `resolve_normalized_exact(root_relative)` — **exact** resolution from vault root | `crates/wikid-core/src/links.rs:375` |
| Relative `./`, `../` (§5.2) | Handled by directory-relative exact resolution | `crates/wikid-core/src/links.rs:378-385` |
| Broken links tolerated (§5.3) | Unresolved links get `resolved: None`; they are reported by doctor but are not a parse/load error | `crates/wikid-core/src/links.rs` (LinkReport with `resolved: Option<…>`) |

Note the one divergence (see Risks): **bare** targets with no `/` or `./` prefix fall through to a *fuzzy stem* resolver (`resolve()`, `links.rs:330/341`) that does global basename matching — more permissive than OKF §5.2's directory-relative intent. This is a tolerant superset, not a rejection.

### Reserved files — already scaffolded, one format nit
| OKF requirement | wikid behavior | Location |
|-----------------|----------------|----------|
| `index.md` reserved, no frontmatter (§6) | `init` scaffolds `index.md` starting with `# Index` and **no** frontmatter block → conformant | `crates/wikid/src/main.rs:649`, template `main.rs:654-677` |
| `log.md` reserved, `## YYYY-MM-DD` headings (§7) | `init` scaffolds `log.md`; but template uses `## [YYYY-MM-DD] ingest | <title>` (bracketed date + typed prefix) | `crates/wikid/src/main.rs:650`, template `main.rs:679-688` |
| `index.md` excluded from orphan detection | `orphan_pages` skips root `index.md`/`README.md` | `crates/wikid-core/src/doctor.rs:442` |

### Doctor health checks — no OKF-specific check yet
- `Check` enum has 11 variants including `MissingFrontmatter` (Low) and `MalformedFrontmatter` (Medium) — `crates/wikid-core/src/doctor.rs:96`.
- `missing_frontmatter` only checks *block presence* (adoption-gated); there is **no** check that a present block contains a non-empty `type` — `crates/wikid-core/src/doctor.rs:700`.
- `DoctorProfile` already exists with `LlmWiki` and `Strict` variants — the extension point for an OKF-flavored profile.

### Structural fit
wikid's `init` already produces an LLM-wiki layout (`raw/`, `concepts/`, `entities/`, `questions/`, `syntheses/` + `index.md`, `log.md`, `AGENTS.md`) — `crates/wikid/src/main.rs:537,649-651`. The shape mirrors OKF's directory-of-concepts + `index.md` + `log.md` convention closely; the difference is OKF keys concepts by a required `type` field rather than by directory role.

## Existing Overlapping Code (Phase B.5)
Work items below overlap with existing code — reuse points, not new subsystems:
- **Frontmatter key access**: `frontmatter.rs` already exposes the full parsed map; a `type`/`resource`/`timestamp` accessor is a small addition next to `aliases()` (`frontmatter.rs:45`) and `page_title()` (`frontmatter.rs:69`), not a new parser.
- **Health check**: a `type`-required check slots into the existing `Check` enum and `run` dispatch in `doctor.rs`, mirroring `missing_frontmatter` (`doctor.rs:700`).
- **Profiles**: `DoctorProfile` (`doctor.rs`) already gates check sets; an `Okf` profile reuses that mechanism.
- **Wire structs**: `Entry` (`ops.rs:35`) and `GrepOptions` (`ops.rs:141`) are the shared CLI/HTTP/MCP wire format (per `lib.rs:5-6`); adding frontmatter fields there propagates to all three transports at once.
- **Tags**: `tags.rs:92` already parses `tags` — no new work for that key.

## Gaps Identified (Section 3 — ordered)

Ordered by dependency and impact (facts about what is absent; search terms noted).

1. **No `type`-presence health check.** Nothing verifies that a non-reserved `.md` has a non-empty `type` (OKF §9 producer conformance). Would be a new `Check` variant beside `MissingFrontmatter`. Search: "type", "required", "frontmatter" in `doctor.rs` → only block-presence check at `doctor.rs:700`.
   - Constraint: the exemption set is **not** wikid's existing one. OKF reserves `index.md` and `log.md` **at every directory level** (§3.1), whereas `doctor.rs:442` exempts only the **root** `index.md` and also treats `README.md` specially — and `README.md` is a wikid/Obsidian-ism, **not** OKF-reserved. A `type` check must exempt `index.md`/`log.md` hierarchy-wide and must NOT exempt `README.md`.
2. **No frontmatter surfacing in `ls`/`status`/`grep`.** `Entry` (`ops.rs:35`) carries only `{path, kind, size, modified}` — no `type`/`title`/`tags`. `GrepOptions` (`ops.rs:141`) has no frontmatter filter. `VaultStatus` (`status.rs:28`) has no type/tag aggregation. Search: "title", "type", "tags" in `ops.rs`/`status.rs` → absent. (One core-struct change; inherited by CLI/HTTP/MCP per `lib.rs:5-6`.)
3. **No OKF bundle detection.** Nothing reads `okf_version` from the root `index.md` (§11). Search: "okf", "okf_version", "version" across `crates/` → not found. Note: `okf_version` is the *only* spec-sanctioned marker; any file-count heuristic (e.g. "≥N files carry `type`") is inference, not spec-mandated.
4. **`log.md` heading format divergence.** Scaffold emits `## [YYYY-MM-DD] ingest | <title>` (`main.rs:683-687`); OKF §7 specifies plain `## YYYY-MM-DD` newest-first with optional bold prefixes. wikid as a *consumer* does not parse `log.md` today, so this is a *producer*-side scaffold divergence only. Search: "log.md", "YYYY-MM-DD" → `main.rs:679`.
5. **No `type`/`resource`/`timestamp` accessors.** These keys are stored in the `Present` map but never read (`frontmatter.rs` exposes only `aliases()` and `page_title()`). Prerequisite for gaps 1–2.
6. **`timestamp` vs filesystem mtime.** `status`/`recent`/`stale_pages` all use filesystem `modified()` (`status.rs:28` `RecentPage.modified`), never the OKF `timestamp` frontmatter field. For OKF bundles the two can disagree. Search: "modified", "stale", "timestamp" in `status.rs`/`doctor.rs`.

## Risks / Decisions where OKF conflicts with wikid's Obsidian conventions (Section 4)

1. **Broken links: error vs tolerated.** wikid ranks `BrokenLinks` as `High` severity (`doctor.rs`, `Check::BrokenLinks`), treating them as defects to fix — an Obsidian/authored-wiki stance. OKF §5.3 requires consumers to **tolerate** broken links as normal. *Decision*: introduce an OKF `DoctorProfile` (the mechanism already exists alongside `LlmWiki`/`Strict`) that downgrades `BrokenLinks` to informational and enables the `type`-required check; do not change the default `LlmWiki` philosophy.
2. **Bare wikilink resolution is more permissive than OKF.** Obsidian `[[stem]]` links resolve by global basename match (`links.rs:330/341`); OKF §5.2 implies directory-relative resolution for bare paths. wikid's behavior is a tolerant superset — it will resolve links OKF might consider broken, never the reverse — so it does not break OKF consumption, but it can mask a link an OKF-strict producer would flag.
3. **`type` requirement vs Obsidian's typeless notes.** OKF §9 requires every concept to declare `type`; Obsidian vaults routinely have none. Making a `type` check default-on would flag ordinary vaults en masse. *Decision*: gate it behind the OKF profile (per risk 1), not the default profile.
4. **`README.md` is a wikid/Obsidian convention, not OKF-reserved.** wikid treats `README.md` as a root-navigation peer of `index.md` (`doctor.rs:442`); OKF reserves only `index.md`/`log.md`. Under an OKF profile, `README.md` would be an ordinary concept requiring a `type`. *Decision* needed: keep the wikid README exemption, or honor OKF strictly under the OKF profile.
5. **`timestamp` semantics.** OKF `timestamp` = "last meaningful change" (author-declared); wikid uses filesystem mtime everywhere. These diverge after checkouts, syncs, or mechanical edits. *Decision*: for OKF bundles, prefer frontmatter `timestamp` (when present) over mtime in `status`/`stale` — or document that wikid reports mtime regardless.
6. **`index.md` frontmatter.** OKF forbids frontmatter in `index.md` except the root's optional `okf_version` (§6, §11). wikid's scaffold already emits frontmatter-free `index.md` files (conformant), but its doctor `missing_frontmatter`/`malformed_frontmatter` checks must continue to exempt `index.md`/`log.md` so they are never flagged for lacking frontmatter under any profile.

## Suggested Bead Breakdown (Section 5)

Sub-issues under `wikid-kre` (titles + one-line scope). Ordered so prerequisites come first.

1. **Fix wikid-kre description: only `type` is required** — Correct the bead's "required keys: type, resource, tags, timestamp" to reflect OKF §4.1 (only `type` required; the rest recommended/optional).
2. **Add frontmatter accessors for OKF keys** — Expose `type`/`resource`/`timestamp`/`description` readers in `frontmatter.rs` beside `aliases()`/`page_title()`; store, never reject, unknown keys (§4.2). Prerequisite for later items.
3. **Add `type`-required doctor check + OKF profile** — New `Check` variant asserting non-empty `type` on non-reserved `.md`; exempt `index.md`/`log.md` hierarchy-wide (not root-only, not `README.md`); gate behind a new `DoctorProfile::Okf`.
4. **OKF profile: downgrade broken-link severity** — Under `DoctorProfile::Okf`, treat `BrokenLinks` as informational per §5.3 while keeping `High` in `LlmWiki`/`Strict`.
5. **Surface frontmatter in `ls`/`status`/`grep`** — Add `type`/`title`/`tags` to `Entry` (`ops.rs:35`) and a frontmatter filter to `GrepOptions` (`ops.rs:141`); propagates to CLI/HTTP/MCP via the shared wire format.
6. **OKF bundle detection via `okf_version`** — Read `okf_version` from the root `index.md` (§11); expose a bundle flag in `VaultStatus`. Label any non-`okf_version` heuristic as inference.
7. **`init --okf` scaffold flag** — Optional OKF-flavored scaffold: concept dirs with `type`-bearing example concepts, plain `## YYYY-MM-DD` `log.md` (§7), and root `index.md` with `okf_version: "0.1"`.
8. **`timestamp` vs mtime decision for OKF bundles** — Decide/implement whether `status`/`stale` prefer frontmatter `timestamp` over filesystem mtime for OKF bundles.

## Evidence Index

### Code files (commit 493e6ad)
- `crates/wikid-core/src/frontmatter.rs:12-121` — `Frontmatter` enum, parsing, unknown-key preservation
- `crates/wikid-core/src/frontmatter.rs:45-65` — `aliases()` accessor pattern
- `crates/wikid-core/src/frontmatter.rs:69-84` — `page_title()` precedence
- `crates/wikid-core/src/tags.rs:92-125` — `frontmatter_tags`/`page_tags`/`tags()`
- `crates/wikid-core/src/links.rs:375` — bundle-relative absolute link resolution (`resolve_normalized_exact`)
- `crates/wikid-core/src/links.rs:378-385` — relative `./`/`../` resolution
- `crates/wikid-core/src/links.rs:330,341` — fuzzy bare-stem fallback (`resolve()`)
- `crates/wikid-core/src/doctor.rs:96` — `Check` enum (11 variants)
- `crates/wikid-core/src/doctor.rs:442` — orphan exclusion of root `index.md`/`README.md`
- `crates/wikid-core/src/doctor.rs:700` — `missing_frontmatter` (block-presence only)
- `crates/wikid-core/src/ops.rs:35` — `Entry` struct (no frontmatter fields)
- `crates/wikid-core/src/ops.rs:141` — `GrepOptions` (no frontmatter filter)
- `crates/wikid-core/src/status.rs:28` — `VaultStatus`/`RecentPage` (mtime-based)
- `crates/wikid/src/main.rs:537,649-651` — `init` scaffold dirs and files
- `crates/wikid/src/main.rs:654-688` — `INDEX_TEMPLATE`/`LOG_TEMPLATE` contents
- `crates/wikid-core/src/lib.rs:5-6` — public result types are the shared CLI/HTTP/MCP wire format

### External
- `okf/SPEC.md` — Open Knowledge Format v0.1 (draft), GoogleCloudPlatform/knowledge-catalog; fetched raw 2026-07-06, read locally at `/tmp/okf_spec.md` (451 lines)
- `okf/bundles/{crypto_bitcoin,ga4,stackoverflow}` — reference bundles (confirmed via GitHub API)

### Beads
- `wikid-kre` — parent task; description's required-keys list corrected in this research

---

## Handoff Inputs

**If planning needed** (for prometheus):
- Scope: `wikid-core` (frontmatter accessors, doctor check + profile, `Entry`/`GrepOptions`/`VaultStatus` fields, link-severity gating) and `wikid` CLI (`init --okf` scaffold). `wikid-server` inherits wire-format changes automatically.
- Entry points: `frontmatter.rs:45/69`, `doctor.rs:96/442/700`, `links.rs:375`, `ops.rs:35/141`, `status.rs:28`, `main.rs:649-688`.
- Constraints found: `type` is the only OKF-required key; consumers MUST tolerate missing optional fields/unknown keys/broken links/missing index.md; reserved files (`index.md`/`log.md`) are exempt at every level; `README.md` is a wikid-ism, not OKF-reserved.
- Open questions: README exemption under OKF profile; `timestamp` vs mtime; whether to keep the bracketed/typed `log.md` scaffold format.

**If implementation needed** (for vulkanus/athena):
- Test locations: co-located `#[cfg(test)] mod tests` in each `wikid-core` module (e.g. `frontmatter.rs:123`).
- Pattern to follow: new frontmatter accessor mirrors `aliases()` (`frontmatter.rs:45`); new doctor check mirrors `missing_frontmatter` (`doctor.rs:700`) and registers in the `Check` enum (`doctor.rs:96`); new profile extends `DoctorProfile`.
- Entry point: `crates/wikid-core/src/frontmatter.rs` and `crates/wikid-core/src/doctor.rs`.
