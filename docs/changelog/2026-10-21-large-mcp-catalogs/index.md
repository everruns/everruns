---
title: Large MCP catalogs use bounded search and schemas
description: MCP catalogs load across pages, search server and schema text within current access, and reveal bounded tool schemas through native calls, CLI, or both.
date: 2026-10-21
kind: feature
significance: major
products: [platform, framework, cloud]
release: "0.49.0"
surfaces: [agent]
links:
  docs: [/features/large-mcp-catalogs/, /capabilities/tool-search/, /capabilities/tools-in-shell/]
---

Everruns discovers every MCP catalog page and searches server/tool descriptions
and parameter schemas within current access. Bounded `tool_search` reveals selected schemas as the
agent needs them. Run selected operations through native tools, the session
shell, or both:

```json
{
  "capabilities": [
    {
      "ref": "tool_search",
      "config": {
        "bounded": true
      }
    },
    {
      "ref": "tools_in_shell",
      "config": {
        "mode": "hybrid"
      }
    }
  ]
}
```

Search and invocation both check current authorization, so a disclosed schema
does not preserve access after a server is removed or its connection is
revoked. CLI search loads at most eight matching deferred servers per query;
other servers stay pending, and search, help, and calls remain within the
current turn's visible scope. Root/server help stays within 64 entries and
12 KiB with explicit omission guidance. Exact operation searches avoid padding
results with incidental matches. See [Large MCP catalogs and tool surfaces](/features/large-mcp-catalogs/),
[Tool Search](/capabilities/tool-search/), and [Tools in Shell](/capabilities/tools-in-shell/).

## Update 2026-10-24

Verbose schemas no longer bury name matches. Exact names also accept spaces,
hyphens or underscores. Byte-omitted candidates include names for exact refinement.
