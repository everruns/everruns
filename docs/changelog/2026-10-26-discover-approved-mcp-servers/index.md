---
title: "Agents can discover approved MCP servers"
description: "Agents can list approved organization MCP presets before adding a personal or chat-only server, using exact names without exposing credentials."
date: 2026-10-26
cloud: 2026-10-26
kind: fix
significance: notable
products: [platform, cloud]
release: "0.50.1"
surfaces: [agent, code]
links:
  docs: [/capabilities/user-mcp-servers/]
---

Agents with [User MCP Servers management](/capabilities/user-mcp-servers/) can
now call `list_mcp_catalog` before adding a server. The tool returns approved
organization presets with their exact catalog names, descriptions, authentication
modes and endpoint URLs with credentials removed.

Previously, an agent could list personal connections but had to guess the
organization's preset name. A valid server could therefore appear unavailable.
The agent can now select a returned name and add it:

```json
{
  "catalog": "approved-docs"
}
```

Discovery follows the same verified-person restrictions as management. It does
not add servers, contact them, expose headers or start OAuth. Adding and enabling
servers still follow the agent's configured approval policy.
