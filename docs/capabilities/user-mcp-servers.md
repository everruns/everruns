---
title: User MCP Servers
description: Give an agent the MCP servers of the person chatting with it, signed in as that person.
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
in as that person. The capability itself adds no tools; the tools come from the
person's servers.

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

```json
{ "ref": "user_mcp", "config": { "use": true } }
```

## Behaviour

- Only the person who sent the message counts, never the session owner or the
  agent's own account. Unattended runs get no servers.
- Sessions with more than one person get no servers.
- An agent or capability server with the same name wins; the person's server is
  skipped.
- A server that fails validation is skipped on its own; the rest still load.

## Risk

Medium. The agent can call any tool on servers the person chose, with the
person's login. Tool approval and the agent's other guardrails apply to these
tools as to any MCP tool.
