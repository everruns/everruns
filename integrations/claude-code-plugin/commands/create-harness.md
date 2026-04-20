---
description: Create a new Everruns harness
argument-hint: "<name> [--base <harness_id>] [--instructions \"...\"]"
---

Create a new harness via the MCP `execute` tool calling `create_harness`.

Arguments: `$ARGUMENTS`

Steps:
1. Parse a name (required) and optional `--base` (harness id to derive from), `--instructions`, `--role`.
2. If no base is specified, call `list_harnesses` and suggest the org base harness as default.
3. Call `create_harness --name "<name>" [--base_id <id>] [--instructions "..."] [--role <role>]`.
4. On success, print the new harness id and suggest assigning it to an agent via `/everruns:create-agent <name> --harness <harness_id>`.
