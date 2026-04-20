---
description: Search the Everruns API catalog for operations
argument-hint: "<query> | --all"
---

Use the `discover` MCP tool to find Everruns API operations.

Arguments: `$ARGUMENTS`

- With a query (e.g. `create agent`, `sessions`, `mcp`), call `discover` with the `query` parameter.
- With `--all`, call `discover` with `all: true` to list every operation grouped by category.
- Render matches as: `name` (category) — description, followed by a short parameter list.
- Suggest using `/everruns:execute "<bash snippet>"` to actually invoke one of the operations discovered.
