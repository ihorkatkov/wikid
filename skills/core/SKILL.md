---
name: wikid-core
description: This skill teaches agents to operate wikid safely when they need to inspect, search, read, edit, link, tag, or lint a plain-Markdown wiki with commands such as wikid status, grep, cat, edit, links, tags, doctor, or remote-mode environment variables.
allowed-tools: Bash(wikid:*)
---

# wikid core workflow

wikid exposes a plain-Markdown wiki through one CLI surface. Use it when an agent needs filesystem-like access to a local wiki, a remote wiki, or a configured wiki without guessing from flags.

Local mode targets a directory:

```sh
wikid --dir ~/wiki status
```

Remote mode targets a daemon entirely from the client:

```sh
WIKID_SERVER=http://127.0.0.1:7448 WIKID_TOKEN=wkd_... WIKID_WIKI=team wikid status
```

If no target flags are passed, wikid uses config discovery. Bare `wikid` is the same as `wikid status`.

Command names covered by this guide and its full references: skills, serve, init, token, update, status, ls, tree, cat, grep, glob, write, edit, edit-batch, mv, rm, links, tags, doctor.

## 1. Orient before changing anything

Start with status, then discover paths, then search content.

```sh
wikid status
wikid ls
wikid tree concepts --depth 2
wikid glob '**/*.md'
wikid grep 'payment provider' -i
```

Expected excerpts:

```text
wiki: team
pages: 42  files: 7  size: 156.4 KiB
health: 0 high  2 medium  9 low
```

```text
concepts/billing.md:12: Stripe is the primary payment provider.
total: 3 matches in 2 files (18 searched)
hint: wikid cat <path> — read a match
```

Grep-before-write rule: always run `wikid grep '<topic>' -i` before creating a page. Prefer updating or linking an existing page over duplicating knowledge.

## 2. Read pages and fragments

Read a page by wiki-root-relative path. Page paths need the `.md` extension.

```sh
wikid cat concepts/billing.md
```

Large reads are truncated by default:

```text
… truncated (730 lines / 58421 bytes total) — use --full or --lines <START-END>
hint: wikid cat concepts/billing.md --lines 1-120 — read a window
```

Use one of these when the default window is not enough:

```sh
wikid cat concepts/billing.md --full
wikid cat concepts/billing.md --lines 120-180
```

Read one heading section or one block anchor:

```sh
wikid cat concepts/billing.md#Refunds
wikid cat concepts/billing.md#^refund-policy
```

Heading fragments return the section through the next same-or-higher ATX heading. Block fragments return the single line with the trailing `^block-id` anchor.

## 3. The edit loop: hash guard every line change

Never edit from memory. wikid edits are optimistic-concurrency edits: read line hashes, send the line number and hash, and let wikid refuse stale writes.

```sh
wikid cat concepts/billing.md --hashes --lines 10-16
```

Example output:

```text
10:8d0c8941b6f4: ## Refunds
11:4a239f71a2cc: Refunds are approved by support.
12:1d7df6b7a9a1: Escalate unusual refunds to finance.
hint: wikid edit concepts/billing.md --line <n> --hash <hash> --new=<text> — replace a line
```

Replace one line:

```sh
wikid edit concepts/billing.md --line 11 --hash 4a239f71a2cc --new='Refunds are approved by support using the refund checklist.'
```

To insert lines, replace the existing line with itself plus embedded newlines. This is the default insertion idiom:

```sh
wikid edit concepts/billing.md --line 11 --hash 4a239f71a2cc --new=$'Refunds are approved by support.\n- New checklist item.'
```

Use the same embedded-newline pattern in `edit-batch` with `new_text`.

Successful output:

```text
edited concepts/billing.md: 1 line replaced (1.4 KiB)
hint: wikid cat concepts/billing.md — verify the change
```

If someone changed the line first, wikid refuses the whole edit:

```text
error[stale_edit]: stale edit in <page>.md: line 3 is now 1d79bf60835e ("THREE"), not 8b5b9db0c13d
hint: the page changed since it was read — run cat <page>.md with hashes and retry with fresh line hashes
```

Feedback loop: stale hash → re-read with `wikid cat <page>.md --hashes --lines N-M` → retry with the new hash.

For multiple replacements in the same page, use `edit-batch`; it is all-or-nothing.

```sh
cat <<'JSON' | wikid edit-batch concepts/billing.md
[
  {"line":11,"expected_hash":"4a239f71a2cc","new_text":"Refunds use the support checklist."},
  {"line":12,"expected_hash":"1d7df6b7a9a1","new_text":"Finance reviews unusual refunds."}
]
JSON
```

For whole-page replacement or new pages, use `write` after grep-before-write:

```sh
wikid grep 'refund policy' -i
cat draft.md | wikid write concepts/refund-policy.md
wikid write concepts/refund-policy.md -m '# Refund policy'
```

Copyable progress checklist:

```text
[ ] wikid status
[ ] wikid grep '<topic>' -i
[ ] wikid cat <page>.md --hashes --lines <start-end>
[ ] wikid edit <page>.md --line <n> --hash <h> --new='<text>'
[ ] wikid cat <page>.md --lines <start-end>
[ ] wikid links <page>.md
[ ] after bulk edits: wikid doctor, then fix what it reports
```

## 4. Link pages deliberately

Use Obsidian-style wikilinks in Markdown:

```markdown
[[concepts/billing.md]]
[[concepts/billing.md|billing notes]]
[[concepts/billing.md#Refunds]]
[[concepts/billing.md#^refund-policy]]
```

Inspect a page's outgoing links and backlinks:

```sh
wikid links concepts/billing.md
```

Excerpt:

```text
outgoing: 2
  [[entities/Stripe.md]] → entities/Stripe.md
  [[Missing]] → (unresolved)
backlinks: 1
  index.md
```

Resolution is Obsidian-compatible for paths, stems, suffixes, attachments, and fragments. wikid also resolves frontmatter aliases for bare wikilinks; that bare-alias behavior is a wikid extension that real Obsidian does not share.

## 5. Track tags

List inline and frontmatter tags:

```sh
wikid tags
```

Excerpt:

```text
#project/billing  4 occurrences  concepts/billing.md, log.md
#project (implied)  4 occurrences  concepts/billing.md, log.md
```

Nested tags imply ancestors. If only `#project/billing` exists, wikid reports `#project` as implied so agents can navigate by parent topic.

## 6. Run hygiene checks after bulk edits

After creating, moving, removing, or batch-editing pages, run doctor:

```sh
wikid doctor
```

Move pages with `mv`; remove pages with `rm` after confirming with `--force`:

```sh
wikid mv concepts/old.md concepts/new.md
wikid rm concepts/obsolete.md --force
```

`wikid rm page.md` refuses without `--force`. `mv` does not rewrite existing links. Follow moves with `wikid doctor --checks broken_links`, then fix any links it reports.

Read structured errors and findings literally. Human errors look like this:

```text
error[not_found]: not found: concepts/Billing
hint: did you mean concepts/Billing.md?
```

`error.code` is stable. `hint:` lines are actionable next steps, not decoration. For full issue data, use JSON:

```sh
wikid doctor --json
```

## 7. More wikid guidance

Use the LLM Wiki guide for agent-maintained wiki conventions:

```sh
wikid skills get llm-wiki
```

Run the full reference when exact JSON fields, doctor checks, or link-resolution precedence matter:

```sh
wikid skills get core --full
```

If this skill was materialized with `wikid skills path`, reference files also exist under `references/`. Otherwise, `wikid skills get core --full` appends them after this body.

Reference files:

- `references/json-shapes.md` — exact `--json` fields per command.
- `references/doctor-checks.md` — every doctor check, severity, and scope.
- `references/link-resolution.md` — wikilink and Markdown-path resolution precedence.
