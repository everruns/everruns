---
title: Plugins
description: Give an agent the plugins your organization approved, all of them or a pick, and let it load another one when a person asks for it.
sidebar:
  order: 97
appliesTo: [platform, cloud]
head:
  - tag: title
    content: "Plugins Capability | Everruns"
---

| | |
|---|---|
| **ID** | `plugins` |
| **Category** | Integrations |
| **Features** | None |
| **Dependencies** | None |
| **Risk** | Medium |

A plugin is a package of skills and MCP servers in the
[Agent Plugins](https://agent-plugins.org/specification) format. An admin
approves plugins in **Settings > Organization > Plugins**; each approved plugin
becomes a capability with the ref `plugin:<id>`.

An agent takes plugins in two ways:

- **Preloaded.** On the agent's **Plugins** setting, choose *All approved*
  (stored as the ref `plugin:_all`, so a plugin approved later joins without
  editing the agent) or pick plugins one by one.
- **On request.** Turn on *Load others when asked*, which adds this
  capability. A person can then say "set up Visti" and the agent finds the
  approved plugin and loads it into that chat.

```json
[
  { "ref": "plugin:_all" },
  { "ref": "plugins" }
]
```

## When to enable

- A general assistant where people need different tools on different days.
- A Work Chat where the agent should pick up an approved plugin without its
  owner editing the agent first.

## Tools

| Tool | What it does |
|---|---|
| `search_plugins` | Lists approved plugins that match, whether each fits this agent and is already loaded, and directory plugins nobody approved yet |
| `load_plugin` | Loads an approved plugin into this chat: its skills at once, its MCP tools from the next turn |

## Behaviour

- A plugin declares what it needs from the agent: chat only, files, a shell or
  a sandbox. A plugin the agent cannot run is skipped when preloaded and
  refused when loaded on request.
- The agent never approves a plugin. One nobody approved is named so the
  person can ask an admin.
- Loading changes only that chat. The agent and its other conversations are
  untouched.
- In Work Chat, the channel can let people add approved plugins to their own
  conversations, or turn off one the agent includes, from **Customizations**.

## Risk

Medium. A loaded plugin's MCP servers can act with the sign-in an admin chose
for them, and its skills shape what the agent does. Only plugins an admin
approved can load, and Tool Approval applies to their tools as to any MCP
tool.
