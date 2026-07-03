---
title: Compounding Knowledge
summary: Why a maintained wiki differs from repeatedly retrieving raw chunks.
tags: [concept, synthesis, memory]
---

# Compounding Knowledge

Compounding knowledge is the central reason to build an LLM Wiki.

A one-shot retrieval system can answer from relevant chunks, but it usually redoes the same synthesis every time. A maintained wiki keeps the synthesis around:

- cross-links are already added
- summaries already reflect prior sources
- open questions are visible
- contradictions can be tracked
- good answers can become pages

This does not replace raw evidence. It adds a maintained layer above [[concepts/raw-sources|Raw Sources]].

The result is the [[concepts/compiled-wiki|Compiled Wiki]]. The workflows that keep it alive are [[workflow/ingest|Ingest]], [[workflow/query|Query]], and [[workflow/lint|Lint]].

Return to [[index|LLM Wiki Demo]].
