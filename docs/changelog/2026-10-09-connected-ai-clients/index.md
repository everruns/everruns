---
title: "See and revoke AI clients connected to Everruns"
description: "Settings \u2192 My agent experience lists the AI clients you connected to Everruns over MCP OAuth, shows when each was last used, and lets you revoke one."
date: 2026-10-09
kind: feature
significance: notable
products: [platform, cloud]
surfaces: [ui, api, agent]
release: "0.47.0"
links:
  docs: [/getting-started/use-in-ai-tools/]
  prs: [everruns/everruns#4461]
visual: "Settings \u2192 My agent experience with the connected clients list and a Revoke button (/settings/agent-experience), cropped to the panel"
---

AI tools that connect to Everruns over MCP OAuth, such as a coding assistant
using the Everruns MCP server, now show up as connected clients.

**In the UI:** open **Settings → My agent experience** to see each client and
when it was last used, and revoke any you no longer use. Revoking deletes the
client's refresh tokens, and its next request is refused.

**In code:** `GET /v1/user/connected-clients` lists them and `DELETE` revokes
one. Every MCP request checks that its grant is still live.

See [Use Everruns in AI tools](/getting-started/use-in-ai-tools/) and [#4461](https://github.com/everruns/everruns/pull/4461).
