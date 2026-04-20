---
description: Run a bash script against the Everruns API (every operation is a builtin)
argument-hint: "<bash script>"
---

Run a bash script via the `execute` MCP tool. Every Everruns API operation is exposed as a builtin command (e.g. `list_agents`, `create_session`, `get_model`), and `jq` is available for filtering.

Arguments: `$ARGUMENTS`

- Everything after the command name is the script body. Pass it verbatim as `commands`.
- Forward `timeout_ms` only if the user specified one (default 30000, max 60000).
- Forward `organization_id` if the user provided one.
- Print stdout and stderr clearly separated. If the exit code is non-zero, do not retry — surface the error.

Tips for the user:
- Run `/everruns:discover <query>` first if you are not sure which builtin you need.
- Pipe through `jq` for filtering, e.g. `list_agents | jq '.data[] | {id, name}'`.
