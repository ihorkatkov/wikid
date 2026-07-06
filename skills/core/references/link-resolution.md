# Link resolution

## Contents

- [Link forms](#link-forms)
- [Wikilink precedence](#wikilink-precedence)
- [Markdown path rules](#markdown-path-rules)
- [Fragments](#fragments)
- [Aliases](#aliases)
- [What doctor validates](#what-doctor-validates)
- [Examples](#examples)

## Link forms

wikid extracts Obsidian wikilinks, embeds, and local markdown links from visible Markdown pages:

```markdown
[[Target]]
[[Target|alias]]
[[Target#Heading]]
[[Target#^block-id]]
![[Target]]
[text](relative/path.md)
![alt](raw/assets/image.png)
```

External markdown links are skipped for graph resolution:

```markdown
[site](https://example.com)
[email](mailto:person@example.com)
```

## Wikilink precedence

Wikilink target resolution uses the first unique match in this order:

| Order | Stage | Example | Result |
|---:|---|---|---|
| 1 | Root path, with or without `.md` | `[[concepts/Billing]]` | `concepts/Billing.md` |
| 2 | Obsidian attachment folder | `[[diagram.png]]` | configured attachment folder match |
| 3 | Unique file stem, case-insensitive | `[[Billing]]` | `concepts/Billing.md` if it is the only Billing stem |
| 4 | Unique path suffix | `[[finance/Billing]]` | `projects/finance/Billing.md` if unique |
| 5 | Unique frontmatter alias | `[[refund rules]]` | page whose `aliases` contains `refund rules` |

At any stage, multiple candidates mean unresolved and doctor reports ambiguity. Later stages do not break ties from earlier stages.

Exact root paths win before stems and aliases. Attachments in `.obsidian/app.json`'s `attachmentFolderPath` can disambiguate duplicate attachment filenames.

## Markdown path rules

Markdown links use path semantics compatible with Obsidian:

| Markdown target | Resolution rule |
|---|---|
| `./x.md` | Relative to the containing page's directory. |
| `../x.md` | Relative to the containing page's directory; escaping the wiki root is unresolved. |
| `/x.md` | From the wiki root. |
| `x.md` | Vault-wide bare path; a root-level match wins over deeper matches. |
| `folder/x.md` | Vault-wide path or unique suffix, depending on candidates. |

Use forward-slash paths in pages and commands. Page paths generally include `.md` in wikid commands.

## Fragments

Fragments are retained on link records and validated after the target page resolves.

Heading fragments:

```markdown
[[concepts/Billing.md#Refunds]]
[text](concepts/Billing.md#Refunds)
```

A heading fragment matches an ATX heading's trimmed text case-insensitively:

```markdown
## Refunds
```

Block fragments:

```markdown
[[concepts/Billing.md#^refund-policy]]
[text](concepts/Billing.md#^refund-policy)
```

A block fragment matches a trailing block anchor on one line:

```markdown
Refund approvals require support review. ^refund-policy
```

Fragment-only links are self-links. These resolve to the containing page and keep the fragment:

```markdown
[[#Refunds]]
[[#^refund-policy]]
[text](#Refunds)
[text](#^refund-policy)
```

For markdown anchors, percent-encoded fragments are decoded before validation.

## Aliases

Frontmatter aliases accept a string or list:

```yaml
---
aliases: [refund rules, refund policy]
---
```

A bare wikilink can resolve through aliases only after path, attachment, stem, and suffix stages fail:

```markdown
[[refund rules]]
```

This bare-alias behavior is a wikid extension. Real Obsidian does not resolve a bare `[[Alias]]` this way.

## What doctor validates

`wikid doctor` validates the link graph after extraction and resolution:

- `broken_links`: no target candidate resolves.
- `ambiguous_links`: multiple candidates match a link target at the same resolution stage.
- `duplicate_aliases`: multiple pages claim the same case-insensitive alias.
- `broken_heading_reference`: target page resolves, but `#Heading` is missing.
- `broken_block_reference`: target page resolves, but `#^block-id` is missing.
- `orphan_pages`: page has no inbound links, excluding root `index.md` and `README.md`.
- `duplicate_stems`: multiple visible files share a case-insensitive stem and can break bare wikilinks.

## Examples

Inspect outgoing links and backlinks:

```sh
wikid links concepts/Billing.md
```

Human excerpt:

```text
outgoing: 3
  [[entities/Stripe.md]] → entities/Stripe.md
  [[#Refunds]] → concepts/Billing.md
  [[Missing]] → (unresolved)
backlinks: 1
  index.md
```

Read the target fragment directly:

```sh
wikid cat concepts/Billing.md#Refunds
wikid cat concepts/Billing.md#^refund-policy
```

After moving or renaming pages, run focused graph checks:

```sh
wikid doctor --checks broken_links,ambiguous_links,broken_heading_reference,broken_block_reference
```
