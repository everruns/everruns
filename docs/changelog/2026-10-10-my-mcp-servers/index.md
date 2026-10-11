---
title: "Add your own MCP servers and use them in chat"
description: "People add personal MCP servers in Settings, and agents with the User MCP Servers capability use them, including servers connected in the middle of a chat."
date: 2026-10-10
kind: feature
significance: major
products: [platform, cloud]
surfaces: [ui, api, agent]
links:
  docs: [/features/user-mcp-servers/, /capabilities/user-mcp-servers/]
  prs: [everruns/everruns#4295, everruns/everruns#4315, everruns/everruns#4343, everruns/everruns#4345, everruns/everruns#4381, everruns/everruns#4383, everruns/everruns#4496]
visual: "Settings \u2192 My agent experience \u2192 My MCP servers list with the Add server dialog (/settings/agent-experience), cropped to the list"
---

**In the UI:** open **Settings > My agent experience > My MCP servers** and
choose **Add server**, from the organization catalog or by URL. Signing in to a
catalog server as yourself also adds it to this list. In Chat, the **MCP**
button lists a conversation's chat-only servers.

**For agents:** with the **User MCP Servers** capability (`user_mcp`), each turn
adds the enabled servers of the person chatting. Platform Chat has it on, with
`manage` so it can add and connect servers mid-conversation after approval.
Personal servers load their tools on demand unless you turn off **Load tools on
demand**.

See [User MCP servers](/features/user-mcp-servers/), the
[capability](/capabilities/user-mcp-servers/) and [#4295](https://github.com/everruns/everruns/pull/4295), [#4315](https://github.com/everruns/everruns/pull/4315), [#4345](https://github.com/everruns/everruns/pull/4345), [#4381](https://github.com/everruns/everruns/pull/4381), [#4496](https://github.com/everruns/everruns/pull/4496).
