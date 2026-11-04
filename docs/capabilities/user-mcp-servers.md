---
title: User MCP Servers
description: Give an agent the MCP servers of the person chatting with it, signed in as that person, and optionally let it add and connect them in chat.
sidebar:
  order: 97
appliesTo: [platform, cloud]
# The feature page shares this title; keep the <title> distinct for search results.
head:
  - tag: title
    content: "User MCP Servers Capability | Everruns"
---

| | |
|---|---|
| **ID** | `user_mcp` |
| **Category** | Integrations |
| **Features** | None |
| **Dependencies** | None |
| **Risk** | Medium |

Adds the [user MCP servers](/features/user-mcp-servers/) of the person who sent
the current message to the agent's MCP servers for that turn. Each server signs
in as that person. With `use` alone the capability adds no tools of its own; the
tools come from the person's servers. With `manage` on, the agent can also
change the person's list in chat.

## When to enable

- A personal assistant such as Platform Chat, where each person brings their own
  Linear, Notion or internal servers.
- Any agent people talk to directly and should be able to extend for themselves
  without editing the agent.

Leave it off for agents that run unattended or in shared channels: they get
nothing from it.

## Configuration

| Setting | Default | Effect |
|---|---|---|
| `use` | `true` | Add the person's enabled servers to each turn. |
| `manage` | `false` | Give the agent the tools below to change the person's list. |
| `allow_custom_urls` | `false` | With `manage`, let the agent add a server by URL, not only from the organization's MCP catalog. |

```json
{ "ref": "user_mcp", "config": { "use": true, "manage": true } }
```

## Tools

Added only when `manage` is on. They act on the list of the person who sent
the message, the same list as **Settings > My MCP servers**, and never on the
organization catalog. Read-only discovery can list approved organization presets
before the agent chooses one to add.

| Tool | What it does | Shared approval |
|---|---|---|
| `list_user_mcp_servers` | Lists the person's servers and those added to this conversation only: enabled, signed in, and whether this agent skips one because its own server has the same name | No |
| `list_mcp_catalog` | Lists approved organization presets with their exact catalog names, sanitized endpoint URLs, descriptions and authentication modes. Follow `next_cursor` until null, including after an empty page | No |
| `add_user_mcp_server` | Adds a catalog server (`{"catalog": "linear"}`), or a server by URL when `allow_custom_urls` is on. With `"scope": "chat"` it is added to this conversation only | In normal/protective mode |
| `remove_user_mcp_server` | Removes a server, from this conversation or from the list | In protective mode |
| `enable_user_mcp_server` | Turns a server on | In normal/protective mode |
| `disable_user_mcp_server` | Turns a server off without removing it | In protective mode |
| `connect_mcp_server` | Shows the person a Connect card for a server that needs a sign-in | No |

Before adding a catalog server, use `list_mcp_catalog` and pass a returned name
as `catalog` to `add_user_mcp_server`. Catalog names are chosen by the organization;
they need not match the provider's name. Discovery returns bounded pages of active
presets with short descriptions, never credentials or headers, and does not contact servers or begin OAuth.
It has the same verified-person and single-participant restrictions as management.

`add_user_mcp_server` and `enable_user_mcp_server` declare that they require
approval through the agent's shared [Tool Approval](/capabilities/tool-approval/)
policy. With `tool_approval` enabled in `normal` or `protective` mode, they wait
for approval in chat. With the capability absent or its mode set to `off`,
they run without a hard approval gate. `user_mcp` adds no separate gate or
approval setting. Other management tools follow the same mode and their tool
hints; `remove` and `disable` only take tools away.

OAuth sign-in remains a separate authorization step, regardless of the tool
approval mode.

A server added or enabled in a turn is usable from the person's next message.

### This conversation only

`add_user_mcp_server` with `"scope": "chat"` adds the server to the current
conversation instead of the person's list. It joins every later turn of this
conversation, loads on demand like the person's other servers, and never
appears in **Settings > My MCP servers** or in their other conversations.
`remove_user_mcp_server` takes it away from the next message. The same rules
apply as for the list: the shared approval policy, catalog servers only unless
`allow_custom_urls` is on, and no keys or headers. A catalog server signs in as
the person, as it would from their list. A custom server that signs in with
OAuth cannot be added this way, because its sign-in needs the list entry; add
it to the list instead. A name that one of the agent's own servers, or a
resource attached through [Resource Discovery](/integrations/ard/), already
uses in this conversation is refused.

In Chat, an **MCP** button in the conversation header lists these servers, with
their host and whether you have signed in, and removes one with a click. The
same list is `GET /v1/sessions/{session_id}/mcp-servers`, and
`DELETE /v1/sessions/{session_id}/mcp-servers/{name}` removes one.
`connect_mcp_server` never sees a credential: the person signs in in their own
browser, as with any Connect card.

`connect_mcp_server` is also added on its own, without `manage` and without
the capability being configured, when one of the agent's own MCP servers acts
as `user` or `user_or_service`, so the agent can offer the sign-in before a
call fails. For a server that acts as the agent (`service`), the card leads to
the agent's **MCP servers** sheet, where only someone allowed to manage MCP
servers can authorize it; anyone else is told to ask an admin. An agent server
set to `connectInChat: never` gets no card: the tool returns the same settings
link for the agent to pass on (see
[Connecting from chat](/features/mcp/#connecting-from-chat)). Servers added in chat cannot carry API keys
or headers; the person adds those in Settings.

## Behaviour

- Only the person who sent the message counts, never the session owner or the
  agent's own account. Unattended runs get no servers.
- Sessions with more than one person get no servers.
- An agent or capability server with the same name wins; the person's server is
  skipped. `list_user_mcp_servers` and the agent's **MCP servers** sheet report
  it.
- The manage tools refuse to run without a person (unattended runs) and in
  sessions with more than one person, with a message saying so.
- A server that fails validation is skipped on its own; the rest still load.
- The person's servers load on demand by default: the agent sees one line per
  server (`mcp_<name>`) and loads a server's tools through `tool_search`, or by
  calling that line, when it needs them. They are callable from its next step
  and stay loaded for the rest of the conversation (see
  [Loading tools on demand](/features/mcp/#loading-tools-on-demand)). Turning
  off **Load tools on demand** on a server in **Settings > My MCP servers**
  (`deferred: false` when adding or updating it under
  `/v1/virtual-users/{id}/mcp-servers`)
  lists its tools at the start of every turn instead. Servers added for one
  conversation only always load on demand.

## Risk

Medium. The agent can call any tool on servers the person chose, with the
person's login. Tool approval and the agent's other guardrails apply to these
tools as to any MCP tool.

With `manage`, adding a server is how a prompt injection would try to send the
person's data somewhere new. Enable Tool Approval in `normal` or `protective`
mode to hold add and enable calls for the person's approval. Leaving approval
absent or off allows these calls to run autonomously. Custom URLs remain off
by default, and every URL goes through the same address checks as the API.
