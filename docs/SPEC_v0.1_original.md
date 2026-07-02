# SPEC: LLM Wiki Runtime

**Status:** Draft v0.1  
**Product type:** Agent-accessible collaboration layer for LLM Wiki repositories  
**Audience:** Product/engineering agents, founders, technical leads  
**Primary user:** A human or team using an LLM-maintained Markdown wiki as a durable second brain for projects, research, and operational knowledge

---

## 1. Summary

LLM Wiki Runtime is a service/library that exposes an LLM Wiki to humans and agents as a reliable, collaborative knowledge workspace.

The system keeps the LLM Wiki as the primary durable artifact: a human-readable, versioned set of Markdown pages that accumulates understanding over time. The runtime makes that wiki usable from laptops, remote virtual machines, agent sessions, and team environments without forcing every agent to manually clone, pull, edit, and push the wiki.

The runtime should help agents read the right context, create session notes, propose wiki updates, summarize new learning, detect wiki quality issues, and support human or trusted-agent review before canonical knowledge is changed.

---

## 2. Problem Statement

A Markdown-based LLM Wiki works well for one person on one machine. It becomes painful when:

- agents run on remote virtual machines;
- multiple agents need fresh context;
- multiple humans or agents collaborate on the same knowledge base;
- agents discover useful knowledge during sessions;
- raw sources such as emails, messages, meeting notes, research, or code need to be compiled into stable wiki pages;
- the wiki grows too large for agents to read directly;
- unreviewed agent output risks polluting canonical knowledge.

The current workaround — asking every agent to update a repository, search files manually, and commit changes — is fragile and does not scale well to team or multi-agent workflows.

---

## 3. Product Goal

Create a runtime layer that lets humans and agents interact with an LLM Wiki through stable, task-oriented operations while preserving the wiki as a readable, portable, versioned Markdown artifact.

The product should make the LLM Wiki feel like a collaborative knowledge operating system rather than a folder of files.

---

## 4. Why This Should Exist

The LLM Wiki pattern is not just long-term memory. It is a way to compile scattered information into durable, navigable understanding.

The runtime exists to make that compilation process reliable:

- agents can retrieve focused context instead of reading everything;
- new discoveries can become proposed wiki updates;
- humans can review important knowledge changes;
- teams can collaborate without corrupting the canonical wiki;
- remote agents can participate without manual repository rituals;
- the wiki can be searched, summarized, checked, and maintained as it grows.

---

## 5. Core Principles

1. **Markdown is the canonical artifact**  
   The primary knowledge base remains human-readable Markdown.

2. **Agents maintain drafts, humans or trusted curators publish canon**  
   Agents may create notes and proposals freely. Canonical wiki updates should be explicit and reviewable.

3. **Raw sources and compiled knowledge are different things**  
   Raw material should be preserved separately from synthesized wiki pages.

4. **Context should be retrieved, not dumped**  
   Agents should receive task-relevant sections, not the whole wiki by default.

5. **Every important claim should be traceable**  
   Wiki updates and summaries should be able to point back to sources, sessions, or prior pages.

6. **The runtime should operate the wiki, not own the knowledge**  
   The system may index, expose, validate, and coordinate the wiki, but the durable knowledge should remain portable.

---

## 6. Personas

### 6.1 Solo Builder

A technical user who maintains a personal/project LLM Wiki and runs coding or research agents locally and on remote machines.

Needs:

- remote agents can access fresh context;
- session learnings are captured;
- wiki updates are easy to review;
- the wiki remains usable in normal editors.

### 6.2 Engineering Team

A team that uses an LLM Wiki as shared project knowledge across product, architecture, research, decisions, and operational history.

Needs:

- multiple contributors can read and propose updates;
- canonical knowledge changes are reviewable;
- outdated or contradictory knowledge can be flagged;
- agents do not silently overwrite shared understanding.

### 6.3 Agent Swarm / Multi-Agent Workflow

A system where many agents work on different tasks and need shared context.

Needs:

- each agent can retrieve relevant context;
- each agent can write session notes or discoveries;
- conflicting findings can be surfaced;
- canonical wiki changes are controlled.

---

## 7. Scope

### 7.1 In Scope

The runtime should support:

- reading wiki pages and sections;
- searching the wiki;
- discovering related pages;
- registering raw sources;
- writing session notes;
- summarizing sessions into candidate knowledge;
- proposing edits to canonical wiki pages;
- reviewing, accepting, or rejecting proposed edits;
- checking wiki health;
- exposing the wiki to agents through a stable API surface;
- supporting collaboration between humans and agents.

### 7.2 Out of Scope

The runtime should not attempt to be:

- a general-purpose chat memory provider;
- a proprietary replacement for the Markdown wiki;
- a full document editor;
- a project management tool;
- a source-of-truth database for business entities;
- an autonomous system that silently rewrites canonical knowledge without review;
- a model-specific framework tied to one LLM provider.

---

## 8. Conceptual Model

### 8.1 Wiki

A collection of Markdown pages representing compiled, human-readable knowledge.

Examples:

- project overviews;
- architecture notes;
- decisions;
- research summaries;
- glossaries;
- open questions;
- people or organization notes;
- operational playbooks.

### 8.2 Source

Raw or semi-raw material from which wiki knowledge may be compiled.

Examples:

- email threads;
- messages;
- meeting transcripts;
- research notes;
- documents;
- code investigation notes;
- web research captures;
- uploaded files.

### 8.3 Session

A bounded interaction where a human or agent works on a task and produces temporary notes, findings, decisions, or proposed knowledge updates.

### 8.4 Proposal

A suggested change to the canonical wiki.

A proposal may create, modify, append to, reorganize, or annotate wiki content. It must be reviewable before becoming canonical.

### 8.5 Canonical Page

A wiki page that has been accepted into the durable knowledge base.

### 8.6 Health Report

A structured report describing wiki quality issues, such as stale pages, broken links, missing sources, duplicate pages, contradictions, or unresolved open questions.

---

## 9. Functional Requirements

### 9.1 Wiki Reading

The runtime must allow humans and agents to read wiki content without direct file-system or repository access.

It should support:

- listing available wiki pages;
- reading a full page;
- reading a specific section;
- retrieving page metadata;
- retrieving links and backlinks;
- retrieving related pages.

### 9.2 Wiki Search

The runtime must allow humans and agents to search the wiki for task-relevant context.

It should support:

- query by natural language;
- query by exact text or title;
- filtering by project, area, page type, status, or trust level;
- returning ranked results;
- returning enough context for the agent to decide what to read next.

### 9.3 Source Registration

The runtime must allow humans or agents to register raw sources that may later be compiled into the wiki.

It should support:

- adding a source with metadata;
- linking a source to one or more projects or topics;
- retrieving a source summary;
- linking wiki claims or proposals back to sources.

### 9.4 Session Logging

The runtime must allow agents to write session-local notes without immediately changing canonical wiki pages.

It should support:

- starting a session;
- appending findings, decisions, questions, and dead ends;
- linking notes to pages, sources, or tasks;
- closing a session with a summary;
- turning session learnings into one or more proposals.

### 9.5 Wiki Update Proposals

The runtime must allow agents to propose changes to the wiki.

It should support:

- creating a proposal;
- targeting one or more wiki pages;
- describing the proposed change;
- explaining why the change matters;
- linking to supporting sources or session notes;
- showing a human-readable diff or change summary;
- updating proposal status.

### 9.6 Review and Publication

The runtime must support a review flow before proposed changes become canonical.

It should support:

- listing pending proposals;
- reading proposal rationale;
- comparing proposed changes against current wiki content;
- accepting a proposal;
- rejecting a proposal;
- requesting changes;
- recording reviewer identity and decision reason;
- publishing accepted changes into the canonical wiki.

### 9.7 Wiki Health and Maintenance

The runtime must help maintain wiki quality over time.

It should detect or report:

- broken links;
- orphan pages;
- duplicate or overlapping pages;
- stale pages;
- missing metadata;
- missing source references;
- contradictory claims;
- oversized pages that should be split;
- unresolved open questions;
- pages that have not been updated after relevant new sources were added.

### 9.8 Summarization and Curation

The runtime must help agents summarize new material into wiki-shaped knowledge.

It should support:

- summarizing a session;
- summarizing a source;
- identifying affected wiki pages;
- suggesting new pages;
- suggesting updates to existing pages;
- producing proposals rather than silently publishing changes.

### 9.9 Collaboration

The runtime must support multiple users and agents working against the same wiki.

It should support:

- identifying who created notes, sources, and proposals;
- showing recent activity;
- preventing accidental overwrites of canonical knowledge;
- surfacing concurrent or conflicting proposals;
- distinguishing human-authored, agent-authored, and accepted canonical content.

### 9.10 Permissions and Trust

The runtime should support role-based behavior at the product level.

Example roles:

- reader;
- contributor;
- proposer;
- reviewer;
- maintainer;
- agent;
- curator agent.

The runtime should allow different roles to perform different actions, especially around publishing canonical changes.

---

## 10. API Surface

The runtime should expose a stable API surface usable by agents, CLIs, local tools, and external integrations.

This section defines product-level operations, not implementation details.

---

### 10.1 Wiki Operations

#### `wiki.search`

Searches wiki content and returns relevant pages or sections.

Input:

```json
{
  "query": "string",
  "project": "string",
  "filters": {
    "page_type": "string",
    "status": "string",
    "trust_level": "string",
    "tags": ["string"]
  },
  "limit": 10
}
```

Output:

```json
{
  "results": [
    {
      "uri": "wiki://projects/example/architecture",
      "title": "Architecture",
      "section": "Current Design",
      "summary": "string",
      "relevance_reason": "string",
      "trust_level": "high"
    }
  ]
}
```

Acceptance expectation: an agent can use this operation to find the best starting context for a task.

---

#### `wiki.read`

Reads a wiki page or section.

Input:

```json
{
  "uri": "wiki://projects/example/architecture",
  "section": "string"
}
```

Output:

```json
{
  "uri": "wiki://projects/example/architecture",
  "title": "Architecture",
  "content": "markdown string",
  "metadata": {
    "project": "string",
    "status": "canonical",
    "trust_level": "high",
    "updated_at": "date-time"
  },
  "links": ["wiki://..."],
  "sources": ["source://..."]
}
```

Acceptance expectation: an agent can read canonical context without needing direct repository access.

---

#### `wiki.list`

Lists pages in the wiki.

Input:

```json
{
  "project": "string",
  "filters": {
    "page_type": "string",
    "tags": ["string"]
  }
}
```

Output:

```json
{
  "pages": [
    {
      "uri": "wiki://projects/example/overview",
      "title": "Overview",
      "page_type": "project_overview",
      "summary": "string"
    }
  ]
}
```

---

#### `wiki.related`

Finds related pages, sources, sessions, or proposals.

Input:

```json
{
  "uri": "wiki://projects/example/architecture",
  "include": ["pages", "sources", "sessions", "proposals"]
}
```

Output:

```json
{
  "related": [
    {
      "uri": "wiki://projects/example/decisions",
      "relation": "linked_page",
      "reason": "Referenced by current page"
    }
  ]
}
```

---

### 10.2 Source Operations

#### `source.add`

Registers raw material as a source.

Input:

```json
{
  "source_type": "email | message | transcript | document | note | code_investigation | web_capture | other",
  "title": "string",
  "content_ref": "string",
  "summary": "string",
  "project": "string",
  "tags": ["string"]
}
```

Output:

```json
{
  "uri": "source://example/123",
  "status": "registered"
}
```

Acceptance expectation: new raw material can be made available for later compilation without immediately changing canonical wiki pages.

---

#### `source.read`

Reads or summarizes a registered source.

Input:

```json
{
  "uri": "source://example/123"
}
```

Output:

```json
{
  "uri": "source://example/123",
  "title": "string",
  "summary": "string",
  "content": "string",
  "metadata": {
    "source_type": "string",
    "created_at": "date-time"
  }
}
```

---

### 10.3 Session Operations

#### `session.start`

Starts a bounded working session.

Input:

```json
{
  "project": "string",
  "actor": "string",
  "task": "string"
}
```

Output:

```json
{
  "session_uri": "session://example/123",
  "status": "active"
}
```

---

#### `session.log`

Adds a note, finding, question, decision, or dead end to a session.

Input:

```json
{
  "session_uri": "session://example/123",
  "entry_type": "finding | question | decision | dead_end | note",
  "content": "string",
  "related_uris": ["wiki://...", "source://..."]
}
```

Output:

```json
{
  "status": "logged"
}
```

---

#### `session.close`

Closes a session and produces a summary.

Input:

```json
{
  "session_uri": "session://example/123"
}
```

Output:

```json
{
  "session_uri": "session://example/123",
  "status": "closed",
  "summary": "string",
  "suggested_next_actions": ["string"]
}
```

---

#### `session.propose_updates`

Turns session learnings into wiki update proposals.

Input:

```json
{
  "session_uri": "session://example/123",
  "proposal_intent": "string"
}
```

Output:

```json
{
  "proposals": [
    {
      "proposal_uri": "proposal://example/123",
      "title": "string",
      "target_uris": ["wiki://..."],
      "summary": "string"
    }
  ]
}
```

Acceptance expectation: important knowledge from an agent session can become reviewable wiki changes.

---

### 10.4 Proposal Operations

#### `proposal.create`

Creates a proposed wiki change.

Input:

```json
{
  "title": "string",
  "project": "string",
  "target_uris": ["wiki://..."],
  "change_type": "create_page | update_page | append_section | replace_section | reorganize | annotate",
  "proposed_content": "markdown string",
  "rationale": "string",
  "supporting_uris": ["source://...", "session://...", "wiki://..."]
}
```

Output:

```json
{
  "proposal_uri": "proposal://example/123",
  "status": "pending_review"
}
```

---

#### `proposal.read`

Reads a proposal.

Input:

```json
{
  "proposal_uri": "proposal://example/123"
}
```

Output:

```json
{
  "proposal_uri": "proposal://example/123",
  "title": "string",
  "status": "pending_review",
  "target_uris": ["wiki://..."],
  "change_summary": "string",
  "proposed_content": "markdown string",
  "rationale": "string",
  "supporting_uris": ["source://...", "session://..."]
}
```

---

#### `proposal.list`

Lists proposals by project or status.

Input:

```json
{
  "project": "string",
  "status": "pending_review | accepted | rejected | needs_changes"
}
```

Output:

```json
{
  "proposals": [
    {
      "proposal_uri": "proposal://example/123",
      "title": "string",
      "status": "pending_review",
      "summary": "string"
    }
  ]
}
```

---

#### `proposal.review`

Records a review decision.

Input:

```json
{
  "proposal_uri": "proposal://example/123",
  "decision": "accept | reject | request_changes",
  "reviewer": "string",
  "reason": "string"
}
```

Output:

```json
{
  "proposal_uri": "proposal://example/123",
  "status": "accepted | rejected | needs_changes"
}
```

Acceptance expectation: canonical wiki changes can be reviewed and approved before publication.

---

### 10.5 Publication Operations

#### `wiki.publish`

Publishes accepted proposal changes into the canonical wiki.

Input:

```json
{
  "proposal_uri": "proposal://example/123",
  "publisher": "string"
}
```

Output:

```json
{
  "status": "published",
  "updated_uris": ["wiki://..."],
  "publication_summary": "string"
}
```

Acceptance expectation: accepted knowledge updates become part of the canonical Markdown wiki.

---

### 10.6 Health Operations

#### `wiki.health_check`

Runs wiki quality checks and returns issues.

Input:

```json
{
  "project": "string",
  "checks": [
    "broken_links",
    "orphans",
    "duplicates",
    "stale_pages",
    "missing_sources",
    "contradictions",
    "oversized_pages",
    "open_questions"
  ]
}
```

Output:

```json
{
  "report_uri": "health://example/123",
  "summary": "string",
  "issues": [
    {
      "issue_type": "string",
      "severity": "low | medium | high",
      "uri": "wiki://...",
      "description": "string",
      "suggested_action": "string"
    }
  ]
}
```

Acceptance expectation: users can identify where the wiki is becoming stale, inconsistent, or hard to navigate.

---

### 10.7 Status Operations

#### `runtime.status`

Returns the current state of the runtime and wiki workspace.

Input:

```json
{
  "project": "string"
}
```

Output:

```json
{
  "project": "string",
  "wiki_status": "available",
  "last_updated_at": "date-time",
  "pending_proposals": 0,
  "recent_sessions": 0,
  "health_summary": "string"
}
```

Acceptance expectation: humans and agents can quickly understand whether the wiki is available and whether attention is needed.

---

## 11. User Acceptance Criteria

### AC1: Remote Agent Context Access

Given an agent running on a remote virtual machine, when it needs project context, then it can search and read relevant canonical wiki pages through the runtime without manually cloning or updating the wiki repository.

### AC2: Markdown Remains Canonical

Given a user opens the wiki outside the runtime, then the accepted knowledge is still available as normal human-readable Markdown files.

### AC3: Focused Retrieval

Given a large wiki, when an agent searches for task context, then the runtime returns ranked, focused pages or sections instead of requiring the agent to read the entire wiki.

### AC4: Session Knowledge Capture

Given an agent completes a task, when it closes its session, then the runtime produces a session summary and can convert important learnings into proposed wiki updates.

### AC5: No Silent Canonical Mutation

Given an agent discovers new information, when it wants to update durable knowledge, then the update is created as a proposal unless the actor has explicit permission to publish directly.

### AC6: Reviewable Wiki Changes

Given a proposal exists, when a reviewer inspects it, then they can understand the target pages, proposed content, rationale, and supporting sources before accepting or rejecting it.

### AC7: Source Traceability

Given a proposed wiki update includes factual claims, when a reviewer reads the proposal, then supporting sources, session notes, or prior wiki pages are visible where available.

### AC8: Collaboration Safety

Given multiple agents or users work on the same wiki, when they produce overlapping or conflicting updates, then the runtime surfaces the conflict instead of silently overwriting canonical content.

### AC9: Wiki Health Visibility

Given the wiki grows over time, when a user runs a health check, then the runtime reports issues such as broken links, orphan pages, stale pages, missing sources, duplicate pages, oversized pages, or contradictions.

### AC10: Agent-Friendly API

Given an external agent or tool integrates with the runtime, when it uses the public operations, then it can search, read, log sessions, create proposals, review proposals if permitted, publish if permitted, and check status without relying on hidden internal behavior.

### AC11: Human-Friendly Workflow

Given a human wants to use the wiki directly, when they open it in an editor or Markdown viewer, then the content remains understandable and useful without requiring the runtime UI.

### AC12: Clear Separation of Raw and Compiled Knowledge

Given a new source is added, when the runtime registers it, then the source is available for future compilation but does not automatically become canonical wiki knowledge.

### AC13: Trust Levels Are Visible

Given an agent reads wiki content, when the runtime returns pages or sections, then it indicates whether the content is canonical, draft, proposed, stale, or otherwise lower trust.

### AC14: Activity Is Attributable

Given a session note, source, proposal, review, or publication exists, when it is inspected, then the actor responsible for creating or changing it is visible.

### AC15: Runtime Is Optional for Reading Canon

Given the runtime is unavailable, when a user accesses the wiki artifact directly, then canonical Markdown knowledge remains readable and useful.

---

## 12. Expected User Flows

### 12.1 Agent Starts Work

1. Agent asks runtime for project status.
2. Agent searches for task-relevant context.
3. Agent reads the top canonical pages or sections.
4. Agent starts a session.
5. Agent performs its task using retrieved context.

Success condition: the agent begins with relevant, current context without manual repository operations.

---

### 12.2 Agent Finishes Work

1. Agent logs key findings during the session.
2. Agent closes the session.
3. Runtime produces or stores a session summary.
4. Agent or runtime creates one or more wiki update proposals.
5. Human or curator reviews the proposals.
6. Accepted proposals are published into the canonical wiki.

Success condition: useful discoveries compound into durable wiki knowledge.

---

### 12.3 Human Reviews Knowledge Updates

1. Human lists pending proposals.
2. Human reads proposal summary, rationale, target pages, and supporting sources.
3. Human accepts, rejects, or requests changes.
4. Accepted changes become canonical.

Success condition: canonical knowledge changes remain intentional and reviewable.

---

### 12.4 Wiki Maintenance

1. Human or curator runs a health check.
2. Runtime reports stale, duplicated, missing, or contradictory knowledge.
3. Agent creates maintenance proposals.
4. Human or curator reviews and publishes accepted fixes.

Success condition: the wiki improves over time instead of degrading.

---

## 13. Non-Functional Requirements

### 13.1 Portability

The canonical wiki must remain usable outside the runtime.

### 13.2 Interoperability

The runtime should be accessible from different agent environments, local tools, scripts, and user interfaces through a stable operation surface.

### 13.3 Auditability

Important actions should be attributable and reviewable.

### 13.4 Minimal Context Waste

The runtime should encourage focused context retrieval rather than broad context injection.

### 13.5 Human Legibility

All canonical knowledge should remain understandable to a human reader.

### 13.6 Agent Legibility

The runtime should expose structure, metadata, trust level, and relationships in a form agents can reliably use.

### 13.7 Safe Collaboration

The runtime should make accidental overwrite, duplicated knowledge, stale context, and unsupported claims visible rather than invisible.

---

## 14. Success Metrics

The product is successful when:

- remote agents can start work with useful context in one or two calls;
- users no longer need to manually prompt agents to pull or inspect the wiki repository;
- important agent discoveries become wiki proposals instead of being lost in transcripts;
- canonical wiki changes are reviewable;
- the wiki remains readable and useful outside the runtime;
- wiki health issues are visible;
- the system improves team and multi-agent collaboration without replacing the Markdown wiki.

---

## 15. Explicit Non-Goals

The first version should not optimize for:

- perfect autonomous knowledge curation;
- complex workflow engines;
- replacing human review;
- replacing existing editors;
- choosing a final search/indexing backend;
- supporting every possible external source;
- becoming a generic agent memory database;
- storing hidden knowledge only accessible through the runtime.

---

## 16. MVP Definition

The minimum useful product should enable:

1. remote agents to search and read canonical wiki content;
2. agents to create session logs;
3. agents to create wiki update proposals;
4. humans or curator agents to review proposals;
5. accepted proposals to become canonical Markdown wiki changes;
6. users to run basic wiki health checks;
7. canonical knowledge to remain readable outside the runtime.

This MVP is enough to validate whether the runtime meaningfully improves LLM Wiki usage across laptops, virtual machines, and multi-agent workflows.

---

## 17. Open Product Questions

1. Should direct publishing to canon be allowed for trusted curator agents, or should all canonical changes require human approval?
2. Should source traceability be mandatory for all proposals or only for factual claims?
3. Should the runtime treat personal, team, and company wikis differently?
4. Should rejected proposals remain searchable as historical context?
5. Should stale wiki pages be downgraded automatically or only flagged for review?
6. Should session logs be part of the wiki artifact or runtime-managed auxiliary records?
7. Should the API expose page-level operations only, or section-level operations as first-class objects?

---

## 18. One-Sentence Product Definition

LLM Wiki Runtime is a Git-native, Markdown-first collaboration layer that lets humans and agents search, maintain, review, and grow an LLM-maintained second brain without turning it into a black-box memory database.
