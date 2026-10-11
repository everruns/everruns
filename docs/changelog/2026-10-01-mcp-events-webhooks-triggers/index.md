---
title: MCP Events add session webhooks and triggers on MCP server events
description: With the experimental MCP Events flag, MCP clients subscribe to signed session webhooks, and agents wake on events published by their own MCP servers.
date: 2026-10-01
kind: feature
significance: notable
products: [platform, cloud]
surfaces: [agent, api]
release: "0.34.0"
links:
  docs: [/features/mcp/, /features/agent-triggers/]
  prs: [everruns/everruns#3931, everruns/everruns#3970]
---

Organizations that turn on the experimental **MCP Events** flag get events in
both directions.

- An MCP client calls `events/subscribe` on `/mcp` to receive signed
  webhooks for `session.completed`, `session.failed` and
  `session.input_required`. Callbacks must be public HTTPS URLs and are
  verified before the first event.
- An agent trigger with `trigger_type: "mcp_event"` subscribes to an event one
  of the agent's own MCP servers publishes, such as a tracker announcing new
  issues, and starts a session when it arrives.

See [MCP](/features/mcp/) and [Agent triggers](/features/agent-triggers/). PRs
[#3931](https://github.com/everruns/everruns/pull/3931), [#3970](https://github.com/everruns/everruns/pull/3970).
