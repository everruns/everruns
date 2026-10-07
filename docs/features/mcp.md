---
title: Model Context Protocol (MCP)
description: Everruns is both an MCP server, exposing its agents and tools to external clients with OAuth 2.1, and an MCP client that registers remote servers as virtual capabilities.
sidebar:
  label: MCP
appliesTo: [platform, cloud]
---

Everruns speaks the [Model Context Protocol](https://spec.modelcontextprotocol.io) on **both sides**: it exposes its own agents and tools as an MCP server, and it consumes remote MCP servers as agent capabilities.

## Everruns as an MCP server

Every Everruns deployment exposes an authenticated MCP endpoint at `/mcp` so external clients, Claude Desktop, Cursor, VS Code, or another agent, can discover and call your agents and tools over JSON-RPC. It is a standard product surface, with no deployment variable or organization feature toggle.

- **Transport**: JSON-RPC 2.0 over Streamable HTTP (`POST /mcp`). Protocol versions `2026-07-28`, `2025-06-18` and `2025-03-26` are supported.
- **Methods**: `initialize`, `server/discover`, `ping`, `tools/list`, `tools/call`, `resources/list`, `resources/read`, and, for organizations that turn on MCP Events, `events/list`, `events/subscribe` and `events/unsubscribe`.
- **Auth**: MCP requests authenticate and resolve an organization before dispatch. OAuth 2.1 uses mandatory PKCE, and MCP access tokens are bound to the exact `/mcp` resource so they cannot be reused against the REST API. Unauthenticated requests fail with `401` and protected-resource discovery metadata is served at `/.well-known/oauth-protected-resource/mcp`.
- **Entity cards**: under protocol `2025-06-18`, tools like `agent_get_card` return a sandboxed `text/html` MCP App resource at the `ui://` scheme alongside a text summary, so MCP-Apps-aware hosts render rich cards while others fall back to text.

Routing is intentionally split: REST under `/api/*`, MCP OAuth under `/oauth/*`, and MCP JSON-RPC at `/mcp`.

### In ChatGPT and Codex

`/mcp` is an [MCP App](https://github.com/modelcontextprotocol/ext-apps): hosts that
render MCP Apps, including ChatGPT, Codex and Claude, show a live session view
after `agent_run`, answer an agent's questions in a form, approve or decline a
pending action, and open an Everruns panel from the sidebar. Hosts that do not
render apps get the same tools as plain text.

### Session events (experimental)

With the **MCP Events** feature flag turned on for an organization, an MCP client
can subscribe to webhooks for `session.completed`, `session.failed` and
`session.input_required`, filtered by agent or session. Callbacks must be public
HTTPS URLs and are verified before the first event. Deliveries are signed per
[Standard Webhooks](https://www.standardwebhooks.com/) and carry identifiers and
state, not message content: read the session with `session_get_status`.

## Everruns as an MCP client

People can also add MCP servers for themselves; agents with the User MCP Servers capability use them while that person chats. See [User MCP Servers](/features/user-mcp-servers/).

Register a remote MCP server and its tools appear as a **virtual capability**: auto-discovered, namespaced, and executed alongside built-in capabilities. No code changes are needed to give an agent new tools.

- **Org-managed servers**: organization-scoped `McpServer` records connect over remote HTTP (Streamable HTTP). `stdio` is rejected by the hosted control plane and is only available to single-tenant runtime/CLI hosts.
- **Scoped `mcpServers`**: harnesses, agents, and sessions can embed remote MCP config directly (the remote-server subset of `.mcp.json`) for session-local or agent-local wiring without creating an org-global record.
- **Tool naming**: discovered tools are namespaced per server so they never collide with built-in capabilities.
- **Protocol compatibility**: the client negotiates the MCP protocol era per server. By default (`auto`) it issues a session-less `2026-07-28` request and transparently falls back to the stateful `initialize` handshake (`2025-06-18` / `2025-03-26`) for servers that require it, caching the verdict per server. Set the protocol mode to `legacy`, `stable`, or `rc` to pin a specific era and skip negotiation. No setting is needed for the common case.

### Who a server acts as

Each MCP server attached to an agent says whose sign-in its calls use
(`actsAs`, or **Who should this server act as?** on the agent's **MCP servers**
sheet):

| `actsAs` | On the sheet | Calls run as |
|---|---|---|
| `none` | No identity | Nobody: the server needs no sign-in |
| `service` | Service identity | The agent's own account, signed in once by someone who manages MCP servers |
| `user` | Invoking user | The person chatting, with their own sign-in; nothing when nobody is chatting |
| `user_or_service` | Each user, or the agent if they have not connected | The person chatting when they have signed in, otherwise the agent's account |

`user_or_service` is never chosen for you. Unattended runs (schedules,
triggers) always use the agent's account, since nobody is chatting. Every MCP
tool call records which account it used in its `tool.completed` event
(`acted_as`: `user` or `service`), so a fallback to the agent is visible.

An agent with a server acting as `user` or `user_or_service` gets the
`connect_mcp_server` tool, so it can show a **Connect** card in chat before a
call fails.

### Connecting from chat

When a call needs a sign-in that is missing, the chat pauses on a **Connect**
card (or, for the agent's own account, an **Authorize** card for someone who
manages MCP servers and **Ask an admin** for everyone else). Set
`connectInChat` on the attachment (**Ask to connect in chat** on the agent's
**MCP servers** sheet) to choose:

| `connectInChat` | A missing sign-in |
|---|---|
| `ask` (default) | Pauses the chat with the card |
| `never` | Fails the call with an error naming the server and its settings link (`/settings/connections` for the person's own sign-in, the agent's **MCP servers** sheet for the agent's), and the turn goes on |

`never` suits agents behind channels that cannot show a card, such as a chat
bridge. `connect_mcp_server` follows it too and returns the same link instead
of a card.

```json
{
  "mcpServers": {
    "github": { "use": "catalog:github", "actsAs": "user", "connectInChat": "never" }
  }
}
```

A catalog entry can take the agent's account from an existing connection
instead of its own sign-in (`service_connection_provider`, **Agent credential**
in the catalog form). The seeded `github` entry
(`https://api.githubcopilot.com/mcp/`) uses the agent's GitHub App, so an agent
with a GitHub App needs no second GitHub login. A connection is only accepted
for servers on its provider's own hosts, so its tokens cannot be sent anywhere
else.

### Waking an agent on a server's events (experimental)

With the **MCP Events** flag on, an agent can also subscribe to events that one
of its own MCP servers publishes, such as a tracker announcing new issues.
Create an agent trigger with `trigger_type: "mcp_event"`, the attachment's name
in `mcp_server`, the event name from the server's `events/list` in `mcp_event`,
and any subscription arguments in `mcp_event_arguments`:

```bash
curl -X POST "$EVERRUNS_API/v1/agents/$AGENT_ID/triggers" \
  -H "Content-Type: application/json" \
  -d '{
    "trigger_type": "mcp_event",
    "mcp_server": "tracker",
    "mcp_event": "issue.created",
    "mcp_event_arguments": {"team": "eng"},
    "message": "Triage {{payload.issue.title}}"
  }'
```

Everruns subscribes with the server's own credentials for that attachment, a
signing secret it generates, and a callback URL of its own; the server must
accept webhook delivery. Each signed event starts a run with the event's `data`
as `{{payload}}`. Subscriptions are renewed before they expire and cancelled
when the trigger is disabled or deleted. The attachment must act as the agent
(`service` or `user_or_service`), not only as the calling user, since a trigger
runs as the agent.

## When a tool needs a person

Some MCP tools cannot finish without a human: a payment to authorize, an API key
to paste, a consent screen to click. Under protocol `2026-07-28` the server hands
back a URL instead of asking for the value, and Everruns holds the turn until
someone answers. See [URL mode elicitation](/features/mcp-url-elicitation/).

A server can also ask the person a few structured questions (form mode
elicitation), such as which environment to deploy to. This is off by default:
set the server's **Elicitation** setting (`elicitation_policy`) to
`url_and_form` to allow it. The questions appear in the conversation marked as
coming from that server, and the answer goes back to the server when the tool
runs again. Everruns refuses questions that ask for a password or key, and
declines on the person's behalf if nobody answers in time. `none` stops a server
eliciting at all.

## Use Everruns from your AI tools

To connect Claude Code, Codex, or Cursor to a deployment via the `everruns` plugin, see [Use in AI tools](/getting-started/use-in-ai-tools/).

## Related

- [URL mode elicitation](/features/mcp-url-elicitation/), tool calls that need a person
- [Capabilities](/features/capabilities/), how virtual capabilities fit the capability system
- [Slack Integration](/capabilities/slack/), publishing an Agent through a messaging endpoint
