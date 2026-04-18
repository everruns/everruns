# TC001: Sales Analysis Workflow

## Description

Verify that an agent on the **Data Analyst** harness can load CSV data into a SQL database, analyze it, render a chart, and persist a correction to cross-session memory.

Exercises the 6-step analysis pipeline (recall → inspect → plan → execute → visualize → learn) and validates that bundled capabilities wire together: `session_sql_database`, `memory`, `openui`, `stateless_todo_list`, `data_knowledge`.

## Preconditions

- Control-plane running (`just start-dev` or `just start-all`)
- LLM API key configured
- Built-in `data-analyst` harness provisioned (automatic on org init)

## Test Data

| Field | Value |
|-------|-------|
| Harness | `data-analyst` |
| Agent name | Sales Analyst |
| Database | `sales` |
| Table | `orders(id, product, category, amount, order_date)` |

### Sample CSV (15 rows)

```
id,product,category,amount,order_date
1,Widget-A,Gadgets,29.99,2026-01-03
2,Widget-B,Gadgets,49.99,2026-01-05
3,Gizmo-X,Tools,89.00,2026-01-07
4,Widget-A,Gadgets,29.99,2026-01-10
5,Gizmo-Y,Tools,129.00,2026-01-15
6,Widget-C,Gadgets,19.99,2026-01-20
7,Gizmo-X,Tools,89.00,2026-02-02
8,Widget-A,Gadgets,29.99,2026-02-10
9,Gizmo-Z,Tools,199.00,2026-02-14
10,Widget-B,Gadgets,49.99,2026-02-28
11,Gizmo-X,Tools,89.00,2026-03-05
12,Widget-A,Gadgets,29.99,2026-03-12
13,Gizmo-Y,Tools,129.00,2026-03-18
14,Widget-C,Gadgets,19.99,2026-03-22
15,Gizmo-Z,Tools,199.00,2026-03-29
```

## Steps

1. **Create an agent** named "Sales Analyst" with the `data-analyst` harness.

2. **Start a session** on that agent.

3. **Send message:** "Load this orders data into a SQL database called `sales` and tell me which category has the highest total revenue. Show a bar chart." — paste the sample CSV above into the message.

4. **Wait for the agent to finish.** It should create the `orders` table, load 15 rows, run an aggregation query, and produce a chart.

5. **Send message:** "Actually, revenue should be calculated net of refunds. For this dataset we have no refunds, but remember this for future sessions. Then re-confirm the top category."

6. **Wait for the agent to finish.** It should save a memory about the refund rule and reconfirm the top category.

## Expected Result

### Harness & Capabilities

- The `data-analyst` harness exists and inherits from `generic`.
- Agent has access to `sql_execute`, `sql_query`, `sql_schema`, `remember`, `recall`, and OpenUI rendering.
- Session workspace contains the `/knowledge/` scaffold (`tables/`, `business/`, `queries/`).

### Turn 1 — Data Loading & Analysis

- Agent calls `recall` before writing SQL (pipeline step 1).
- Agent creates the `orders` table and inserts all 15 rows.
- Agent runs a `GROUP BY category` query and identifies **Tools** ($922.00) as the top category over **Gadgets** ($328.91).
- Agent renders a bar chart via OpenUI comparing the two categories.
- Agent's answer clearly states Tools is the highest-revenue category.

### Turn 2 — Correction & Learning

- Agent calls `remember` to save a correction about revenue being net of refunds.
- Agent reconfirms **Tools** as the top category (totals unchanged since there are no refund rows).
- The memory store contains at least one memory mentioning "refund".

### Failure Modes

| Failure | What to look for |
|---------|-----------------|
| Missing capabilities | `sql_execute`, `remember`, or OpenUI tools not available to the agent |
| Zero-row result without self-correction | Agent reports "no data" without investigating |
| No chart rendered | Response has no OpenUI visualization |
| Memory not persisted | `remember` never called or memory store empty for "refund" |
| Recall skipped | Agent jumps straight to SQL without checking prior knowledge first |

## Notes

- Deterministic model output is not guaranteed; check tool usage and database state rather than exact wording.
- Memory persists across sessions — a second session with the same agent should surface the refund correction via passive recall.
