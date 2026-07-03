---
title: Ingest
summary: Turn one source into durable wiki updates.
tags: [workflow, ingest, llm-wiki]
---

# Ingest

Ingest is the workflow that converts a new source into durable wiki structure.

A maintaining agent should:

1. Read a source from [[raw/source-queue|Source Queue]].
2. Extract stable claims, definitions, and open questions.
3. Update concept pages such as [[concepts/raw-sources|Raw Sources]] or [[concepts/compiled-wiki|Compiled Wiki]].
4. Add cross-links so future readers can navigate without re-discovering the source.
5. Append the action to [[log|Log]].

The important constraint: raw inputs are treated as evidence, while generated pages are treated as synthesis. That boundary keeps the wiki auditable.

Return to [[index|LLM Wiki Demo]].
