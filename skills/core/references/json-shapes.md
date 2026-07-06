# JSON shapes

## Contents

- [Global rules](#global-rules)
- [skills](#skills)
- [status](#status)
- [ls and tree](#ls-and-tree)
- [cat](#cat)
- [cat --hashes](#cat---hashes)
- [grep](#grep)
- [glob](#glob)
- [write](#write)
- [edit and edit-batch](#edit-and-edit-batch)
- [mv](#mv)
- [rm](#rm)
- [links](#links)
- [tags](#tags)
- [doctor](#doctor)
- [init](#init)
- [token show](#token-show)
- [update](#update)
- [serve startup](#serve-startup)
- [errors](#errors)

## Global rules

Every command accepts `--json`. Success output is one JSON object on stdout. Errors are also stdout and exit 1. Usage errors are clap errors and exit 2.

Optional fields are omitted when absent. Human `hint:` lines do not appear in JSON success output.

## skills

`wikid skills --json` and `wikid skills list --json`:

```json
{"skills":[{"name":"core","description":"This skill teaches agents..."},{"name":"llm-wiki","description":"This skill teaches agents..."}]}
```

Fields:

- `skills`: catalog entries sorted in registry order.
- `name`: CLI skill name.
- `description`: parsed from SKILL.md frontmatter.

`wikid skills get core --json`:

```json
{"name":"core","full":false,"content":"---\nname: wikid-core\n..."}
```

Fields:

- `name`: CLI skill name.
- `full`: true only when `--full` was requested.
- `content`: exact printed skill text. With `--full`, reference documents are appended after the body.

`wikid skills path --json`:

```json
{"path":"/home/me/.local/share/wikid/skills/<crate-version>","version":"<crate-version>"}
```

Fields:

- `path`: materialized version directory, or the named skill directory when a name was passed.
- `version`: wikid crate version.

## status

`wikid status --json`:

```json
{
  "version":"<crate-version>",
  "root":"/wiki",
  "total_pages":3,
  "total_files":1,
  "total_bytes":1200,
  "recent":[{"path":"index.md","modified":"2026-07-06T12:00:00Z"}],
  "doctor_summary":{"high":0,"medium":0,"low":1}
}
```

Fields:

- `version`: wikid package version.
- `root`: absolute vault root. In remote mode this is the server-side path.
- `total_pages`: visible Markdown pages.
- `total_files`: visible non-page files.
- `total_bytes`: bytes across visible files.
- `recent`: up to five most recently modified pages, newest first.
- `doctor_summary`: issue counts from default doctor options.

## ls and tree

`wikid ls --json` and `wikid tree --json`:

```json
{
  "entries":[{"path":"concepts/","kind":"dir","size":0,"modified":"2026-07-06T12:00:00Z"}],
  "total_dirs":1,
  "total_files":0,
  "total_pages":2
}
```

Entry fields:

- `path`: wiki-root-relative path. Directories end with `/`.
- `kind`: `dir`, `file`, or `page`.
- `size`: bytes for files, zero for directories.
- `modified`: RFC3339 UTC timestamp.

Totals cover the full subtree, even when the displayed entries are depth-limited.

## cat

`wikid cat index.md --json`:

```json
{
  "path":"index.md",
  "content":"# Index\n",
  "truncated":false,
  "total_lines":1,
  "total_bytes":8,
  "modified":"2026-07-06T12:00:00Z"
}
```

Windowed reads add `range_start` and `range_end`:

```json
{"path":"index.md","content":"# Index\n","truncated":false,"range_start":1,"range_end":1,"total_lines":1,"total_bytes":8,"modified":"2026-07-06T12:00:00Z"}
```

## cat --hashes

`wikid cat index.md --hashes --json`:

```json
{
  "path":"index.md",
  "lines":[{"line":1,"hash":"e96375231199","text":"# Index"}],
  "truncated":false,
  "total_lines":1,
  "total_bytes":8,
  "modified":"2026-07-06T12:00:00Z"
}
```

Fields mirror `cat`, but `content` becomes `lines`. Each line has its 1-based `line`, 12-character `hash`, and line `text` without the line ending.

## grep

`wikid grep Index --json`:

```json
{
  "matches":[{"path":"index.md","line":1,"text":"# Index"}],
  "total_matches":1,
  "matched_files":1,
  "total_files":3,
  "truncated":false
}
```

With `-C`, match entries may include `context_before` and `context_after` arrays.

Zero matches are a valid JSON result but exit 1:

```json
{"matches":[],"total_matches":0,"matched_files":0,"total_files":3,"truncated":false}
```

## glob

`wikid glob '**/*.md' --json`:

```json
{"entries":[{"path":"index.md","kind":"page","size":8,"modified":"2026-07-06T12:00:00Z"}],"total":1}
```

## write

`wikid write page.md -m '# Page' --json`:

```json
{"path":"page.md","created":true,"bytes":7}
```

`created` is false when an existing page was overwritten.

## edit and edit-batch

`wikid edit page.md --line 1 --hash e96375231199 --new '# New' --json`:

```json
{"path":"page.md","replacements":1,"bytes":6}
```

`edit-batch` reads an array from stdin:

```json
[{"line":1,"expected_hash":"e96375231199","new_text":"# New"}]
```

The success shape is the same as `edit`.

## mv

`wikid mv old.md new.md --json`:

```json
{"from":"old.md","to":"new.md"}
```

## rm

`wikid rm old.md --force --json`:

```json
{"path":"old.md"}
```

Without `--force`, `rm` returns `force_required`.

## links

`wikid links index.md --json`:

```json
{
  "outgoing":[{"raw":"[[concepts/Billing.md]]","target":"concepts/Billing.md","resolved":"concepts/Billing.md","kind":"wikilink"}],
  "backlinks":["README.md"]
}
```

Outgoing fields:

- `raw`: exact link text.
- `target`: target after alias and fragment stripping.
- `resolved`: resolved wiki-root-relative path, omitted/null when unresolved.
- `kind`: `wikilink` or `markdown`.
- `embed`: present only when true.
- `fragment`: present only when the link has `#Heading` or `#^block-id`.

## tags

`wikid tags --json`:

```json
{"tags":[{"tag":"project","count":2,"pages":["index.md"],"implied":true},{"tag":"project/billing","count":2,"pages":["index.md"]}]}
```

Fields:

- `tag`: tag text without leading `#`.
- `count`: occurrences across inline tags and frontmatter tags.
- `pages`: pages containing the tag or descendant tags.
- `implied`: present and true only for nested-tag ancestors never written literally.

## doctor

`wikid doctor --json`:

```json
{
  "issues":[{"check":"broken_links","severity":"high","category":"graph_navigation","path":"index.md","detail":"unresolved link [[Missing]]","suggested_action":"create the target page or update the link"}],
  "counts":{"broken_links":1,"orphan_pages":0},
  "summary":"1 issue: 1 high, 0 medium, 0 low"
}
```

Fields:

- `issues`: all findings, grouped by check order and path.
- `check`: stable check name.
- `severity`: `high`, `medium`, or `low`.
- `category`: human grouping such as `graph_navigation` or `authored_pages`.
- `path`: wiki-root-relative path.
- `detail`: specific finding.
- `suggested_action`: next action.
- `counts`: executed check names to finding counts, including zeros.
- `summary`: one-line summary.

## init

`wikid init --json`:

```json
{
  "path":"/wiki",
  "config_path":"/home/me/.config/wikid/config.toml",
  "wiki_name":"wiki",
  "registered":true,
  "config_created":false,
  "created":["raw/","index.md"],
  "skipped":["log.md"]
}
```

## token show

`wikid token show admin --json`:

```json
{"actor":"admin","token":"wkd_...","config_path":"/home/me/.config/wikid/config.toml"}
```

## update

`wikid update --check --json`:

```json
{"current":"<crate-version>","target":"<crate-version>","action":"already_current","updated":false}
```

When an asset is selected, `asset` may be present.

## serve startup

`wikid serve --json` prints one startup object before serving:

```json
{"config_path":"/home/me/.config/wikid/config.toml","bind":"127.0.0.1:7448","bootstrapped":false,"wikis":[{"name":"wiki","path":"/wiki"}],"admin_token":"admin token written to /home/me/.config/wikid/config.toml (not printed)"}
```

## errors

Human errors:

```text
error[not_found]: not found: missing.md
hint: run ls or glob to discover valid paths
```

JSON errors:

```json
{"error":{"code":"not_found","message":"not found: missing.md","hint":"run ls or glob to discover valid paths"}}
```
