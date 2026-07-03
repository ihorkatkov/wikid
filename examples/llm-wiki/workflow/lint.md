---
title: Lint
summary: Health-check the wiki for broken structure, stale claims, and missing pages.
tags: [workflow, lint, llm-wiki]
---

# Lint

Lint is the workflow that keeps the wiki useful as it grows.

A maintaining agent should check for:

- broken links
- orphan pages
- duplicate concepts
- stale claims
- missing source references
- important repeated terms without their own page
- contradictions between synthesized pages and raw evidence

`wikid doctor` catches structural issues like broken links and orphans. An LLM can add semantic checks: contradictions, outdated claims, and gaps worth researching.

Structural lint supports [[concepts/compiled-wiki|Compiled Wiki]] because generated pages are only valuable if they remain navigable.

Return to [[index|LLM Wiki Demo]].
