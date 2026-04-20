---
description: Get full details for an Everruns agent
argument-hint: "<agent_id>"
---

Fetch agent details via the MCP `execute` tool invoking `get_agent --id <agent_id>`.

Arguments: `$ARGUMENTS`

- The first argument must be an agent id (`agent_{32-hex}`). If missing, ask for it.
- Render: id, name, model, harness, instructions (truncated to ~500 chars), created_at, updated_at, capabilities list.
- If the user passes `--full`, print the raw JSON instead.
