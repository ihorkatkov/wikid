---
title: Query
summary: Answer questions by reading the wiki first, then filing useful answers back.
tags: [workflow, query, llm-wiki]
---

# Query

Query is the workflow for answering questions against the compiled wiki.

A maintaining agent should first read [[index|LLM Wiki Demo]], then inspect relevant pages such as [[concepts/compounding-knowledge|Compounding Knowledge]] and [[concepts/schema|Schema]]. Only after that should it synthesize an answer.

Good query outputs can become new pages. For example, a comparison, a decision memo, or a project brief should not disappear into chat history if it will be useful again.

This is where the wiki compounds: answers become reusable artifacts.

Related workflow: [[workflow/ingest|Ingest]]. Maintenance workflow: [[workflow/lint|Lint]].
