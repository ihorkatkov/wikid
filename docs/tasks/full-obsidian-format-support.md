# Full Obsidian Format Support (wikid-vg2)

## TL;DR

> **Summary**: Extend wikid-core from MVP wikilink handling to first-class Obsidian vault
> compatibility — embeds/transclusions, inline + frontmatter tags, callouts, block references,
> alias resolution, and `.obsidian/` config awareness — so `links` and `doctor` understand these
> constructs. wikid is a CLI/server for agent use of vaults, **not a renderer**: these constructs
> matter for the link graph, doctor health, and structured reads, never for HTML output.
> **Deliverables**: typed embeds in the link graph, alias-aware resolution, a tag model, callout
> and block-reference awareness, `.obsidian/` attachment-folder respect, plus the doctor checks
> that fall out of each.
> **Effort**: Large (3d+), decomposed into 6 independently-landable phases.
> **Parallel Execution**: Mostly sequential — Phase 1 (link-struct fields) is a soft prerequisite
> for Phases 4/5 because they add fields to the same `Link` struct. Phases 2, 3, 6 are independent
> of each other.

---

## Context

### Original Request
Issue **wikid-vg2 — "Full Obsidian format support"**: go beyond MVP wikilink basics to first-class
Obsidian vault compatibility. Constructs to support: embeds/transclusions (`![[note]]`,
`![[note#heading]]`), tags (inline `#tag` + frontmatter `tags`/`aliases`), callouts (`> [!note]`),
block references (`^block-id`), heading+alias link edge cases, and respecting `.obsidian/` config
where relevant (e.g. attachment folder). Requirement: doctor and links must understand these
constructs — embeds count as links; aliases resolve. Keep it pragmatic.

### Research Findings (verified against code + DESIGN.md)
| Source | Finding | Implication |
|--------|---------|-------------|
| `links.rs:16-23` | `LinkKind { Wikilink, Markdown }`, `#[serde(rename_all="lowercase")]` | Any new variant/field is wire-visible (CLI/HTTP/MCP). |
| `links.rs:26-36` | `Link { raw, target, resolved: Option<String>, kind }` | Field additions land here; co-design them once (Phase 1). |
| `links.rs:82-135` | `extract_links` splits inner on `\|` then `#`, keeps bare target; regex `!?\[\[…\]\]` already matches `![[…]]` | Embeds already extracted but **indistinguishable** from wikilinks; fragment already dropped. |
| `links.rs:188-208` | `LinkIndex::build(files: Vec<String>)` keyed by lowercased stem+filename | **Aliases not indexed; build takes paths only** — alias resolution needs frontmatter data flowed in. |
| `links.rs:215-259` | `resolve()` / `decide()` implement DESIGN §4 precedence exactly | Alias resolution is a genuine **extension** to the §4 contract, not a bugfix. |
| `frontmatter.rs:10-19,89` | `Frontmatter::Present(BTreeMap<String, serde_yaml::Value>)`; `tags`/`aliases` parsed into map but never read | Tag/alias data is already parsed and available — no new parse pass needed. |
| `doctor.rs:94-141` | `Check` enum (8 variants) with `name()`/`severity()`, serialized | New checks are new serialized variants (owner-flag). |
| `doctor.rs:255-311` | `PageScan { rel, frontmatter, links, … }`; `Vault::doctor` reads each page once (fm + extract + resolve) | Reuse this existing per-page read to feed alias/tag data — do **not** add a second pass. |
| `vault.rs` (WalkBuilder) | `hidden(true)` skips all dot-dirs → `.obsidian/` invisible | Reading `.obsidian/app.json` needs a **targeted read**, not un-hiding the walk. |
| `lib.rs:1-30` | "Every public result type … is the wire format shared by CLI, HTTP, and MCP" | Wire-compat is the tightest constraint; `remote.rs:109` deserializes `LinkReport` from the server. |
| `DESIGN.md §4/§5/§10` | Embeds, inline tags, callouts, block refs are **absent** from the MVP blueprint; MCP deferred but "thin adapter over same core" | These features have no prior spec → owner input is genuine; wire stability protects the future MCP too. |
| repo root | Docs live in `docs/` (`DESIGN.md`, `SPEC.md`); no `thoughts/` | Plan filed under `docs/tasks/`. |

### Interview Decisions (defaults applied — research pre-provided, no interview conducted)
- **Not a renderer**: callouts/embeds are modeled for graph + doctor + structured reads only; no HTML.
- **Follow existing patterns**: co-located `#[cfg(test)] mod tests`, shared `vault()`/`knowledge_vault()` fixtures, regex-based extraction, three-stage resolution.
- **No breaking changes preferred**: additive serde fields with `#[serde(default)]` over new enum variants wherever the choice exists (see owner flags).
- **Verification**: `cargo test --workspace` and `cargo clippy --workspace` gate every phase.

---

## Objectives

### Core Objective
Make `links` and `doctor` understand the full set of common Obsidian constructs so that an agent
operating over a real Obsidian vault sees an accurate link graph and accurate health signal.

### Scope
| IN (Must Ship) | OUT (Explicit Exclusions) |
|----------------|---------------------------|
| Embeds typed distinctly in the link graph (still counted as links) | HTML/Markdown rendering of any construct |
| Alias resolution (`[[alias]]` → target via frontmatter `aliases`) | Full Obsidian query/Dataview language |
| Inline `#tag` + frontmatter `tags` extraction into a tag model | Live-preview / editor semantics |
| Fragment capture (`#heading`, `#^block-id`) on links | Rewriting/normalizing links on write |
| Block-reference anchors (`^block-id`) + `#^id` link awareness | Rendering embedded content inline |
| Callout awareness for structured reads (metadata, not render) | Graph visualization |
| `.obsidian/` attachment-folder respect for resolution | Honoring every `.obsidian/` setting (only attachment folder in scope) |
| Doctor checks that fall out of the above | Tag-based CI gating (`--fail-on`, already deferred in DESIGN §10) |

### Definition of Done
- [ ] Embeds are distinguishable from plain wikilinks in `LinkReport`, and still counted as outgoing links + backlinks.
- [ ] `[[alias]]` where `alias` is a frontmatter alias of a page resolves to that page.
- [ ] `#tag` (inline) and frontmatter `tags` are surfaced through a documented core structure.
- [ ] Links retain their `#heading` / `#^block-id` fragment.
- [ ] `doctor` reports the new construct-specific findings (embeds/aliases/block refs) under appropriate severities and respects the `llm-wiki`/`strict` profile split.
- [ ] `.obsidian/`-configured attachment folder participates in resolution when present.
- [ ] `cargo test --workspace` passes; `cargo clippy --workspace` is clean.
- [ ] `DESIGN.md §4/§5` updated to document the extended link model and new checks.

### Must NOT Have (Guardrails)
- No renderer, no inline expansion of embedded content.
- No second file-read pass in doctor/links — reuse the existing per-page read.
- No un-hiding the vault walk to see `.obsidian/`.
- No silent wire-format break: every `Link`/`Check`/`LinkReport` change is classified additive-vs-behavioral and, where behavioral, flagged for the owner.

---

## Verification Strategy

- **Infrastructure exists**: YES — co-located `#[cfg(test)] mod tests`, `test-util` fixtures (`vault()`, `knowledge_vault()`), `assert_cmd`/`predicates` CLI tests, `tower::ServiceExt::oneshot` server tests (DESIGN §8).
- **Approach**: Tests-after per phase, matching the existing crate convention, with each phase adding fixture pages exercising the new construct.
- **Framework**: `cargo test` (Rust built-in), `cargo clippy` for lint.
- **Per-phase gate**: `cargo test --workspace` + `cargo clippy --workspace` must both pass before a phase is considered landable.

---

## ⚑ Owner-Input Decisions (resolve before or during the flagged phase)

These have no prior spec (DESIGN §4/§5 predate these features) and involve real trade-offs. Each
has a recommended default so implementation is not blocked, but the owner should confirm.

1. **⚑ Embed representation (Phase 1) — wire-format shape.**
   Option A: new `LinkKind::Embed` variant. Option B (recommended): keep `LinkKind { Wikilink,
   Markdown }` and add an orthogonal additive field `embed: bool` on `Link`/`ExtractedLink`.
   *Why B*: (a) embeds exist in both syntaxes (`![[…]]` and `![alt](path)`), so embed-ness is a
   separate axis from syntax — a variant would conflate them; (b) a new enum variant breaks an
   *older* reader deserializing a *newer* server's response (remote mode / future MCP), while a
   `#[serde(default)]` bool does not. **Recommend Option B.**

2. **⚑ Alias resolution precedence (Phase 2) — extends DESIGN §4.**
   §4's order is exact-path → stem → path-suffix. Where do aliases sit? Recommended: **after** all
   filename/stem matching (a real file always wins over an alias), and an alias matching ≥2 pages,
   or an alias colliding with a real stem, yields `Ambiguous`. Confirm the precedence and the
   collision policy; this becomes new §4 text.

3. **⚑ New doctor checks (Phases 1/2/4) — new serialized `Check` variants.**
   Candidates: `unresolved_embed` (embed target missing), `broken_block_reference` (`#^id` where the
   anchor doesn't exist), and possibly `broken_heading_reference`. Each is a new serialized enum
   variant + a severity + `llm-wiki`/`strict` scoping. Confirm *which* to add and their severities
   (recommended: unresolved embed = high like broken_links; broken block ref = medium).

4. **⚑ Tag surface (Phase 3) — scope of the feature.**
   Is the deliverable (a) tags attached to existing read/scan structures only, or (b) a new
   first-class surface — CLI `tags` command + `GET /v1/wikis/{wiki}/tags` route? Recommended for
   this issue: **(a)** — attach a `tags` field to the relevant core struct; defer a dedicated
   command/route unless the owner wants agent-facing tag discovery now.

5. **⚑ Fragment validation depth (Phase 4).**
   Capture of `#heading`/`#^block` is cheap and additive. *Validating* that the heading/block exists
   in the target is expensive (must read + parse the target's headings/anchors). Recommended:
   **capture in Phase 1, validate only block references in Phase 4** behind a doctor check; leave
   heading-existence validation deferred unless requested.

6. **⚑ Callout deliverable (Phase 5) — what does "support" mean here?**
   Links inside callouts are *already* extracted (regex runs over raw text). So the incremental value
   is structured-read metadata (e.g. flagging callout blocks / not misparsing `[!note]`).
   Recommended: minimal callout-type awareness for structured reads; **flag for possible deferral**
   if the owner sees no agent-facing need.

7. **⚑ `.obsidian/` scope (Phase 6).**
   Only `attachmentFolderPath` from `.obsidian/app.json` is in scope. Confirm no other settings
   (e.g. `newLinkFormat`, `useMarkdownLinks`) are expected in this issue.

### ✅ Owner decisions (resolved 2026-07-06)

1. **Embed representation**: Option B — additive `embed: bool` field, no new `LinkKind` variant.
2. **Alias precedence**: as recommended — real file/stem always wins; alias collisions → `Ambiguous`.
3. **Doctor checks**: add `unresolved_embed` (high), `broken_block_reference` (medium), **and `broken_heading_reference` (medium)** — see decision 5.
4. **Tag surface**: option (b) — new CLI `tags` command **and** `GET /v1/wikis/{wiki}/tags` route, in addition to the core struct field.
5. **Fragment validation depth**: **validate existence of both `#heading` and `#^block-id`** in the target page via doctor (deviates from the plan's capture-only default for headings; reuse doctor's per-page scan, no second read pass).
6. **Callouts**: **include Phase 5 now** (not deferred).
7. **`.obsidian/` scope**: confirmed — `attachmentFolderPath` only.

---

## Execution Phases

### Dependency Graph
```
Phase 1 (embed flag + fragment field on Link) ──┬──> Phase 4 (block refs: reuse fragment field)
                                                 └──> Phase 5 (callouts: may add field to same struct)
Phase 2 (alias resolution)      ── independent
Phase 3 (tags)                  ── independent
Phase 6 (.obsidian config)      ── independent (light coupling to Phase 2 resolution)
```
Phase 1 first is a *soft* prerequisite: it settles the `Link` struct's new fields once, so Phases 4/5 don't churn the wire format twice. Phases 2, 3, 6 can land in any order.

---

### Phase 1: Distinguish embeds in the link graph

**Goal**: Embeds (`![[note]]`, `![[note#heading]]`, `![alt](path)`) are typed distinctly in
`LinkReport` while still counting as outgoing links and backlinks. Simultaneously introduce a
`fragment` field so `#heading`/`#^block` is no longer silently dropped (co-designed here so Phase 4
reuses it).

**Files** (CONFIRMED):
- `crates/wikid-core/src/links.rs` — add `embed: bool` and `fragment: Option<String>` to `Link` (26-36) and `ExtractedLink` (48-56); set `embed` from the leading `!` in `extract_links` (82-135); capture the `#…` fragment instead of discarding it after the `|`/`#` split.
- `crates/wikid-core/src/lib.rs` — no export change (fields on existing types); confirm doc comment still accurate.
- `crates/wikid/src/render.rs` (~246, `fn links`) — render an embed marker (e.g. `embed →`) in human output; keep JSON as the raw struct.
- `crates/wikid-core/src/doctor.rs` — `PageScan.links` carries the new fields automatically; ensure broken/orphan/backlink logic treats embeds as links (they already flow through `extract_links`).

**Key design decisions**:
- **⚑ Owner flag #1**: `embed: bool` (recommended) vs `LinkKind::Embed`. Plan assumes the bool.
- Fields are additive with `#[serde(default)]` (+ `skip_serializing_if = "…is_none"`/`"is_false"` where the crate already does so) → old readers unaffected.
- `fragment` stores the substring after the first unescaped `#` (heading text, or `^block-id` for block refs), `None` when absent. `target`/`resolved` semantics unchanged.
- **Attachment embeds already resolve — no new indexing needed.** `Vault::links` builds the `LinkIndex` from `self.visible_files()` (`links.rs:276-277`), i.e. *all* visible files, not only `.md` pages. So `![[diagram.png]]`/`![[report.pdf]]` targets are already in the index and resolve today via the standard §4 order; only outgoing-link *extraction* is page-only (`is_page` guard, `links.rs:281`), which is correct (binaries carry no links). Phase 1 therefore does not touch resolution; Phase 6 only refines *where* an unqualified attachment name is looked up when `.obsidian/` names a folder.

**Test behaviors to add** (`links.rs` tests + `render.rs`/CLI):
- Given `![[Note]]`, when extracting, then the link has `embed == true` and `kind == Wikilink`.
- Given `[[Note]]`, when extracting, then `embed == false` (regression: plain wikilinks unchanged).
- Given `![[Note#Section]]`, then `embed == true` and `fragment == Some("Section")` and `target == "Note"`.
- Given `![alt](img.png)`, then `embed == true` and `kind == Markdown`.
- Given a page that only *embeds* another (`![[Target]]`), when computing backlinks for `Target`, then the embedding page appears as a backlink (embeds count as links).
- Given a vault with `logo.png` and a page containing `![[logo.png]]`, when computing that page's outgoing links, then the embed resolves to `logo.png` (regression-guard for attachment indexing) and is not reported broken.
- Given a JSON round-trip of a `Link` produced by an older serializer (no `embed`/`fragment` keys), when deserialized, then `embed == false` and `fragment == None` (wire back-compat).

**Commands**:
```bash
cargo test --workspace   # Expect: PASS (new + existing link/doctor tests)
cargo clippy --workspace # Expect: 0 warnings
```

**Dependencies**: None. Land first.

**Must NOT do**: Add `LinkKind::Embed` without owner sign-off; drop or re-resolve the fragment as part of target resolution (§4 keeps heading "ignored for resolution").

**Acceptance criteria**:
- [ ] Embeds are distinguishable in `LinkReport` and still counted as links + backlinks.
- [ ] `fragment` is populated for `#…` links and defaults cleanly on old payloads.
- [ ] Human `links` output marks embeds; `--json` unchanged except additive fields.
- [ ] `cargo test --workspace` + `cargo clippy --workspace` pass.

---

### Phase 2: Alias resolution from frontmatter

**Goal**: `[[alias]]` resolves to the page declaring `alias` in its frontmatter `aliases`, extending
the DESIGN §4 resolution order.

**Files** (CONFIRMED):
- `crates/wikid-core/src/links.rs` — change `LinkIndex::build` (197-208) to accept alias data alongside paths (e.g. `build(files: Vec<String>, aliases: &[(usize, Vec<String>)])` or a small `PageMeta` struct); add an `by_alias: BTreeMap<String, Vec<usize>>` map; extend `resolve`/`decide` (215-259) to consult aliases *after* stem/suffix matching.
- `crates/wikid-core/src/frontmatter.rs` — add a helper `aliases(&Frontmatter) -> Vec<String>` reading the `aliases` key (string or sequence-of-strings; ignore other shapes), mirroring the existing `title` read at 43-56.
- `crates/wikid-core/src/links.rs` `Vault::links` (265-314) — parse frontmatter for each indexed file (reuse the content already read) to feed aliases into `build`.
- `crates/wikid-core/src/doctor.rs` `Vault::doctor` (270-311) — feed the same alias data into its `LinkIndex` (it already parses each page's frontmatter into `PageScan`, so reuse it — **no second read**).

**Key design decisions**:
- **⚑ Owner flag #2**: precedence — real file/stem always wins; alias matched only if no file match; alias→multiple pages or alias-vs-stem collision → `Ambiguous`.
- Alias matching is case-insensitive, consistent with stem matching.
- `Vault::links` currently builds the index over paths only; it must now read+parse frontmatter for indexed files. Keep this to the pages already being walked; accept the extra parse cost (frontmatter parse is cheap and already done in doctor).

**Test behaviors to add** (`links.rs` tests, extend `index(&[...])` helper at ~431 to accept aliases):
- Given page `entities/acme.md` with `aliases: [ACME Corp, Acme]`, when resolving `[[Acme]]`, then it resolves to `entities/acme.md`.
- Given the same, resolving `[[ACME Corp]]` (alias with spaces), then resolves to that page.
- Given an alias that matches two different pages, when resolving, then `Ambiguous` (and doctor's `ambiguous_links` fires).
- Given an alias equal to an existing file stem, when resolving, then the **file** wins (alias does not override).
- Given `aliases:` as a single string (not a list), then it is still honored.
- Regression: a target that resolves by exact path/stem today resolves identically after the change.

**Commands**:
```bash
cargo test --workspace   # Expect: PASS
cargo clippy --workspace # Expect: 0 warnings
```

**Dependencies**: None (independent of Phase 1).

**Must NOT do**: Let aliases override real files; add a separate frontmatter read pass in doctor.

**Acceptance criteria**:
- [ ] `[[alias]]` resolves via frontmatter `aliases` with the confirmed precedence.
- [ ] Alias collisions surface as `Ambiguous` and are reported by doctor.
- [ ] `DESIGN.md §4` updated to document aliases in the resolution order.
- [ ] `cargo test --workspace` + `cargo clippy --workspace` pass.

---

### Phase 3: Tag extraction (inline `#tag` + frontmatter `tags`)

**Goal**: Surface inline `#tag` and frontmatter `tags` through a documented core structure so agents
and doctor can see a page's tags.

**Files** (CONFIRMED / NEW):
- `crates/wikid-core/src/tags.rs` — **NEW** module: a `tags_re()` and `extract_tags(content) -> Vec<String>` for inline tags, plus `frontmatter_tags(&Frontmatter) -> Vec<String>`; merge/dedupe helper returning a page's full tag set. (New file under confirmed `crates/wikid-core/src/`.)
- `crates/wikid-core/src/lib.rs` — declare `mod tags;` and export the public tag type(s).
- `crates/wikid-core/src/doctor.rs` — `PageScan` may gain a `tags: Vec<String>` field (populated from the already-parsed frontmatter + body).
- **⚑ Owner flag #4**: if surface option (b), also `crates/wikid/src/main.rs`/`render.rs` (CLI `tags` command) and `crates/wikid-server/src/app.rs` (route). Plan defaults to option (a): attach `tags` to a core read/scan struct, no new command/route.

**Key design decisions**:
- Inline tag grammar (the hard part — encode as explicit tests): a tag is `#` followed by a tag char class (letters, digits, `_`, `-`, `/` for nested tags), **not** preceded by a word char, and **excluding**:
  - `#` inside a wikilink fragment (`[[note#heading]]`) — must not be read as a tag.
  - Markdown ATX headings (`# `, `## ` — `#` followed by space).
  - Fenced code blocks and inline code spans.
  - Pure-numeric `#123` (Obsidian rejects digit-only tags).
- Frontmatter `tags` accepts a list or a single string; normalize (strip leading `#`, dedupe, preserve case as authored but compare case-insensitively for dedupe).

**Test behaviors to add** (`tags.rs` tests):
- Given `Body with #project and #area/work`, when extracting, then tags `["project", "area/work"]`.
- Given `[[note#heading]]`, then **no** tag extracted (fragment `#` excluded).
- Given `# Heading` and `## Sub`, then no tags (ATX headings excluded).
- Given a fenced code block containing `#notatag`, then excluded.
- Given `#123`, then excluded (numeric-only).
- Given frontmatter `tags: [alpha, beta]`, then those tags surface; given `tags: alpha`, then `["alpha"]`.
- Given inline `#alpha` and frontmatter `tags: [alpha]`, then the merged set dedupes to one `alpha`.

**Commands**:
```bash
cargo test --workspace   # Expect: PASS
cargo clippy --workspace # Expect: 0 warnings
```

**Dependencies**: None (independent).

**Must NOT do**: Treat `#` in headings, code, or wikilink fragments as tags; build a tag *query* engine (out of scope).

**Acceptance criteria**:
- [ ] Inline + frontmatter tags are extracted, merged, and deduped per the grammar above.
- [ ] The tag set is reachable through a documented core structure.
- [ ] `cargo test --workspace` + `cargo clippy --workspace` pass.

---

### Phase 4: Block references (`^block-id`) and heading/fragment awareness

**Goal**: Recognize block-id anchors (`^block-id`) as targetable locations and treat `[[note#^id]]`
as a block-reference link; optionally validate broken block references via doctor. Builds on the
`fragment` field from Phase 1.

**Files** (CONFIRMED / NEW):
- `crates/wikid-core/src/links.rs` — in `extract_links`, when a fragment starts with `^`, mark the link as a block reference (via the `fragment` value, or an additive `fragment_kind` if the owner wants an explicit discriminator). Add a helper `block_anchors(content) -> Vec<String>` scanning for trailing `^block-id` tokens on lines/blocks.
- `crates/wikid-core/src/doctor.rs` — **⚑ Owner flag #3/#5**: add a `broken_block_reference` `Check` variant (new serialized variant) that fires when a `#^id` link resolves to a page but the `^id` anchor is absent in that page. Reuse the target page's already-scanned content where possible.
- `crates/wikid-core/src/frontmatter.rs`/heading util — heading edge cases: a `#heading` fragment with alias (`[[note#heading|Alias]]`) must split correctly (alias already handled at 82-135; add tests for the combined form).

**Key design decisions**:
- **⚑ Owner flag #5**: default is **capture + block-ref validation only**; heading-existence validation deferred (expensive, needs heading parse of every target).
- Block-anchor grammar: `^` + `[A-Za-z0-9-]` at end of a block/line; not inside code.
- Validation reuses doctor's page scan set to avoid extra reads; if the target wasn't scanned (e.g. an asset), skip validation rather than error.

**Test behaviors to add** (`links.rs` + `doctor.rs` tests):
- Given `[[note#^abc123]]`, when extracting, then `target == "note"`, `fragment == Some("^abc123")`, and it is recognized as a block reference.
- Given `[[note#Heading|Alias]]`, then `target == "note"`, `fragment == Some("Heading")`, alias handled (display), embed false.
- Given a page containing `Some text ^abc123`, when scanning anchors, then `abc123` is found.
- Given `[[note#^missing]]` where `note.md` has no `^missing`, when doctoring, then `broken_block_reference` fires (medium).
- Given `[[note#^abc]]` where `note.md` contains `^abc`, then no finding.
- Given `llm-wiki` profile, then block-ref findings respect the same authored-page scoping as other checks.

**Commands**:
```bash
cargo test --workspace   # Expect: PASS
cargo clippy --workspace # Expect: 0 warnings
```

**Dependencies**: Phase 1 (reuses the `fragment` field).

**Must NOT do**: Validate heading existence (deferred); treat block ids as separate graph nodes beyond link fragments.

**Acceptance criteria**:
- [ ] `#^id` links are recognized as block references and retain their fragment.
- [ ] Heading+alias combined fragments split correctly.
- [ ] Broken block references are reported by doctor (if owner confirms the check).
- [ ] `cargo test --workspace` + `cargo clippy --workspace` pass.

---

### Phase 5: Callout awareness (structured metadata, not rendering)

**Goal**: Recognize Obsidian callout blocks (`> [!note]`, `> [!warning]+`, etc.) as structured
metadata for reads/doctor, without misinterpreting `[!type]` as a link and without rendering.

**Files** (CONFIRMED / NEW):
- `crates/wikid-core/src/callouts.rs` — **NEW** (or a helper in an existing module): `callout_re()` and `extract_callouts(content) -> Vec<Callout>` where `Callout { kind: String, title: Option<String>, foldable: Option<bool> }` (metadata only).
- `crates/wikid-core/src/lib.rs` — declare/export if a new module.
- `crates/wikid-core/src/links.rs` — confirm `[!type]` inside a callout header is **not** captured as a markdown link (regex `!?\[[^\]]*\]\(…\)` requires `(…)`, so `[!note]` without parens is already safe; add a regression test).

**Key design decisions**:
- **⚑ Owner flag #6**: minimal metadata extraction vs deferral. Since links inside callouts are already
  extracted, the incremental value is only structured-read metadata. If the owner sees no agent need,
  **this phase can be deferred** without blocking the others.
- Callout grammar: a blockquote line `> [!TYPE]` optionally followed by `+`/`-` (fold state) and a
  title; nested content is the following `>` lines.

**Test behaviors to add** (`callouts.rs` tests):
- Given `> [!note] Title\n> body`, when extracting, then one callout `kind == "note"`, `title == Some("Title")`.
- Given `> [!warning]-`, then `kind == "warning"`, `foldable == Some(true)` (folded).
- Given `[!note]` in normal text (no blockquote), then **not** a callout.
- Regression: `[!note]` is never captured as a link by `extract_links`.
- Given a callout containing `[[Link]]`, then the wikilink is still extracted as a link (callouts don't hide links).

**Commands**:
```bash
cargo test --workspace   # Expect: PASS
cargo clippy --workspace # Expect: 0 warnings
```

**Dependencies**: Soft on Phase 1 (if callout metadata is attached to a read struct). Can be deferred.

**Must NOT do**: Render callouts; strip callout syntax from content; treat callout types as tags/links.

**Acceptance criteria**:
- [ ] Callout blocks are recognized with kind/title/fold metadata.
- [ ] `[!type]` is never misread as a link.
- [ ] Links inside callouts remain in the link graph.
- [ ] `cargo test --workspace` + `cargo clippy --workspace` pass.

---

### Phase 6: Respect `.obsidian/` config (attachment folder)

**Goal**: When a vault has `.obsidian/app.json` with `attachmentFolderPath`, use it so attachment
resolution/doctor reflect the vault's real layout — without exposing `.obsidian/` to the normal walk.

**Files** (CONFIRMED / NEW):
- `crates/wikid-core/src/vault.rs` — add a **targeted** read of `.obsidian/app.json` (direct `read_to_string` of the known path, bypassing the `hidden(true)` `WalkBuilder`); parse the single key `attachmentFolderPath` via `serde_json`. `.obsidian/` stays invisible to `ls`/`tree`/`grep`/page-walking.
- `crates/wikid-core/src/obsidian_config.rs` — **NEW** (optional): `ObsidianConfig { attachment_folder: Option<String> }` + `load(root) -> ObsidianConfig` (Absent-tolerant: no dir / no file / malformed JSON → empty config, never an error).
- `crates/wikid-core/src/links.rs` / `doctor.rs` — thread the attachment-folder hint into resolution/doctor where an attachment path would otherwise look broken.

**Key design decisions**:
- **⚑ Owner flag #7**: only `attachmentFolderPath` is honored. Missing/malformed config degrades to
  "no config" silently (like malformed frontmatter is tolerated), never a hard error.
- The read is explicit and narrow — do **not** change `WalkBuilder.hidden(true)`; do **not** start
  walking `.obsidian/`.
- Attachment-folder awareness affects resolution/doctor only; it does not make `.obsidian/` contents
  listable.

**Test behaviors to add** (`vault.rs`/`links.rs`/`doctor.rs` tests; extend fixture to write `.obsidian/app.json`):
- Given `.obsidian/app.json` with `{"attachmentFolderPath":"assets"}`, when loading config, then `attachment_folder == Some("assets")`.
- Given no `.obsidian/` dir, then config loads as empty (no error).
- Given malformed `.obsidian/app.json`, then config loads as empty (no error).
- Given the config present, `.obsidian/` still does **not** appear in `ls`/`tree`/`grep`/page walk (existing invisibility regression holds).
- Given an embed/link to an attachment under the configured folder, then doctor does not flag it as broken (resolution honors the folder).

**Commands**:
```bash
cargo test --workspace   # Expect: PASS
cargo clippy --workspace # Expect: 0 warnings
```

**Dependencies**: None (light coupling to Phase 2/1 resolution paths). Independent.

**Must NOT do**: Un-hide the vault walk; honor `.obsidian/` settings beyond the attachment folder; error on missing/bad config.

**Acceptance criteria**:
- [ ] `attachmentFolderPath` is read via a targeted read and honored in resolution/doctor.
- [ ] `.obsidian/` remains invisible to all listing/search/walk operations.
- [ ] Missing/malformed config degrades gracefully.
- [ ] `cargo test --workspace` + `cargo clippy --workspace` pass.

---

## Risks and Mitigations

| Risk | Trigger | Mitigation |
|------|---------|------------|
| Wire-format break for remote/MCP consumers | New `LinkKind`/`Check` variant reaches an older reader | Prefer additive `#[serde(default)]` fields (Phase 1); classify every enum change as owner-flagged; add round-trip back-compat tests. |
| Alias resolution changes existing results | Alias collides with a real stem | Precedence: file/stem always wins; collision → `Ambiguous`; regression test that pre-alias resolutions are unchanged. |
| Inline `#tag` false positives | `#` in headings/code/fragments | Explicit exclusion tests (headings, code fences, `[[note#heading]]`, numeric-only). |
| Double file reads hurt large-vault doctor perf | Adding alias/tag parse as a new pass | Reuse the existing single per-page read in `Vault::doctor`/`Vault::links`. |
| `.obsidian/` accidentally exposed | Un-hiding the walk to read config | Targeted `read_to_string` of the known path only; invisibility regression test. |
| Fragment/field churn across phases | Phases 1/4/5 all touch `Link` | Co-design fields in Phase 1; Phases 4/5 reuse, not re-shape. |

---

## Success Criteria

### Verification Commands
```bash
cargo test --workspace     # All tests pass (new + existing)
cargo clippy --workspace   # 0 warnings
```

### Final Checklist
- [ ] All "IN scope" items present; all "OUT scope" items absent.
- [ ] Every owner-flagged decision (§Owner-Input) confirmed or explicitly deferred.
- [ ] `DESIGN.md §4/§5` updated to reflect the extended link model and any new checks.
- [ ] `cargo test --workspace` + `cargo clippy --workspace` pass on each landed phase.
