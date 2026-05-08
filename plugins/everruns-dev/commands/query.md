---
name: query
description: Inspect Everruns(Dev) platform data with a bash script
argument-hint: "<bash script>"
---

Call the `query` MCP tool with the provided script as `commands`. Only
read-only Everruns(Dev) API operations are available as bash builtins; `jq` is
available. Use `execute` instead when the script needs side effects. See the
`everruns-dev` skill for the command inventory, patterns, and examples. If the
output shape is unclear, call `discover` first and check `output_shape`:
`paginated` uses `.data[]`; `array` uses `.[]`.

Arguments: `$ARGUMENTS`
