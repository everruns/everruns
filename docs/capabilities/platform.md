---
title: Platform
description: Discover, inspect, and manage Everruns resources through the command catalog.
appliesTo: [platform, cloud]
---

| | |
|---|---|
| **ID** | `platform` |
| **Category** | Platform |
| **Risk** | High |
| **Tools** | Native `platform_<command>` functions by default; `everruns` in the session shell with `surface: shell` |
| **Dependencies** | `skills`; `session_file_system` when embedded docs are enabled |
| **Mounts** | `/workspace/docs`, the Everruns documentation, read-only, when embedded docs are enabled |

The Platform capability gives an agent the same catalog-backed command surface
as Everruns' `/mcp` endpoint. Operations come from the server's registered
command inventory, so models can discover current names and schemas instead of
guessing them or relying on a separate handwritten API.

Platform Chat includes this capability by default. Other agents and harnesses
must be assigned it explicitly. Because platform commands can mutate resources,
the capability is high-risk and follows the normal admin-only assignment rule.

## Interfaces and built-in skills

Platform Chat uses native `platform_<command>` functions with bounded
[Tool Search](/capabilities/tool-search/). The full public command inventory is
searchable alongside attached MCP tools; only selected schemas enter the active
native tool view. Search includes capability IDs, names and descriptions plus
MCP server context. A capability-only match asks for refinement rather than
revealing arbitrary operations. Search describes operations, not resource state.

```json
{
  "capabilities": [
    {"ref": "platform"},
    {"ref": "tool_search", "config": {"bounded": true}}
  ]
}
```

Call the discovered function with structured JSON arguments. Arrays and objects
remain JSON values. Mutations accept `reason` and `context_revision`, using the
same history and manager-context checks as the shell.

Platform Chat retains `everruns <noun> <verb>` in its shell for bulk work, with
progressive `--help`, JSON output, pipes, and workspace redirection. Its hybrid
[Tools in Shell](/capabilities/tools-in-shell/) interface also searches the same
registry. `surface: shell` exposes only the command-line interface;
`surface: scripted` explicitly selects the three scripted tools below. Every
interface uses the same command authorization and documentation mounts.

Workspace builds also ship three read-only skills with this capability:

| Skill | Purpose |
|---|---|
| `everruns` | Product concepts, Cloud and self-hosted entry points, and documentation links |
| `everruns-cli` | Command usage, scripting examples, and differences between installed CLI, session shell, and MCP |
| `everruns-agent-authoring` | Agent design, configuration, dependencies, and behavioral verification |

The capability enables [Agent Skills](/capabilities/agent-skills/) for discovery
and activation. Use `list_skills` and `activate_skill`; full instructions load
on demand from `/workspace/.agents/skills/<name>/SKILL.md`. Follow the current
session interface when a skill shows an external MCP example. Adding skills
grants no additional command permissions. Like embedded docs, these product
files are included from the repository at build time; standalone crate builds
without the repository skills tree omit them.

The external [MCP endpoint](/features/mcp/#skills-for-mcp-clients) serves these
three skills plus `everruns-mcp`. Rust development guidance is maintained
separately in the installable `everruns-framework` skill.

## Scripted tools (`surface: scripted`)

### `discover`

Search for operations by name, category, description, or schema terms. Results
include command metadata, read-only decision, and output-shape hints.
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

For commands with a `cli` spelling, use command-tree `--help` as directed by
discovery. Flat-only builtins have no `--help`; use their `bash_usage` and returned
schema. Pass array and
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

## Platform documentation

When the build embeds the product documentation, this capability mounts it at
`/workspace/docs` as a read-only virtual filesystem, served from memory with no
database writes per session. Agents browse it with the standard file tools
(`read_file`, `list_directory`, `grep`) or with `cat`, `ls`, and `grep` through
Bashkit Shell.

Key sections:

- `/workspace/docs/getting-started/`, introduction, concepts, architecture
- `/workspace/docs/features/`, SDK, CLI, UI, events, harnesses, capabilities
- `/workspace/docs/capabilities/`, per-capability reference
- `/workspace/docs/integrations/`, external integrations (Slack, Daytona, etc.)
- `/workspace/docs/advanced/`, budgets, compaction, embedding, network access
- `/workspace/docs/sre/`, environment variables, runbooks

## See also

- [Platform Chat harness](/built-ins/harnesses/platform-chat/)
- [MCP](/features/mcp/)
- [Agent Triggers](/features/agent-triggers/)
- [Platform Management](/capabilities/platform-management/), the removed
  predecessor
