---
title: Platform
description: Discover, inspect, and manage Everruns resources through the authoritative command catalog.
---

| | |
|---|---|
| **ID** | `platform` |
| **Category** | Platform |
| **Risk** | High |
| **Tools** | `discover`, `query`, `execute` |
| **Dependencies** | `session_file_system` when embedded docs are enabled |

The Platform capability gives an agent the same catalog-backed command surface
as Everruns' `/mcp` endpoint. Operations come from the server's registered
command inventory, so models can discover current names and schemas instead of
guessing them or relying on a separate handwritten API.

Platform Chat includes this capability by default. Other agents and harnesses
must be assigned it explicitly. Because `execute` can mutate platform resources,
the capability is high-risk and follows the normal admin-only assignment rule.

## Tools

### `discover`

Search for operations by name, category, description, or schema terms. Results
include command metadata, input/output schemas, read-only classification, and
output-shape hints. Use `all: true` to list the entire scriptable catalog.

```json
{ "query": "models" }
```

### `query`

Run a bounded Bashkit script with only read-only Everruns commands available as
builtins. It supports pipes, variables, loops, conditionals, and `jq`.

```json
{ "commands": "list_models | jq '.data[] | {id, model_id, display_name}'" }
```

Commands with mutations or open-world side effects are not available in
`query`. Use it to inspect current state and validate changes.

### `execute`

Run a bounded Bashkit script with the full scriptable command catalog. Use it
for requested create, update, delete, and other mutating operations.

```json
{
  "commands": "create_agent --name 'support-agent' --system_prompt 'Help users.' --default_model_id 'model_...'"
}
```

`execute` is not transactional. If a later command in a script fails, earlier
commands may already have succeeded. Inspect the resulting state with `query`
before retrying.

## Scope and authorization

Platform tools are always bound to the current session's organization. Their
schemas do not accept `organization_id`, and an injected override is rejected.
The server resolves the session's human owner for every distributed call and
applies that caller's normal command permissions. Attaching this capability
does not grant authority the owner does not already have.

## Autonomous workflows

For recurring autonomous work, create an Agent and an Agent Trigger. Do not use
a schedule on the Platform Chat session: that would wake the management chat,
not provision an independently owned worker workflow.

Credentials are not transferred from Platform Chat session secrets into a new
Agent. Configure integrations through their supported Agent-scoped credential
or connection flow; do not paste credentials into command scripts.

## Embedded documentation

When embedded platform docs are enabled, public Markdown documentation is
mounted read-only at `/workspace/docs`. Files come from the repository `docs/`
directory at compile time and do not create per-session database rows.

## See also

- [Platform Chat harness](/built-ins/harnesses/platform-chat/)
- [MCP](/features/mcp/)
- [Agent Triggers](/features/agent-triggers/)
- [Platform Management](/capabilities/platform-management/) — legacy
  handwritten compatibility capability
