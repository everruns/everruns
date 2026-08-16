---
name: everruns
description: Reference for working with the Everruns managed harnesses platform (https://app.everruns.com) - core concepts, UI links, entity naming, and API workflows for agents, harnesses, capabilities, sessions, models, and apps.
---

# Everruns

Everruns is the production deployment of the durable Everruns agentic harness
engine. Docs: <https://docs.everruns.com>. This plugin talks to the
Everruns deployment over MCP and exposes both focused session tools and
scriptable API access.

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
- **Model** - an LLM model id supported by the target deployment. Resolution
  priority: per-message controls -> session override -> agent default -> harness
  default -> system default. Use the API catalog or model list before naming a
  specific model.
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
   `create_session --harness_id ...` and then call `create_message`.

Capabilities are the answer to "how does my agent get tool X?" - attach the
capability at the harness level for org-wide defaults, at the agent level for
agent-specific tools, or at the session level for one-off additions.

## UI links

All user-facing outputs should include links to the relevant Everruns UI
objects when an object is created, updated, inspected, or returned as a primary
result. Use Markdown links. Default base URL: `https://app.everruns.com` unless
the user provides another deployment URL.

Top-level UI paths:

- Dashboard: `https://app.everruns.com/dashboard`
- Sessions list: `https://app.everruns.com/sessions`
- Session chat: `https://app.everruns.com/sessions/{session_id}/chat`
- Session events/files/resources/schedules/storage:
  `https://app.everruns.com/sessions/{session_id}/{tab}`
- Agent: `https://app.everruns.com/agents/{agent_id}`
- Harness: `https://app.everruns.com/harnesses/{harness_id}`
- Capability: `https://app.everruns.com/capabilities/{capability_id}`
- App: `https://app.everruns.com/apps/{app_id}`
- MCP servers list: `https://app.everruns.com/mcp-servers`
- Models list: `https://app.everruns.com/models`
- Organization: `https://app.everruns.com/orgs/{org_id}`

Examples:

- `[Support Triage](https://app.everruns.com/agents/agent_01933b5a00007000800000000000001)`
- `[Session](https://app.everruns.com/sessions/session_01933b5a00007000800000000000003/chat)`
- `[Base Harness](https://app.everruns.com/harnesses/harness_01933b5a00007000800000000000002)`
- `[Web Fetch](https://app.everruns.com/capabilities/web_fetch)`

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

## MCP tools exposed by Everruns

| Tool | When to use |
|---|---|
| `me` | Current user + default org. Cheap sanity check. |
| `list_organizations` | Multi-org accounts. MCP has no current-org switch; use this to find an org id, then pass `organization_id` on every org-scoped MCP tool call that should target that org. |
| `agent_run` | Create a session and send the first message in one go. Returns `session_id`, `message_id`. |
| `session_send_message` | Follow-up user message in an existing session. |
| `session_get_status` | Poll session status + latest agent message + recent events. Supports `since_event_id` for incremental polling and `event_types` to filter. |
| `agent_get_card` | Render an MCP-Apps card for an agent: a sandboxed `text/html` resource at `ui://everruns/agent/{agent_id}/card` plus a one-line summary. Use in MCP-UI-aware hosts (Claude Desktop, mcp-ui clients, the Everruns chat UI) to surface a stats card alongside text. Falls back gracefully, hosts that ignore embedded resources still get the summary line. Read-only; safe to call in `query`-style flows. Only available under MCP `2025-06-18`; older clients should fall back to `get_agent`. |
| `discover` | Search the API catalog. Returns operation name, category, description, parameters. |
| `query` | Run a bash script where only read-only Everruns API operations are builtins. `jq` is available. Prefer this for inspection, search, preview, export, grep, and status checks. |
| `execute` | Run a bash script where every Everruns API operation is a builtin. `jq` is available. Use this when the workflow needs side effects. |

Default to `query` for reads. Switch to `execute` only when the workflow needs
to create, update, delete, or trigger actions.

## Multi-org semantics

The MCP transport is stateless. There is no org-switching tool and no server-side
current org for MCP clients. If `organization_id` is omitted, Everruns uses
the default org reported by `me`.

For non-default orgs:

1. Call `list_organizations`.
2. Pick the target `org_{32-hex}` id.
3. Include `"organization_id": "org_..."` on each org-scoped MCP tool call:
   `agent_run`, `session_send_message`, `session_get_status`, `agent_get_card`,
   `discover`, `query`, or `execute`.

For `query` and `execute`, put `organization_id` on the MCP tool call, not inside
the bash script:

```text
query {
  "organization_id": "org_01933b5a00007000800000000000004",
  "commands": "list_agents | jq -r '.data[] | [.id, .name] | @tsv'"
}

agent_run {
  "organization_id": "org_01933b5a00007000800000000000004",
  "agent_id": "support-triage",
  "message": "Summarize the latest failed sessions."
}
```

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

**Work with app invocation channels.**

Use app-channel commands for app schedules. Do not use durable schedule
commands for app schedules; those infrastructure builtins are intentionally not
exposed through MCP scripting.

```bash
query { "commands": "list_app_channels app_01933b5a00007000800000000000001 | jq '.[] | {id, channel_type, enabled, channel_config}'" }
```

Schedule channels accept either 5-field cron (`*/10 * * * *`) or 7-field cron
(`0 */10 * * * * *`). The server stores 7-field cron.

```bash
execute { "commands": "CID=$(add_schedule_app_channel --app_id \"$APP_ID\" --cron_expression '*/10 * * * *' --timezone UTC --session_mode shared_session --message 'Run the scheduled check' | jq -r .id)\npublish_app \"$APP_ID\"\ntrigger_app_schedule_channel --app_id \"$APP_ID\" --channel_id \"$CID\"" }
```

Channel secret fields are write-only. Reads return non-secret fields and
`*_configured` booleans instead of plaintext tokens or signing secrets.

**Look up an org by id.**

`get_org` accepts the `org_...` entity id as a positional arg.

```bash
query { "commands": "get_org org_01933b5a00007000800000000000004 | jq '{id, name, default_model_id, default_harness_id}'" }
```

**Find which org owns an entity.**

`resolve_org` takes any prefixed entity id (`agent_...`, `session_...`,
`harness_...`, `app_...`, `skill_...`, `mcp:...`, `identity_...`, `eval_...`)
and returns `{org_id, org_name}` of the owning org, but only if the caller
is a member of that org. Use it to recover from a direct link into a
resource owned by a different org you also belong to.

```bash
query { "commands": "resolve_org session_019db85695a8785e87e8203109109343 | jq" }
```

**Preview an agent or harness before saving.**

`preview_agent` and `preview_harness` resolve capabilities, scoped MCP servers, and
parent harness inheritance into the final system prompt and tool list without
persisting anything. Read-only, available in `query`.

```bash
query { "commands": "preview_agent --system_prompt \"You are a careful researcher.\" --capabilities '[{\"ref\":\"web_fetch\"}]' | jq '{system_prompt, tools: [.tools[].name]}'" }
```

```bash
query { "commands": "preview_harness --parent_harness_id \"$HID\" --system_prompt \"Research harness.\" | jq '{system_prompt, tools: [.tools[].name]}'" }
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
- Org context for `query` and `execute` is selected by the top-level
  `organization_id` MCP argument. Do not pass `--organization_id` to builtins
  inside the script.

## Entity cards (MCP Apps)

Everruns exposes a small family of *entity cards*, sandboxed HTML
resources hosts can render alongside text output. See
[`knowledge/ui/mcp-cards.md`](https://github.com/everruns/everruns/blob/main/knowledge/ui/mcp-cards.md)
for the standard.

- Tool: `agent_get_card { "agent_id": "<agent-id-or-name>" }`.
  Add `"organization_id": "org_..."` when targeting a non-default org.
- Result: an MCP `resource` content item at
  `ui://everruns/agent/{agent_id}/card` (MIME `text/html`) plus a one-line
  text summary.
- The HTML is a self-contained sandboxed document with strict CSP. Hosts
  that don't understand embedded UI resources still see the summary line.
- Currently read-only; future iterations will add buttons (run, archive)
  that route through `tools/call` with host confirmation.

When a user asks for an "agent card", "agent overview", or wants stats
visible alongside a reply, prefer `agent_get_card` over `get_agent` if the
host is MCP-UI-aware. Otherwise call `get_agent` and render the JSON.

Card tools require MCP protocol `2025-06-18`. Older clients won't see them
in `tools/list` and should use `get_agent`.

## When things go wrong

- **401 / OAuth loop** - The host handles the OAuth 2.1 flow. If it fails,
  sign out at <https://app.everruns.com> and retry so a fresh client registers.
- **`tool not found` in query** - `query` only exposes read-only builtins. If
  the command mutates state, switch to `execute`.
- **`tool not found` in execute** - run `discover { "all": true }` to confirm
  the builtin exists under the expected name.
- **Operation unclear** - `<cmd> --help` inside `query` or `execute` prints usage;
  `discover { "query": "<cmd>" }` shows parameter types.
- **Wrong org** - `me` reports the default org. Call `list_organizations`, then
  pass top-level `organization_id` explicitly on each org-scoped MCP call.

## Docs

- Product docs: <https://docs.everruns.com>
- Marketing: <https://everruns.com>
- Production deployment: <https://app.everruns.com>
- MCP endpoint: <https://app.everruns.com/mcp>
