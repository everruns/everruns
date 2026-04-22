description: Run a bash script with full Everruns API access, including side effects
argument-hint: "<bash script>"
---

Call the `execute` MCP tool with the provided script as `commands`. Every
Everruns API operation is a bash builtin; `jq` is available. Prefer `query`
for read-only inspection and use `execute` when the script needs to create,
update, delete, or trigger other side effects. See the `everruns` skill for
the command inventory, patterns, and examples.

Arguments: `$ARGUMENTS`
