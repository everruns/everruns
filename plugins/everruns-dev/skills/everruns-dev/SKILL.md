---
name: everruns-dev
description: Reference for working with the Everruns(Dev) managed harnesses platform (https://dev.everruns.com) - core concepts, UI links, entity naming, and API workflows for agents, harnesses, capabilities, sessions, models, and apps.
---

# Everruns(Dev)

Everruns(Dev) is the dev deployment of the durable Everruns agentic harness
engine. Docs: <https://docs.everruns.com>. This plugin talks to the
Everruns(Dev) deployment over MCP and exposes both focused session tools and
scriptable API access.

If a user asks something Everruns(Dev)-related, prefer the MCP tools over guessing.
When the exact shape of an operation is unclear, call `discover` first.

## Core concepts

```
Harness --has--> Capability
   ^
   |has
   |
 Agent --has--> Capability
   ^
   |runs in
   |
 Session --has--> Capability (additive)
```

- **Harness** - top-level setup for agent execution. Configures infrastructure,
  defaults, and constraints. Each session has exactly one. Harnesses support
  single-parent inheritance, so capabilities and prompt chunks flow down the
  chain. Think "runtime profile".
- **Agent** - domain- or task-specific configuration: system prompt, default
  model, enabled capabilities. Not bound to a harness at creation; the harness
  is picked when the session starts. Optional on a session.
- **Capability** - the reusable unit that actually adds behavior. Each
  capability contributes three things: system-prompt additions, tools, and
  files mounted in the session filesystem. Capabilities attach to a harness, an
  agent, or a session. Session capabilities are **additive** to agent
  capabilities.
- **Session** - the working instance of an agentic loop. Assembled from harness
  + (optional) agent + session overrides, then resolved into a `RuntimeAgent`.
  Status values: `started`, `active`, `idle`, `waiting_for_tool_results`,
  `paused`. Sessions live indefinitely.
- **Model** - an LLM model id supported by the target deployment. Resolution
  priority: per-message controls -> session override -> agent default -> harness
  default -> system default. Use the API catalog or model list before naming a
  specific model.
- **MCP server** - a remote server exposing tools; integrated into Everruns(Dev) as
  a virtual capability (`mcp:{uuid}`).
- **App** - binds a Harness + Agent to a channel (Slack, web widget, etc.) for
  external deployment.

**Typical setup flow when a user wants a new agent:**

1. Pick or create a harness (covers infra + base capabilities). Most users
   start from the org base harness.
2. Create an agent: system prompt, default model, capabilities it needs beyond
   what the harness supplies. Agents are not bound to a specific harness - the
   harness is chosen at session start.
3. Run the agent with `agent_run` - that creates a session (using the default
   harness), sends the first user message, and triggers the turn loop. To pin a
   specific harness, create the session directly via
   `create_session --harness_id ...` and then call `create_message`.

Capabilities are the answer to "how does my agent get tool X?" - attach the
capability at the harness level for org-wide defaults, at the agent level for
agent-specific tools, or at the session level for one-off additions.

## UI links

All user-facing outputs should include links to the relevant Everruns(Dev) UI
objects when an object is created, updated, inspected, or returned as a primary
result. Use Markdown links. Default base URL: `https://dev.everruns.com` unless
the user provides another deployment URL.

Top-level UI paths:

- Dashboard: `https://dev.everruns.com/dashboard`
- Sessions list: `https://dev.everruns.com/sessions`
- Session chat: `https://dev.everruns.com/sessions/{session_id}/chat`
- Session events/files/resources/schedules/storage:
  `https://dev.everruns.com/sessions/{session_id}/{tab}`
- Agent: `https://dev.everruns.com/agents/{agent_id}`
- Harness: `https://dev.everruns.com/harnesses/{harness_id}`
- Capability: `https://dev.everruns.com/capabilities/{capability_id}`
- App: `https://dev.everruns.com/apps/{app_id}`
- MCP servers list: `https://dev.everruns.com/mcp-servers`
- Models list: `https://dev.everruns.com/models`
- Organization: `https://dev.everruns.com/orgs/{org_id}`

Examples:

- `[Support Triage](https://dev.everruns.com/agents/agent_01933b5a00007000800000000000001)`
- `[Session](https://dev.everruns.com/sessions/session_01933b5a00007000800000000000003/chat)`
- `[Base Harness](https://dev.everruns.com/harnesses/harness_01933b5a00007000800000000000002)`
- `[Web Fetch](https://dev.everruns.com/capabilities/web_fetch)`

## Names and IDs

- `id` is the stable API and UI-link identifier. Use it for follow-up calls,
  joins, and links. Core IDs are prefixed, such as `agent_...`, `harness_...`,
  `session_...`, `app_...`, and `org_...`. Capability IDs are capability refs,
  such as `web_fetch`, `session_storage`, `mcp:{uuid}`, or `skill:{id}`.
- `name` is the addressable slug for agents and harnesses. It is lowercase,
  URL-friendly, and good for search or operator-facing references, but `id` is
  safer for exact operations.
- `display_name` is the human-readable UI label for agents and harnesses.
  Render entities as `display_name || name || id`. Apps use `name`; sessions use
  `title || preview || id`; capabilities use `name || id`.
- When returning a list, include both a readable label and the stable ID. Link
  the readable label when possible.

## MCP tools exposed by Everruns(Dev)

| Tool | When to use |
|---|---|
| `me` | Current user + active org. Cheap sanity check. |
| `list_organizations`, `switch_organization` | Multi-org accounts. `switch_organization` is advisory - the MCP transport is stateless; pass `organization_id` per call to stick with a non-default org. |
| `agent_run` | Create a session and send the first message in one go. Returns `session_id`, `message_id`. |
| `session_send_message` | Follow-up user message in an existing session. |
| `session_get_status` | Poll session status + latest agent message + recent events. Supports `since_event_id` for incremental polling and `event_types` to filter. |
| `discover` | Search the API catalog. Returns operation name, category, description, parameters. |
| `query` | Run a bash script where only read-only Everruns(Dev) API operations are builtins. `jq` is available. Prefer this for inspection, search, preview, export, grep, and status checks. |
| `execute` | Run a bash script where every Everruns(Dev) API operation is a builtin. `jq` is available. Use this when the workflow needs side effects. |

Default to `query` for reads. Switch to `execute` only when the workflow needs
to create, update, delete, or trigger actions.

## Using `discover`, `query`, and `execute`

`discover` surfaces what is available, `query` runs the read-only subset, and
`execute` runs the full catalog. Always use `discover` if you are unsure which
builtin to call.

`discover` returns JSON. Focused searches include each operation's
`input_schema`, `output_schema`, and `output_shape`. Use `output_shape` before
writing jq:

- `paginated` means the root is `{data,total,offset,limit}`; iterate with
  `.data[]`.
- `array` means the root is a bare array; iterate with `.[]`.
- `unknown` means inspect `output_schema` or run a small sample first.

### Finding operations

```text
# Find anything agent-related
discover { "query": "agent" }

# List every operation grouped by category
discover { "all": true }
```

### Running operations

Inside `query` and `execute`, each available operation becomes a bash builtin.
Pass parameters as
`--name value` flags. Builtins emit JSON on stdout.

Rules of thumb:

- Pipe through `jq` to extract fields.
- Capture ids in shell variables so follow-up calls stay readable.
- Prefer `query` when a read-only path is enough.
- Prefer one scripted call that does the whole workflow over several round-trips.
- `<cmd> --help` prints usage if you are uncertain about flags.
- If a builtin is missing in `query`, it is probably mutating; switch to `execute`.

### Examples

**List agents by name.**

```bash
query { "commands": "list_agents | jq '.data[] | {id, name, default_model_id}'" }
```

**List harnesses.**

`list_harnesses` returns a bare array, not a paginated object:

```bash
query { "commands": "list_harnesses | jq '.[] | {id, name, display_name, description, status, parent_harness_id}'" }
```

**Create an agent and run it with the default harness.**

```bash
execute { "commands": "AID=$(create_agent --name \"researcher\" --system_prompt \"You are a careful researcher.\" | jq -r .id)\nagent_run --agent_id \"$AID\" --message \"Summarize https://docs.everruns.com/\"" }
```

`agent_run` uses the deployment's default harness. To pin a specific harness to
the session, create the harness, then create the session directly:

```bash
execute { "commands": "HID=$(create_harness --name \"research\" --system_prompt \"Research assistant.\" | jq -r .id)\nAID=$(create_agent --name \"researcher\" --system_prompt \"You are a careful researcher.\" | jq -r .id)\nSID=$(create_session --harness_id \"$HID\" --agent_id \"$AID\" | jq -r .id)\ncreate_message --session_id \"$SID\" --message '{\"role\":\"user\",\"content\":[{\"type\":\"text\",\"text\":\"Summarize https://docs.everruns.com/\"}]}'" }
```

**Attach a capability to an agent.**

```bash
query { "commands": "list_capabilities | jq '.data[] | {id, name}'" }
```

```bash
execute { "commands": "update_agent --id \"$AID\" --capabilities '[{\"ref\":\"web_fetch\"}]'" }
```

**Poll a session until it idles.**

```bash
query { "commands": "SID=session_abc...\nwhile true; do\n  STATUS=$(get_session --session_id \"$SID\" | jq -r .status)\n  echo \"status=$STATUS\"\n  [ \"$STATUS\" = \"idle\" ] && break\n  sleep 2\ndone\nlist_messages --session_id \"$SID\" | jq '.data[-1]'" }
```

**Iterate over agents.**

```bash
query { "commands": "list_agents | jq -r '.data[].id' | while read id; do\n  get_agent --id \"$id\" | jq '{id, name, display_name, default_model_id}'\ndone" }
```

**Add an MCP server (becomes a virtual capability).**

```bash
execute { "commands": "create_mcp_server --name \"jira\" --url \"https://mcp.example.com/jira\" --auth_mode oauth" }
```

### Raw builtin examples

```bash
list_agents | jq '.data[] | {id, name, default_model_id}'
```

### Defaults and limits

- `query` and `execute` time out after 30 s by default, max 60 s. For long
  workflows, split them into multiple calls.
- `list_*` operations default to `--limit 20` (max 100) with `--offset` for
  paging.
- Every org-scoped operation accepts `--organization_id org_{32-hex}` to target
  a non-default org in multi-org accounts.

## When things go wrong

- **401 / OAuth loop** - The host handles the OAuth 2.1 flow. If it fails,
  sign out at <https://dev.everruns.com> and retry so a fresh client registers.
- **`tool not found` in query** - `query` only exposes read-only builtins. If
  the command mutates state, switch to `execute`.
- **`tool not found` in execute** - run `discover { "all": true }` to confirm
  the builtin exists under the expected name.
- **Operation unclear** - `<cmd> --help` inside `query` or `execute` prints usage;
  `discover { "query": "<cmd>" }` shows parameter types.
- **Wrong org** - `me` reports the active org; pass `--organization_id`
  explicitly on each call to override.

## Docs

- Product docs: <https://docs.everruns.com>
- Marketing: <https://everruns.com>
- Dev deployment: <https://dev.everruns.com>
- MCP endpoint: <https://dev.everruns.com/mcp>
