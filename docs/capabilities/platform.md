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
include command metadata, read-only classification, and output-shape hints.
Searches with multiple matches omit schemas and return a refinement hint to
keep the result compact. A query that exactly matches a command name returns
only that command with its schemas and `bash_usage`, a copyable invocation with
the exact supported flags. It also includes bounded `output_fields` paths for
building `jq` filters without guessing field names. If expanded schemas would
make the result too large, the response omits them with a notice while retaining
the authoritative scripting summaries. Use `all: true` only when you truly need
to list the entire scriptable catalog, not for a task-specific lookup.

```json
{ "query": "models" }
```

Once you find a command, discover its exact name before invoking it:

```json
{ "query": "create_agent" }
```

Platform builtins do not implement `--help`. Use `bash_usage` and the returned
schema instead of probing with `--help` or guessing flag names. Pass array and
object values as JSON text, for example
`--capabilities '[{"ref":"mcp:..."}]'`.

Unknown flags are rejected before a command runs. This prevents misspelled
security-sensitive options, such as an authentication flag, from being silently
ignored during a mutation.

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

MCP server command results include both their public resource `id` and a
derived `capability_ref` in the `mcp:<uuid>` form accepted by Agent capability
configuration. Capture JSON results and use `jq` to pass dependent IDs or
capability references to later commands in the same script. No separate MCP
attachment operation is needed.

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

## See also

- [Platform Chat harness](/built-ins/harnesses/platform-chat/)
- [MCP](/features/mcp/)
- [Agent Triggers](/features/agent-triggers/)
- [Platform Management](/capabilities/platform-management/), legacy
  handwritten compatibility capability
