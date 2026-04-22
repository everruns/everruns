---
name: everruns
description: Reference for working with the Everruns agent platform (https://everruns.com) over MCP - core concepts, how to connect capabilities to agents and harnesses, and how to drive the API with the `discover`, `query`, and `execute` tools. Use whenever the user asks about Everruns agents, harnesses, capabilities, sessions, models, or wants to call the Everruns API.
---

# Everruns

Everruns is a durable agentic harness engine. Docs:
<https://docs.everruns.com>. This plugin talks to Everruns over MCP and exposes
read-only API operations as bash builtins inside `query` and the full API
surface inside `execute`.

If a user asks something Everruns-related, prefer the MCP tools over guessing.
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
- **Model** - an LLM like `gpt-4o` or `claude-sonnet-4`. Resolution priority:
  per-message controls -> session override -> agent default -> system default.
- **MCP server** - a remote server exposing tools; integrated into Everruns as
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
   `create_session --harness_id ...` and then call `send_message`.

Capabilities are the answer to "how does my agent get tool X?" - attach the
capability at the harness level for org-wide defaults, at the agent level for
agent-specific tools, or at the session level for one-off additions.

## MCP tools exposed by Everruns

| Tool | When to use |
|---|---|
| `me` | Current user + active org. Cheap sanity check. |
| `list_organizations`, `switch_organization` | Multi-org accounts. `switch_organization` is advisory - the MCP transport is stateless; pass `organization_id` per call to stick with a non-default org. |
| `agent_run` | Create a session and send the first message in one go. Returns `session_id`, `message_id`. |
| `session_send_message` | Follow-up user message in an existing session. |
| `session_get_status` | Poll session status + latest agent message + recent events. Supports `since_event_id` for incremental polling and `event_types` to filter. |
| `discover` | Search the API catalog. Returns operation name, category, description, parameters. |
| `query` | Run a bash script where only read-only Everruns API operations are builtins. `jq` is available. Prefer this for inspection, search, preview, export, grep, and status checks. |
| `execute` | Run a bash script where every Everruns API operation is a builtin. `jq` is available. Use this when the workflow needs side effects. |

Default to `query` for reads. Switch to `execute` only when the workflow needs
to create, update, delete, or trigger actions.

## Using `discover`, `query`, and `execute`

`discover` surfaces what is available, `query` runs the read-only subset, and
`execute` runs the full catalog. Always use `discover` if you are unsure which
builtin to call.

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

**Create a harness and an agent, then run the agent.**

```bash
execute { "commands": "HID=$(create_harness --name \"research\" --system_prompt \"Research assistant.\" | jq -r .id)\nAID=$(create_agent --name \"researcher\" --system_prompt \"You are a careful researcher.\" | jq -r .id)\nagent_run --agent_id \"$AID\" --message \"Summarize the latest MCP spec.\"" }
```

`agent_run` uses the default harness. To pin `$HID` (or any specific harness)
to the session, create the session directly:

```bash
execute { "commands": "SID=$(create_session --harness_id \"$HID\" --agent_id \"$AID\" | jq -r .id)\ncreate_message --session_id \"$SID\" --message '{\"role\":\"user\",\"content\":[{\"type\":\"text\",\"text\":\"Summarize the latest MCP spec.\"}]}'" }
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
query { "commands": "list_agents | jq -r '.data[].id' | while read id; do\n  get_agent --id \"$id\" | jq '{id, name, harness_id}'\ndone" }
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
  sign out at <https://everruns.com> and retry so a fresh client registers.
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
- MCP endpoint spec: see `specs/mcp.md` in the main repo
