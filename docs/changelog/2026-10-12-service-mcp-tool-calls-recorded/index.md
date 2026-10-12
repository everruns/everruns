---
title: "Agent-login MCP tool calls now show in chats people start"
description: "Tool calls through an MCP server acting as the agent now reach sessions people start, including the connect prompt shown before an admin signs in."
date: 2026-10-12
kind: fix
significance: minor
products: [platform, cloud]
surfaces: [ui, api]
links:
  docs: [/integrations/mcp-credentials/]
  prs: [everruns/everruns#4523, everruns/everruns#PRNUM]
---

Before this fix, a tool call through an MCP server set to `actsAs: service` could
vanish from a chat, API or MCP session a person started: the call ran, but the
server refused to store its `tool.started` and `tool.completed` events, so the
transcript never showed it.

The event now records the agent's own identity as the actor and the person who
asked as the initiator. When no admin has signed the agent in yet, the call
still shows up with its connection-required result and no actor, so the prompt
to authorize the agent reaches the session. See
[MCP credentials](/integrations/mcp-credentials/).
