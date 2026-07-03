---
title: LLM Wiki Demo
summary: A tiny public-safe example of a persistent LLM-maintained wiki.
tags: [llm-wiki, demo, wikid]
---

# LLM Wiki Demo

This demo vault shows the **LLM Wiki** pattern: an agent incrementally turns raw sources and questions into a persistent, interlinked Markdown knowledge base.

It is inspired by [Andrej Karpathy's LLM Wiki gist](https://gist.github.com/karpathy/442a6bf555914893e9891c11519de94f), but the pages here are original demo content rather than a copy of the gist.

## How to read this wiki

Start with the workflow pages:

- [[workflow/ingest|Ingest]] — how new sources become durable wiki pages
- [[workflow/query|Query]] — how answers become reusable artifacts
- [[workflow/lint|Lint]] — how the wiki stays healthy over time

Then inspect the structure:

- [[concepts/raw-sources|Raw Sources]] — immutable input material
- [[concepts/compiled-wiki|Compiled Wiki]] — generated pages that compound over time
- [[concepts/schema|Schema]] — the operating instructions for the maintaining agent
- [[concepts/compounding-knowledge|Compounding Knowledge]] — why this differs from one-shot retrieval

Operational files:

- [[raw/source-queue|Source Queue]] — example raw-source intake
- [[questions/example-queries|Example Queries]] — questions an agent can answer against the wiki
- [[log|Log]] — append-only history of wiki maintenance

## Demo claim

A normal RAG system retrieves chunks at query time. An LLM Wiki compiles useful structure ahead of time: links, summaries, contradictions, open questions, and reusable answers. The value is not only search; it is accumulated organization.
