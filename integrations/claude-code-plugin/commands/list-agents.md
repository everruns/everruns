---
description: List agents in the current Everruns organization
argument-hint: "[search query]"
---

List agents on Everruns using the MCP `execute` tool with a `list_agents` command.

Arguments: `$ARGUMENTS`

- If a search term was provided, pass it as `--search "<term>"`.
- Default page size is 20; if the user asked for more, pass `--limit <n>` (max 100).
- Render a compact table with columns: id, name, model, updated_at.
- If an `organization_id` appears in arguments (format `org_{32-hex}`), forward it to the tool.
