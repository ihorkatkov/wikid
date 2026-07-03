---
title: Raw Sources
summary: Immutable evidence files that the agent reads but does not rewrite.
tags: [concept, sources, evidence]
---

# Raw Sources

Raw sources are the evidence layer of an LLM Wiki.

They can be articles, transcripts, meeting notes, papers, screenshots, PDFs converted to Markdown, or short notes added by a human. The maintaining agent reads them, but should not silently rewrite them.

In this demo, [[raw/source-queue|Source Queue]] stands in for a tiny raw layer. In a real vault, raw sources usually live under a dedicated `raw/` directory and may include checksums or capture metadata.

Raw sources feed [[workflow/ingest|Ingest]]. The result of ingest is the [[concepts/compiled-wiki|Compiled Wiki]].

Return to [[index|LLM Wiki Demo]].
