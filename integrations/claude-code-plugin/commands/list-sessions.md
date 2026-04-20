---
description: List recent Everruns sessions
argument-hint: "[--agent <agent_id>] [--search <title>] [--limit <n>]"
---

List sessions using the MCP `execute` tool with `list_sessions`.

Arguments: `$ARGUMENTS`

- Forward `--agent_id`, `--search`, `--limit`, `--offset` when the user provides them.
- Default `--limit 20`, max 100.
- Render a compact table: id, title, agent_id, status, updated_at.
- Suggest `/everruns:session-status <id>` to inspect any row.
