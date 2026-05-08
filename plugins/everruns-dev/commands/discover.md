---
name: discover
description: Search the Everruns(Dev) API catalog for available operations
argument-hint: "<query> | --all"
---

Call the `discover` MCP tool. With `--all`, pass `all: true`; otherwise pass
the text as `query`. Focused discovery returns `input_schema`, `output_schema`,
and `output_shape`; use `output_shape` to choose `.data[]` for paginated
outputs or `.[]` for bare arrays. Use `query` for read-only operations and
`execute` for mutating workflows. See the `everruns-dev` skill for examples.

Arguments: `$ARGUMENTS`
