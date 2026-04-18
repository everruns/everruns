# Gap Analysis: OpenAI Dash (Data Agent) vs Everruns

> Analysis of what Everruns needs to support a Dash-like self-learning data agent.
>
> References:
> - [OpenAI: Inside our in-house data agent (Kepler)](https://openai.com/index/inside-our-in-house-data-agent/)
> - [Dash open-source repo (agno-agi/dash)](https://github.com/agno-agi/dash)
> - [Ashpreet Bedi: Dash article](https://www.ashpreetbedi.com/articles/dash)

## Executive Summary

OpenAI's Kepler is a natural-language-to-SQL agent grounded in **6 layers of context** with continuous learning. Dash is its open-source cousin. Everruns has evolved significantly: of the 8 gaps identified in the previous analysis, **4 are now closed** (memory, evals, charts via OpenUI, context management) and **2 are partially closed** (knowledge base via skills/memory hybrid, provenance). Only the **hybrid-search ranking** and **external database connectors** remain as structural gaps.

**Verdict:** Everruns can host a Dash-equivalent agent today by composing existing building blocks (`memory`, `openui`, `skills`, `session_sql_database`, `infinity_context`, `evals`, `subagents`). The remaining work is a `data_analyst` harness + prompt engineering + optional hybrid-search + external DB connectors.

---

## Everruns has Changed — Closed Gaps

### Gap 2 (Cross-Session Memory) → CLOSED
Previously missing. Now implemented as the **Memory Capability** ([`specs/memory.md`](memory.md)):
- Org-scoped memory stores (`mst_...`) with tools `remember`, `recall`, `forget`
- Tagged, importance-scored, typed memories (`fact`, `preference`, `correction`, `procedure`, `context`)
- Passive auto-recall on every turn (configurable count)
- Image attachments per memory; validated capacity limits
- Corrections survive across sessions — the exact pattern Dash's `LearningMachine` implements

### Gap 6 (Evaluation Framework) → CLOSED
Previously missing. Now the **Evals** entity ([`specs/evals.md`](evals.md)):
- Full CRUD over evals / cases / runs / results
- 10 scorer types: `contains`, `regex`, `tool_called`, `tool_call_count`, `turns_within`, `file_contains`, `json_schema`, **`llm_judge`** (Phase 2), etc.
- Each case creates a real debuggable session
- Run summaries: pass rate, avg score, latency, tokens
- Dedicated UI (Evals tab, run comparison)
- Publish-gate on apps
- **Covers Dash's**: golden SQL (via `file_contains` or `contains`), LLM grading (`llm_judge`), regression tracking (run comparison)

### Gap 5 (Data Visualization / Charts) → CLOSED
Previously missing. Now the **OpenUI capability** ([`specs/openui.md`](openui.md)):
- LLM emits ``` ```openui ``` ``` blocks for rich UI components
- ~55 component library (charts, tables, forms, cards, dashboards) via `crates/openui`
- Chat UI splits and renders with `<Renderer>` from `@openuidev/react-ui`
- Degrades gracefully to code block if rendering fails
- Matches Dash's chart rendering capability

### Context Budget / "Infinity Context" → NEW (bonus)
Not strictly required by Dash but highly relevant — long analytical conversations always blow context:
- Infinity Context capability ([`specs/infinity-context.md`](infinity-context.md)) sends only recent messages, provides a `query_history` tool for on-demand retrieval
- Compaction framework ([`specs/compaction.md`](compaction.md)) with strategies `Native`, `ObservationMasking`, `Summarization`, and cascade
- Enabled by default in Generic harness

### Budgeting → NEW (bonus)
Not in Dash, but enterprise data agents need cost limits:
- [`specs/budgeting.md`](budgeting.md) — per-session/agent/user/org budgets in USD/tokens/credits
- Cascades; most restrictive wins; soft limits pause/warn before hard stop
- Maps directly to OpenAI Kepler's "imposed limits on cost"

### Subagents → NEW (bonus)
Dash doesn't have this; Kepler likely does implicitly:
- [`specs/subagents.md`](subagents.md) — `spawn_subagent`, `get_subagents`, `message_subagent`
- Enables parallel analyses, or dedicated "SQL writer" vs "result interpreter" workflows
- Clean context window per subagent

### Tool Search → NEW (bonus)
OpenAI-specific but relevant to data agents with many tools:
- [`specs/tool-search.md`](tool-search.md) — deferred tool loading (~47% token reduction)
- Namespace grouping by capability category
- Native to GPT-5.4+

### Platform Chat Harness → NEW
"Chat" harness pattern for managing the platform from chat — a Kepler-like conversational entry point for the platform itself.

---

## Remaining Gaps (Reduced Scope)

### Gap A: Structured Knowledge Base (6 layers)
**Partially covered.** Dash's 6 layers map across existing Everruns primitives:

| Dash Layer | Everruns Mapping | Gap |
|---|---|---|
| 1. Table usage (schema + query history) | `sql_schema` tool; no query-history | Need query log + retrieval |
| 2. Human annotations | Session filesystem (`/knowledge/business/*.md`) + `agent_instructions` (AGENTS.md) | **Works** but flat |
| 3. Query patterns (validated SQL) | Session filesystem (`/knowledge/queries/*.sql`) | Need semantic lookup |
| 4. Institutional knowledge (Slack/Docs) | MCP servers or custom integrations | **Works** (MCP to Notion/Slack) |
| 5. Learning memory | **Memory capability** | **Works** |
| 6. Runtime schema introspection | `sql_schema` tool | **Works** |

What's still missing: a **dedicated knowledge-base capability** that unifies layers 1–3 behind one `search_knowledge(query, layer?)` tool, with hybrid retrieval. Workaround today: use `grep_files` + `read_file` over `/knowledge/**` — functional but keyword-only.

**Complexity:** Low-Medium if built on top of Memory + filesystem. High if new vector search.

### Gap B: Hybrid Search (Vector + Keyword)
**Still missing.** Both Memory (`recall`) and any knowledge base rely on keyword search today. For good recall at enterprise scale, vector embeddings are needed.
- Add pgvector extension
- Embedding generation (OpenAI `text-embedding-3-*` or local)
- Extend `recall` and any `search_knowledge` tool to combine cosine similarity with `ts_vector`
- Rerank with RRF

**Complexity:** Medium. Memory store already has the right shape to accept embeddings.

### Gap C: NL-to-SQL Pipeline (Data Analyst Harness)
**Infrastructure done; pipeline missing.** All primitives exist (SQL tools, memory, file system, OpenUI, evals). What's missing is the **harness configuration** — see the "Future Harness Types" section of `specs/harness-types.md` which explicitly calls out:
> **Data** — SQL database, file system, sample data for analytics

What to build:
- `data-analyst` harness bundling: `session_sql_database`, `session_file_system`, `memory`, `openui`, `infinity_context`, `web_fetch`, `agent_instructions`
- System prompt templating the 6-layer retrieval pattern (recall → check knowledge files → introspect schema → plan → SQL → validate → interpret → learn)
- An optional `validate_sql` tool (EXPLAIN + row-count sanity before committing)
- Seed a data-agent starter with F1-like sample dataset (matching Dash's demo)

**Complexity:** Low. Mostly a harness + seed.

### Gap D: External Database Connectors
**Still missing.** Session SQLite is fine for demos; real Kepler-like use needs PostgreSQL/Snowflake/BigQuery read-only connectors.
- Candidate home: `integrations/` (crate pattern is already established — Daytona, Browserless, E2B, Sprites ship there)
- Pattern: connection provider (OAuth/API-key) + tools (`external_query`, `external_schema`) + threat-model entry
- Read-only enforcement: `SET TRANSACTION READ ONLY` / service-account scoping
- Credentials stored via existing encrypted `secret_store` / user connections

**Complexity:** High. New integration crate per DB type, but pattern is well-established.

### Gap E: Provenance / Citations
**Still partially missing.** Tools return raw output; no structured `{ sources }` metadata.
- Extend tool result schema with optional `provenance: { sources: [{ type, id, label, url? }] }`
- SQL tools: emit table names + query in provenance
- Memory/knowledge tools: emit entry IDs + similarity scores
- OpenUI or chat UI renders a "Sources" collapsible below answers

**Complexity:** Low. Schema extension + UI component.

---

## Capability Matrix (Updated)

| Feature | Dash/Kepler | Everruns (as of 0.8.13) |
|---|---|---|
| Natural-language → SQL | Core | **SQL tools + harness needed (low effort)** |
| Session-scoped SQL databases | PG/warehouse | Session SQLite via VFS over PG |
| Schema introspection (live) | `introspect_schema` | `sql_schema` tool |
| Cross-session memory | `LearningMachine` | **Memory capability** (equivalent) |
| Curated knowledge (tables/queries/business) | JSON/SQL/JSON files | Session filesystem (needs search) |
| Institutional knowledge (Slack/Notion) | MCP | **MCP** (equivalent) |
| Hybrid search | Vector + keyword | **Missing** |
| Self-correction loop | Auto-retry on error | Agent loop + memory auto-capture (recipe) |
| Evaluation framework | Golden SQL, LLM judge, regression | **Evals capability** (equivalent+) |
| Charting / visualization | Chart components | **OpenUI capability** (equivalent) |
| Institutional knowledge search | Slack/Docs/Notion | MCP-driven |
| Codebase-derived table semantics (Codex) | Codex crawler | **Missing** (could use `/knowledge/*.md` + memory) |
| Cost / PII guardrails | Imposed limits | **Budgeting** + **session limits** + secret store |
| Provenance / citations | Links + query fragments | Partial — needs schema extension |
| Scheduled reports | Implicit | `durable_schedules` |
| Multi-agent / subagent | Implicit | **Subagents** |
| Durable execution | Not mentioned | **Durable engine** (Everruns advantage) |
| Multi-provider LLM | OpenAI only | OpenAI + Anthropic + Gemini (Everruns advantage) |
| Context management | Implicit compaction | **Infinity Context + Compaction** (Everruns advantage) |
| Apps / distribution | N/A | **Apps + channels** (Slack, AG-UI) (Everruns advantage) |

---

## Priority Ranking (Updated)

| Priority | Item | Effort | Status |
|---|---|---|---|
| **P0** | `data-analyst` harness + system prompt | Low | New (ship first) |
| **P0** | Extend `recall` + optional `knowledge_base` cap with pgvector | Medium | Gap B + A |
| **P1** | `validate_sql` tool (EXPLAIN) + 0-row warnings on `sql_query` | Low | Tweak existing |
| **P1** | Seed demo dataset (F1 or similar) via sample_data capability | Low | New seed |
| **P1** | Provenance metadata in tool results | Low | Gap E |
| **P2** | External DB connectors (PG first via `integrations/postgres`) | High | Gap D |
| **P2** | Codex-style schema enrichment from pipeline code | High | Optional; use AGENTS.md workaround |
| **P3** | Dedicated `introspect_schema` depth (column statistics, sample rows) | Medium | Dash-parity polish |

---

## What Everruns Has That Dash Does Not

Still applies and grew stronger:

1. **Durable execution** — sessions survive restarts (event-sourced)
2. **Multi-provider LLM** — OpenAI, Anthropic, Gemini
3. **MCP** — extensible tool ecosystem
4. **Scheduled tasks** — cron via durable engine
5. **Budgeting** — cost/token limits across subjects with soft/hard thresholds
6. **Subagents** — parallel delegated workstreams
7. **Infinity Context + Compaction** — long conversations without truncation
8. **Apps + channels** — deploy to Slack / AG-UI / WhatsApp
9. **Multi-tenant + encryption** — org-scoped, AES-256-GCM envelope encryption
10. **Eval-gated publishes** — block app publish on regression
11. **Harness inheritance** — compose from parent harnesses, no duplication
12. **Integration parity contract** — strict checklist for every integration crate
13. **Reliability test framework** — worker-crash / CP-restart / network-partition tests
14. **Skills Registry** — agentskills.io-compliant portable instruction packages

---

## Implementation Plan (Revised, Short Path)

### Phase 1: Ship the data-analyst harness (days, not weeks)
1. Add `Data` (or `data-analyst`) built-in harness in `crates/server/src/seed.rs`:
   - Capabilities: `session_sql_database`, `session_file_system`, `memory`, `openui`, `infinity_context`, `web_fetch`, `stateless_todo_list`, `agent_instructions`, `sample_data`
   - System prompt: describe 6-layer retrieval + self-correction + result-interpretation flow
   - Starter files under `/knowledge/{tables,business,queries}/` — example markdown + SQL
2. Seed a demo dataset (F1-like) via a new sample-data capability config
3. Update `specs/harness-types.md` — move "Data" from "Future" to shipped

### Phase 2: Sharper retrieval (1 sprint)
4. Add pgvector migration
5. Extend `memory_store` + `recall` tool with embedding-backed hybrid search (keep keyword path as fallback)
6. Optional: new `knowledge_base` capability unifying `/knowledge/**` search with hybrid ranking

### Phase 3: SQL safety + provenance (1 sprint)
7. Add `validate_sql` (EXPLAIN) tool to `session_sql_database` capability
8. Extend `sql_query` result with 0-row and >1k-row warnings
9. Add optional `provenance` field to `ToolExecutionResult`
10. UI: render provenance accordion below assistant messages

### Phase 4: External connectivity (multi-sprint)
11. `integrations/postgres` crate following the parity contract
12. Connection provider (user connection, OAuth/API-key form)
13. `external_query` / `external_schema` tools with read-only enforcement
14. Write threat-model entry; add live-test workflow

### Phase 5: Nice-to-haves
15. Codex-style schema enrichment — a background scheduled task that reads pipeline repos via user connections and updates `/knowledge/tables/*.md` (lossy but 90% of the value without Codex)
16. Evals pre-filled with Dash-style golden SQL cases for the demo dataset
17. Publish an `app` for the data-analyst harness on Slack channel

---

## Conclusion

Since the previous analysis (6 months of main), Everruns has **closed most of the data-agent gaps** by shipping general-purpose primitives: Memory, Evals, OpenUI, Infinity Context, Compaction, Subagents, Budgeting, Apps. The remaining work to deliver a Dash/Kepler equivalent is now mostly **configuration and composition** (harness + prompt + seed data + starter knowledge files), with two genuine engineering efforts: **hybrid search** over memory/knowledge and **external DB connectors**. The Kepler-specific Codex-enrichment layer is the only item without an obvious low-effort path, and it is also the most optional.
