---
title: User MCP Servers
description: Give an agent the MCP servers of the person chatting with it, signed in as that person, and optionally let it add and connect them in chat.
sidebar:
  order: 97
appliesTo: [platform, cloud]
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
organization catalog.

| Tool | What it does | Asks the person first |
|---|---|---|
| `list_user_mcp_servers` | Lists the person's servers: enabled, signed in, and whether this agent skips one because its own server has the same name | No |
| `add_user_mcp_server` | Adds a catalog server (`{"catalog": "linear"}`), or a server by URL when `allow_custom_urls` is on | Yes |
| `remove_user_mcp_server` | Removes a server | No |
| `enable_user_mcp_server` | Turns a server on | Yes |
| `disable_user_mcp_server` | Turns a server off without removing it | No |
| `connect_mcp_server` | Shows the person a Connect card for a server that needs a sign-in | No |

`add_user_mcp_server` and `enable_user_mcp_server` wait for the person to
approve them in chat (the card shows the call, such as the catalog name or URL), whether or not the
agent has [Tool Approval](/capabilities/tool-approval/) on. `remove` and
`disable` only take tools away and run without asking.

A server added or enabled in a turn is usable from the person's next message.
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

## Risk

Medium. The agent can call any tool on servers the person chose, with the
person's login. Tool approval and the agent's other guardrails apply to these
tools as to any MCP tool.

With `manage`, adding a server is how a prompt injection would try to send the
person's data somewhere new. That is why adding and enabling always wait for
the person's approval, custom URLs are off by default, and every URL goes
through the same address checks as the API.
