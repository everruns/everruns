---
description: Run an Everruns(Dev) workflow that can modify platform state
argument-hint: "<bash script>"
---

Call the `execute` MCP tool with the provided script as `commands`. Every
Everruns(Dev) API operation is a bash builtin; `jq` is available. Prefer `query`
for read-only inspection and use `execute` when the script needs to create,
update, delete, or trigger other side effects. See the `everruns-dev` skill for
the command inventory, patterns, and examples.

Arguments: `$ARGUMENTS`
