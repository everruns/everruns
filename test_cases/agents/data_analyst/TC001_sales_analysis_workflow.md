# TC001: Data Analyst Harness - Sales Analysis Workflow

## Description

Verify that an agent running on the **Data Analyst** harness can complete a realistic end-to-end data analysis workflow: load sales data into a SQL database, run analytical queries with self-validation, render a chart via OpenUI, and persist a correction to cross-session memory for future use.

This test exercises the 6-step analysis pipeline baked into the Data Analyst harness (recall → inspect → plan → execute → visualize → learn) and validates that all bundled capabilities wire together: `session_sql_database`, `memory`, `openui`, `stateless_todo_list`, `data_knowledge`, plus the Generic parent capabilities.

## Preconditions

- Control-plane running (`just start-dev` or `just start-all`)
- LLM API keys configured via environment variables (OpenAI, Anthropic, or Gemini)
- Built-in `data-analyst` harness provisioned (automatic on org init or reconciliation)
- Default org exists with at least one LLM model enabled

## Test Data

| Field | Value |
|-------|-------|
| Harness | `data-analyst` (built-in, inherits from `generic`) |
| Agent Name | Sales Analyst |
| Database name | `sales` |
| Table | `orders(id, product, category, amount, order_date)` |

### Sample orders CSV (sent inline in the first user message)

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

### User messages

| Turn | Message |
|------|---------|
| 1 | Load this orders data into a SQL database called `sales` and tell me which category has the highest total revenue. Show a bar chart. Here is the CSV: *(CSV above)* |
| 2 | Actually, revenue should be calculated net of refunds. For this dataset we have no refunds, but remember this for future sessions. Then re-confirm the top category. |

## Steps

### 1. Resolve the Data Analyst harness

```bash
curl -s "http://localhost:9300/api/v1/harnesses/data-analyst" | jq '{id: .id, name: .name, parent: .parent_harness_id, capabilities: [.capabilities[].ref]}'
```

Expected: harness exists with `parent_harness_id` pointing at `generic`, and capabilities include `session_sql_database`, `memory`, `openui`, `stateless_todo_list`, `data_knowledge`.

Save the harness `id` for subsequent calls.

### 2. Create the Sales Analyst agent

```bash
curl -s -X POST "http://localhost:9300/api/v1/agents" \
  -H "Content-Type: application/json" \
  -d '{
    "name": "Sales Analyst",
    "description": "Sales data analyst backed by the Data Analyst harness",
    "system_prompt": "You are a Sales Analyst. You analyze sales data, produce charts, and remember corrections across sessions."
  }'
```

Save `agent_id` from the response.

### 3. Create a session on the Data Analyst harness

```bash
curl -s -X POST "http://localhost:9300/api/v1/sessions" \
  -H "Content-Type: application/json" \
  -d '{
    "agent_id": "{agent_id}",
    "harness_name": "data-analyst"
  }'
```

Save `session_id` from the response.

### 4. Send turn 1 (load data + analyze + chart)

```bash
curl -s -X POST "http://localhost:9300/api/v1/sessions/{session_id}/messages" \
  -H "Content-Type: application/json" \
  -d '{
    "message": {
      "role": "user",
      "content": [{"type": "text", "text": "Load this orders data into a SQL database called sales and tell me which category has the highest total revenue. Show a bar chart.\n\nid,product,category,amount,order_date\n1,Widget-A,Gadgets,29.99,2026-01-03\n2,Widget-B,Gadgets,49.99,2026-01-05\n3,Gizmo-X,Tools,89.00,2026-01-07\n4,Widget-A,Gadgets,29.99,2026-01-10\n5,Gizmo-Y,Tools,129.00,2026-01-15\n6,Widget-C,Gadgets,19.99,2026-01-20\n7,Gizmo-X,Tools,89.00,2026-02-02\n8,Widget-A,Gadgets,29.99,2026-02-10\n9,Gizmo-Z,Tools,199.00,2026-02-14\n10,Widget-B,Gadgets,49.99,2026-02-28\n11,Gizmo-X,Tools,89.00,2026-03-05\n12,Widget-A,Gadgets,29.99,2026-03-12\n13,Gizmo-Y,Tools,129.00,2026-03-18\n14,Widget-C,Gadgets,19.99,2026-03-22\n15,Gizmo-Z,Tools,199.00,2026-03-29"}]
    }
  }'
```

### 5. Wait for completion

Poll `GET /api/v1/sessions/{session_id}/events` until a `session.idled` event appears (allow 60-120 seconds for multi-step SQL + chart workflow).

### 6. Send turn 2 (correction + learning)

```bash
curl -s -X POST "http://localhost:9300/api/v1/sessions/{session_id}/messages" \
  -H "Content-Type: application/json" \
  -d '{
    "message": {
      "role": "user",
      "content": [{"type": "text", "text": "Actually, revenue should be calculated net of refunds. For this dataset we have no refunds, but remember this for future sessions. Then re-confirm the top category."}]
    }
  }'
```

### 7. Wait for completion and collect data

```bash
# Events from the entire session
curl -s "http://localhost:9300/api/v1/sessions/{session_id}/events"

# Verify SQL database was created
curl -s "http://localhost:9300/api/v1/sessions/{session_id}/databases"

# Verify the orders table has 15 rows
curl -s -X POST "http://localhost:9300/api/v1/sessions/{session_id}/databases/sales/query" \
  -H "Content-Type: application/json" \
  -d '{"sql": "SELECT COUNT(*) AS n FROM orders"}'

# Verify category totals (Tools should be top)
curl -s -X POST "http://localhost:9300/api/v1/sessions/{session_id}/databases/sales/query" \
  -H "Content-Type: application/json" \
  -d '{"sql": "SELECT category, SUM(amount) AS total FROM orders GROUP BY category ORDER BY total DESC"}'

# Verify /knowledge/ scaffold is mounted (read-only)
curl -s "http://localhost:9300/api/v1/sessions/{session_id}/fs/knowledge?recursive=true"

# Verify memory was written for the correction
curl -s "http://localhost:9300/api/v1/memory-stores" \
  | jq '.[0].id' \
  | xargs -I {} curl -s "http://localhost:9300/api/v1/memory-stores/{}/memories?q=refund"
```

## Expected Result

### Harness Setup Assertions

| Check | Expected |
|-------|----------|
| Harness exists | `GET /v1/harnesses/data-analyst` returns 200 |
| Inherits Generic | `parent_harness_id` is set and resolves to the Generic harness |
| Bundles data capabilities | Capability list includes `session_sql_database`, `memory`, `openui`, `stateless_todo_list`, `data_knowledge` |
| `/knowledge/` mounted | `GET /v1/sessions/{session_id}/fs/knowledge?recursive=true` returns entries for `tables/`, `business/`, `queries/` |

### Event Lifecycle Assertions (Turn 1)

| Check | Expected |
|-------|----------|
| Turn started | `turn.started` event exists |
| Tools invoked | At least one `tool.called` event with `tool_name: "sql_execute"` (CREATE TABLE + INSERT) |
| Query executed | At least one `tool.called` event with `tool_name: "sql_query"` (GROUP BY category) |
| Self-recall attempted | `tool.called` event with `tool_name: "recall"` before the first SQL call (pipeline step 1) |
| Reasoning completed | `reason.completed` with `success: true` and `has_tool_calls: true` |
| Turn completed | `turn.completed` event exists |
| Session idled | `session.idled` event exists |

### Analysis Correctness Assertions

| Check | Expected |
|-------|----------|
| Database created | `GET /v1/sessions/{session_id}/databases` lists `sales` |
| All rows loaded | `SELECT COUNT(*) FROM orders` returns `15` |
| Category totals correct | `SELECT category, SUM(amount) ... ORDER BY total DESC` returns `Tools` first (922.00) then `Gadgets` (328.91) |
| Assistant answer | Final assistant message for turn 1 identifies `Tools` as the top-revenue category |

### Visualization Assertions (OpenUI)

| Check | Expected |
|-------|----------|
| OpenUI block rendered | Final assistant message for turn 1 contains a ` ```openui ` fenced code block |
| Chart type | Block contains `BarChart` or an equivalent OpenUI chart component |
| Categories shown | Block references both `Gadgets` and `Tools` |

### Memory / Learning Assertions (Turn 2)

| Check | Expected |
|-------|----------|
| Remember tool called | `tool.called` event with `tool_name: "remember"` after turn 2 is sent |
| Memory persisted | Default memory store contains at least one memory mentioning `refund` (kind `correction` or `fact`) |
| Top category reconfirmed | Final assistant message for turn 2 again identifies `Tools` as the top category (no refund rows, so total unchanged) |

### Failure Modes to Catch

| Failure | Symptom |
|---------|---------|
| Harness missing capabilities | `sql_execute` / `remember` / `openui` tools not available to the agent |
| Zero-row result without self-correction | Agent reports "no data" without investigating (pipeline step 4 broken) |
| Assistant skips chart | No `openui` fenced block in the response |
| Memory not persisted | `remember` call missing or memory store empty for `refund` query |

## Validation Commands

```bash
# Assert: harness exists and inherits generic
curl -s "http://localhost:9300/api/v1/harnesses/data-analyst" \
  | jq 'select(.parent_harness_id != null) | .capabilities | map(.ref) | contains(["session_sql_database","memory","openui","data_knowledge"])'

# Assert: 15 orders loaded
curl -s -X POST "http://localhost:9300/api/v1/sessions/{session_id}/databases/sales/query" \
  -H "Content-Type: application/json" \
  -d '{"sql":"SELECT COUNT(*) AS n FROM orders"}' \
  | jq '.rows[0][0] == 15'

# Assert: Tools is top category
curl -s -X POST "http://localhost:9300/api/v1/sessions/{session_id}/databases/sales/query" \
  -H "Content-Type: application/json" \
  -d '{"sql":"SELECT category FROM orders GROUP BY category ORDER BY SUM(amount) DESC LIMIT 1"}' \
  | jq '.rows[0][0] == "Tools"'

# Assert: SQL tools were invoked
curl -s "http://localhost:9300/api/v1/sessions/{session_id}/events" \
  | jq '[.data[] | select(.type == "tool.called") | .data.tool_name] | any(. == "sql_execute")'

# Assert: recall was invoked before first sql_execute (pipeline step 1)
curl -s "http://localhost:9300/api/v1/sessions/{session_id}/events" \
  | jq '[.data[] | select(.type == "tool.called") | .data.tool_name] | index("recall") < index("sql_execute")'

# Assert: OpenUI block present in assistant output
curl -s "http://localhost:9300/api/v1/sessions/{session_id}/events" \
  | jq '[.data[] | select(.type == "output.message.completed") | .data.message.content[].text // empty] | any(test("```openui"))'

# Assert: memory written with refund mention
curl -s "http://localhost:9300/api/v1/memory-stores" \
  | jq '.[0].id' -r \
  | xargs -I {} curl -s "http://localhost:9300/api/v1/memory-stores/{}/memories?q=refund" \
  | jq '.memories | length > 0'
```

## Notes

- The `data-analyst` harness is inspired by OpenAI's Kepler data agent and the open-source [Dash](https://github.com/agno-agi/dash) project. See [`docs/built-ins/harnesses/data-analyst.md`](../../../docs/built-ins/harnesses/data-analyst.md) for the full harness documentation.
- Deterministic model output cannot be guaranteed; assertions target tool usage and database state rather than exact assistant wording.
- If `recall` is called zero times, the pipeline step 1 is not being followed — investigate the harness system prompt.
- Memory persistence spans sessions; a second session with the same agent and harness should surface the refund correction via passive recall.
