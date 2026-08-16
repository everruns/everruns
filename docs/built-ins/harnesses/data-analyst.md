---
title: Data Analyst Harness
description: Data analysis harness with SQL databases, persistent memory, interactive charts, and a structured analysis pipeline inspired by OpenAI's Dash.
---

The **Data Analyst** harness extends the [Generic harness](/built-ins/harnesses/generic/) with capabilities for data analysis: SQL databases, persistent cross-session memory, rich visualization via OpenUI, and a curated knowledge scaffold. Its system prompt implements a structured 6-step analysis pipeline inspired by [OpenAI's Kepler data agent](https://openai.com/index/inside-our-in-house-data-agent/) and the open-source [Dash](https://github.com/agno-agi/dash) project.

## When to Use

- Natural-language data analysis (ask questions, get SQL + charts)
- Interactive data exploration with visualization
- Agents that learn from corrections and remember them across sessions
- Analytics workflows grounded in curated knowledge bases (table docs, business rules, validated SQL)

## Configuration

| Property | Value |
|----------|-------|
| **Type** | `data-analyst` |
| **System Prompt** | Structured 6-step analysis pipeline |
| **Default Model** | None (inherits from agent or organization) |

## Analysis Pipeline

The system prompt guides the agent through six steps on every data question:

1. **Recall**: Search persistent memory for corrections, column mappings, and business definitions from earlier sessions
2. **Inspect**: Use `sql_schema` to verify table structure before writing SQL
3. **Plan**: State the query plan: tables, joins, filters, expected grain, and potential pitfalls
4. **Execute & Validate**: Run the query, then validate (zero rows? duplicates? NULL aggregations?). Self-correct if results look wrong
5. **Visualize**: Summarize findings in plain language, then render charts and tables via OpenUI
6. **Learn**: Use `remember` to save corrections and patterns for future sessions

This mirrors the six-layer context pattern described in [OpenAI's data agent blog post](https://openai.com/index/inside-our-in-house-data-agent/) and implemented by [Dash](https://github.com/agno-agi/dash).

## Bundled Capabilities

All [Generic harness capabilities](/built-ins/harnesses/generic/#bundled-capabilities) plus:

| Capability | What it provides |
|------------|-----------------|
| Session SQL Database | `sql_execute`, `sql_query`, `sql_schema`, session-scoped SQLite databases that auto-create on first write |
| Persistent Memory | `remember`, `recall`, `forget`, cross-session memory with passive recall (8 memories auto-injected per turn) |
| OpenUI | Rich interactive charts, tables, dashboards, and KPI cards rendered inline in chat |
| Todo List | `write_todos`, track multi-step analysis tasks |
| Data Knowledge | Mounts `/knowledge/` scaffold with directories for table docs, business rules, and validated SQL patterns |

## Knowledge Files

The harness mounts a `/knowledge/` directory scaffold in every session:

```
/knowledge/
  tables/README.md      # Add one .md per table: columns, types, gotchas
  business/README.md    # Add metric definitions, business rules, domain terms
  queries/README.md     # Add validated .sql files as reusable templates
```

These files are read-only scaffolds. Populate them with your organization's curated knowledge to ground the agent's SQL generation in reality. The agent reads these files before writing any SQL query.

Combined with persistent memory (which accumulates corrections automatically), this implements the layered context pattern:

| Layer | Source |
|-------|--------|
| Table usage & schema | `sql_schema` tool + `/knowledge/tables/` |
| Business annotations | `/knowledge/business/` + AGENTS.md |
| Validated queries | `/knowledge/queries/` |
| Institutional knowledge | MCP servers (Slack, Notion, Confluence) |
| Learning memory | `remember` / `recall` tools |
| Runtime context | `sql_query` / `sql_execute` |

## Example Session

```
User: Load this CSV and tell me which product category has the highest revenue

Agent: [recalls relevant memories] [inspects any existing schema]
       [creates table, imports data]
       [runs SELECT category, SUM(revenue) ... GROUP BY category]
       [validates: 5 categories, no NULLs, totals match]
       [renders bar chart via OpenUI]
       [remembers: "revenue column is net of refunds"]
```

## See Also

- [Generic Harness](/built-ins/harnesses/generic/), the parent harness this extends
- [Capabilities overview](/features/capabilities/), full capability catalog including memory and OpenUI
- [Harnesses feature guide](/features/harnesses/), harness selection and API management
