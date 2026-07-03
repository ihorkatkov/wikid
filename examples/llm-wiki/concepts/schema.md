---
title: Schema
summary: The local operating instructions that make an LLM a disciplined wiki maintainer.
tags: [concept, schema, agent-instructions]
---

# Schema

The schema is the instruction layer for the maintaining agent.

In a real project it might be `AGENTS.md`, `CLAUDE.md`, or another local guide. It should describe:

- directory layout
- page templates
- source-ingest rules
- citation expectations
- link conventions
- lint workflow
- when to ask a human before rewriting synthesis

Without a schema, an agent tends to behave like a generic chatbot. With a schema, it behaves more like a careful maintainer of a Markdown codebase.

The schema governs [[workflow/ingest|Ingest]], [[workflow/query|Query]], and [[workflow/lint|Lint]].

Return to [[index|LLM Wiki Demo]].
