---
description: Run a read-only bash script against the Everruns Dev API
argument-hint: "<bash script>"
---

Call the `query` MCP tool with the provided script as `commands`. Only
read-only Everruns Dev API operations are available as bash builtins; `jq` is
available. Use `execute` instead when the script needs side effects. See the
`everruns-dev` skill for the command inventory, patterns, and examples.

Arguments: `$ARGUMENTS`
