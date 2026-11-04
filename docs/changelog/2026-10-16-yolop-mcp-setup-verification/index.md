---
title: "MCP setup verifies login without restarting"
description: "Yolop verifies MCP connections after browser sign-in, reports pending authorization honestly, and loads newly installed tools in the same session."
date: 2026-10-16
kind: fix
significance: notable
products: [yolop]
surfaces: [cli, agent]
links:
  docs: ["https://github.com/everruns/yolop#mcp-servers"]
  prs: [everruns/yolop#857]
---

MCP setup now distinguishes saved configuration, pending browser authorization,
and a working connection. Requests such as Cloudflare's published setup prompt
continue through sign-in and verification. Newly installed servers become
available in the same session without restarting.

```sh
yolop mcp test cloudflare
```

The command lists the server's tools or exits with an authentication or transport
error. OAuth still requires your consent in the browser; saving a server does
not imply that sign-in succeeded. See the [MCP server guide](https://github.com/everruns/yolop#mcp-servers)
for supported transports and configuration scopes.
