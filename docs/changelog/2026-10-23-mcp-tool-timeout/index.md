---
title: MCP tool calls can run longer than 60 seconds
description: Each MCP server can set how long one tool call may run, from 1 second to 30 minutes, instead of the fixed 60-second limit.
date: 2026-10-23
kind: feature
significance: minor
products: [platform, framework, cloud]
release: "0.49.0"
surfaces: [agent, api, ui]
links:
  docs: [/features/mcp/]
---

Until now a call to an MCP tool failed if the server had not answered within
60 seconds. Each server can now set its own limit with `tool_timeout_ms`, from
1 second to 30 minutes, on an agent's attachment or on a catalog entry
(**Tool call timeout** in the catalog form). An attachment that uses a catalog
entry inherits the entry's limit unless it sets its own. `timeout`, the key
other `mcpServers` config files use, is also accepted:

```json
{
  "mcpServers": {
    "ci": {
      "type": "http",
      "url": "https://ci.example.com/mcp",
      "tool_timeout_ms": 600000
    }
  }
}
```

Servers without the setting keep the 60-second limit. See
[MCP](/features/mcp/).
