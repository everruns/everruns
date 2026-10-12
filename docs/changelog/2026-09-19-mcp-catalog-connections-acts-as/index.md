---
title: MCP catalog presets, acts-as identity and personal connections
description: MCP servers gain an org catalog of presets, an actsAs setting for whose sign-in a server uses, an agent MCP tab and a place to revoke your own sign-ins.
date: 2026-09-19
kind: feature
significance: notable
products: [platform, cloud]
surfaces: [ui, api, agent]
release: "0.30.0"
links:
  docs: [/features/mcp/, /integrations/mcp-credentials/]
  prs: [everruns/everruns#3726, everruns/everruns#3675, everruns/everruns#3638, everruns/everruns#3635]
visual: "Agent detail MCP tab listing attached servers with identity and connection state (/agents/<id>, MCP tab); crop to the list"
---

**In the UI:** the MCP servers page becomes a catalog of presets showing which
active agents use each one and what archiving would affect. The agent detail page
gains an MCP tab listing the agent's effective servers, their identity and
connection state, with flows to add a preset or custom HTTP server and to revoke a
connection. People can see and revoke the MCP sign-ins they granted.

**In agent config:** an MCP entry can reference a preset with `catalog:<name>` and
say whose sign-in its calls use with `actsAs` (`none`, `service`, `user` or
`user_or_service`):

```json
{ "github": { "use": "catalog:github", "actsAs": "user" } }
```

See [MCP](/features/mcp/) and [MCP credentials](/integrations/mcp-credentials/). [#3726](https://github.com/everruns/everruns/pull/3726), [#3675](https://github.com/everruns/everruns/pull/3675), [#3638](https://github.com/everruns/everruns/pull/3638)
