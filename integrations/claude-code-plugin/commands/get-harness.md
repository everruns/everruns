---
description: Get full details for an Everruns harness
argument-hint: "<harness_id>"
---

Fetch harness details via the MCP `execute` tool calling `get_harness --id <harness_id>`.

Arguments: `$ARGUMENTS`

- Require a harness id. If missing, ask.
- Render: id, name, role, base chain, capabilities, instructions (truncated), updated_at.
- If the user passes `--full`, print the raw JSON instead.
