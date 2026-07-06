# Doctor checks

## Contents

- [Profiles and scope](#profiles-and-scope)
- [Issue fields](#issue-fields)
- [Checks](#checks)
- [Broken links](#broken-links)
- [Ambiguous links](#ambiguous-links)
- [Duplicate aliases](#duplicate-aliases)
- [Orphan pages](#orphan-pages)
- [Broken block reference](#broken-block-reference)
- [Broken heading reference](#broken-heading-reference)
- [Missing frontmatter](#missing-frontmatter)
- [Malformed frontmatter](#malformed-frontmatter)
- [Stale pages](#stale-pages)
- [Oversized pages](#oversized-pages)
- [Duplicate stems](#duplicate-stems)
- [Useful commands](#useful-commands)

## Profiles and scope

`wikid doctor` runs structural checks only. No LLM is involved.

The default profile is `llm-wiki`. It focuses on authored pages plus root meta pages when an LLM Wiki layout is detected:

```text
SCHEMA.md
index.md
log.md
concepts/**
entities/**
questions/**
syntheses/**
queries/**
meetings/**
```

Raw captures, assets, generated exports, and other non-authored subtrees are excluded from default authored-page linting. Use strict mode to lint everything:

```sh
wikid doctor --profile strict
```

## Issue fields

JSON issues have this shape:

```json
{"check":"broken_links","severity":"high","category":"graph_navigation","path":"index.md","detail":"unresolved link [[Missing]]","suggested_action":"create the target page or update the link"}
```

- `check`: check name in the table below.
- `severity`: default severity, sometimes adjusted by profile.
- `category`: human grouping.
- `path`: wiki-root-relative page or file.
- `detail`: exact problem.
- `suggested_action`: concrete repair.

## Checks

| Check | Severity | What fires it |
|---|---:|---|
| `broken_links` | high | A wikilink or markdown link resolves to nothing. |
| `ambiguous_links` | medium | A link target matches more than one candidate at the same resolution stage. |
| `duplicate_aliases` | low | Two or more pages claim the same case-insensitive frontmatter alias. |
| `orphan_pages` | low | A page has no inbound links, excluding root `index.md` and `README.md`. |
| `broken_block_reference` | medium | A resolved `#^block-id` fragment has no matching trailing block anchor. |
| `broken_heading_reference` | medium | A resolved `#Heading` fragment has no matching ATX heading. |
| `missing_frontmatter` | low | The wiki uses frontmatter and a linted page lacks it. |
| `malformed_frontmatter` | medium | A leading `---` frontmatter block cannot parse as YAML mapping data. |
| `stale_pages` | low | A page mtime is older than `--stale-days` (default 90). |
| `oversized_pages` | medium | A page is larger than 64 KiB or longer than 1500 lines. |
| `duplicate_stems` | medium/low | Multiple visible files share the same case-insensitive stem. |

## Broken links

Severity: high.

Fires when a link target cannot be resolved. Examples:

```markdown
[[Missing Page]]
[missing](missing.md)
```

Fix by creating the target page or updating the link. Verify with:

```sh
wikid links index.md
wikid doctor --checks broken_links
```

## Ambiguous links

Severity: medium.

Fires when a target matches multiple candidates at one link-resolution stage. Example: `[[Roadmap]]` with both `plans/Roadmap.md` and `archive/Roadmap.md`.

Fix by writing a more specific path:

```markdown
[[plans/Roadmap.md]]
```

Duplicate alias ambiguity is reported as `duplicate_aliases`, not `ambiguous_links`.

## Duplicate aliases

Severity: low.

Fires when two or more pages claim the same frontmatter alias case-insensitively:

```yaml
---
aliases: [billing]
---
```

Fix by keeping the alias on the canonical page or making aliases more specific.

## Orphan pages

Severity: low.

Fires when a page has no inbound links. Root-level `index.md` and `README.md` are excluded.

Fix by linking the page from `index.md` or from its nearest parent topic.

## Broken block reference

Severity: medium.

Fires when a resolved link points at a missing block anchor:

```markdown
[[concepts/billing.md#^refund-policy]]
```

The target page must contain a line ending with the matching anchor:

```markdown
Refund rules live here. ^refund-policy
```

## Broken heading reference

Severity: medium.

Fires when a resolved link points at a missing heading:

```markdown
[[concepts/billing.md#Refunds]]
```

Headings match ATX heading text case-insensitively after trimming. Fix the fragment or add the heading:

```markdown
## Refunds
```

## Missing frontmatter

Severity: low.

Fires only when at least half of pages have frontmatter, which means the wiki is using it. Root meta pages are excluded from missing-frontmatter adoption findings.

Fix by adding the page's minimal schema frontmatter. If `SCHEMA.md` exists, read it first.

## Malformed frontmatter

Severity: medium.

Fires when a page starts with a frontmatter block but the YAML parse or type check fails. Details include a sanitized one-line parse message and line number when available.

Fix the YAML mapping between the opening and closing `---` lines.

## Stale pages

Severity: low.

Fires when a page modification time is older than `--stale-days`; default is 90 days.

Run a focused stale check with:

```sh
wikid doctor --checks stale_pages --stale-days 180
```

Fix by reviewing the page, updating stale claims, and logging the review if the wiki uses `log.md`.

## Oversized pages

Severity: medium.

Fires when a page is larger than 64 KiB or longer than 1500 lines.

Fix by splitting a focused section into a new page, replacing the moved section with a summary and wikilink, then running doctor again.

## Duplicate stems

Severity: medium for page/page duplicates in strict mode. In the default `llm-wiki` profile, page/page remains medium, page/asset may be low, and asset/asset noise is suppressed.

Fires when visible files share the same case-insensitive stem, because bare wikilinks become ambiguous:

```text
concepts/Roadmap.md
archive/roadmap.md
```

Fix by renaming one page or using precise links everywhere.

## Useful commands

```sh
wikid doctor
wikid doctor --json
wikid doctor --checks broken_links,broken_heading_reference
wikid doctor --profile strict
wikid links <page>.md
wikid cat <page>.md#Heading
```
