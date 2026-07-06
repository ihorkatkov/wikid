---
name: wikid-llm-wiki
description: This skill teaches agents to maintain a Karpathy-style LLM wiki when they run wikid init, update index.md or log.md, apply SCHEMA.md frontmatter, process raw/ intake, author concepts, entities, questions, syntheses, or split oversized pages.
allowed-tools: Bash(wikid:*)
---

# LLM Wiki maintenance workflow

Use this guide for agent-maintained wikis scaffolded by `wikid init` and linted by `wikid doctor` with its default `llm-wiki` profile.

## 1. Know the layout

`wikid init` creates one navigation root, one maintenance log, raw-source space, and authored-page space:

```text
index.md
log.md
AGENTS.md
raw/
raw/assets/
concepts/
entities/
questions/
syntheses/
```

Default rule: raw inputs stay in `raw/`; authored pages live in `concepts/`, `entities/`, `questions/`, and `syntheses/`.

## 2. Keep index.md as the navigation root

Every new authored page gets linked from `index.md`. Before adding a page:

```sh
wikid grep 'refund policy' -i
wikid cat index.md --hashes
```

After creating or changing a page, add a short index entry under the best section:

```markdown
- [[concepts/refund-policy.md]] — support-owned refund rules and finance escalation points.
```

If no section fits, add one. Keep the index navigable; do not turn it into a transcript.

## 3. Maintain log.md newest-first

Each meaningful maintenance action gets one dated entry near the top of `log.md`, immediately under the title.

Use this heading shape:

```markdown
## [YYYY-MM-DD] action | subject
```

Use one of these action words unless the wiki already has a tighter vocabulary:

```text
ingest
query
synthesis
lint
edit
```

Example:

```markdown
## [2026-07-06] ingest | Stripe refund notes

- Added [[concepts/refund-policy.md]] from `raw/stripe-refunds.md`.
- Linked [[entities/Stripe.md]] and updated billing references.
```

Do not append new work to the bottom. Newest-first order makes recent agent activity visible in one read.

## 4. Follow SCHEMA.md when present

If `SCHEMA.md` exists, read it before authoring pages:

```sh
wikid cat SCHEMA.md
```

Common frontmatter conventions:

```yaml
---
title: Refund policy
type: concept
tags: [finance/refunds]
aliases: [refund rules]
---
```

Keep frontmatter small and stable. Use `aliases` for common names agents will search or link with. Use tags for durable facets, not temporary task state.

## 5. Separate raw intake from authored pages

Put source captures in `raw/` and attachments in `raw/assets/`. Do not rewrite raw sources except by explicit human request.

Turn raw evidence into authored pages:

```text
raw/vendor-email.md            source capture
concepts/refund-policy.md      synthesized concept
entities/Stripe.md             entity page
questions/how-refunds-work.md  reusable answer
syntheses/billing-risks.md     cross-page analysis
```

Default ingest loop:

1. `wikid grep '<source topic>' -i` to find prior work.
2. `wikid write raw/<source>.md` only for the source capture.
3. Create or update authored pages under the appropriate authored directory.
4. Link every new authored page from `index.md`.
5. Add a newest-first entry to `log.md`.
6. Run `wikid doctor` and fix reported authored-page issues.

## 6. Choose the authored page type

Use one default destination per purpose:

- `concepts/` — durable ideas, processes, policies, architecture, terminology.
- `entities/` — people, teams, vendors, systems, products, repos, organizations.
- `questions/` — reusable questions and answers an agent may ask again.
- `syntheses/` — multi-source briefs, comparisons, decisions, and investigations.

When unsure, create a concept page and link it clearly from `index.md`.

## 7. Split oversized pages

Split a page when it becomes hard to scan, mixes unrelated concepts, or doctor reports `oversized_pages`.

Split workflow:

1. `wikid cat <page>.md --hashes --lines <range>` to identify the section boundary.
2. Create the focused child page with `wikid write concepts/<child>.md` or the matching authored directory.
3. Replace the moved section with a short summary and a wikilink to the child page.
4. Add backlinks between parent and child.
5. Update `index.md` if the child page is independently useful.
6. Add a `log.md` entry.
7. Run `wikid doctor`.

## 8. Keep doctor clean where it matters

The default `llm-wiki` doctor profile focuses on authored pages plus root meta pages and reduces noise from raw intake. Treat findings in authored pages as work to fix, especially broken links, malformed frontmatter, broken fragments, duplicate stems, and oversized pages.

Use strict mode only when auditing everything:

```sh
wikid doctor --profile strict
```
