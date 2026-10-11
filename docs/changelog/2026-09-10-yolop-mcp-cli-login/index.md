---
title: "Manage and log in to MCP servers from the CLI"
description: "The yolop mcp command now covers show, enable, disable and an OAuth login for remote MCP servers, so MCP setup no longer needs the TUI or config files."
date: 2026-09-10
kind: feature
significance: notable
products: [yolop]
surfaces: [cli]
release: "0.18.0"
links:
  docs: ["https://github.com/everruns/yolop#mcp-servers"]
  prs: [everruns/yolop#661, everruns/yolop#677]
---

`yolop mcp` now covers MCP server configuration from a terminal: `list`, `show`,
`add`, `remove`, `enable`, `disable` and `login`. `show <name>` prints the
global, workspace or effective entry, and `disable` is its own command instead
of `enable --disable`.

`yolop mcp login <name>` runs the OAuth flow for a remote server: it prints the
authorize URL, opens the browser, waits for the callback and saves the tokens.
Before, that flow existed only as `/mcp login` inside the TUI.

```bash
yolop mcp login linear
```

See [MCP servers](https://github.com/everruns/yolop#mcp-servers), [#661](https://github.com/everruns/yolop/pull/661) and [#677](https://github.com/everruns/yolop/pull/677).
