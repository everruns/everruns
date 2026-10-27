---
title: Platform Chat discovers native operations as needed
description: Platform Chat searches platform commands and MCP tools with capability context, reveals bounded native schemas, and keeps its shell for bulk work.
date: 2026-10-27
kind: improvement
significance: minor
products: [platform, cloud, framework]
surfaces: [agent]
links:
  docs: [/capabilities/platform/, /capabilities/tool-search/]
---

Platform Chat now searches the public platform command catalog and attached MCP
tools through one bounded discovery path. Search includes capability IDs, names
and descriptions alongside MCP server context. Selected operations become native
functions with structured JSON arguments; the shell remains available for bulk
work.

Other agents can enable the same [Platform capability](/capabilities/platform/)
and bounded [Tool Search](/capabilities/tool-search/):

```json
{
  "capabilities": [
    {
      "ref": "platform"
    },
    {
      "ref": "tool_search",
      "config": {
        "bounded": true
      }
    }
  ]
}
```

Every call retains current organization, caller permissions and change-history
checks. Capability-only matches ask for refinement; oversized schemas retain
bounded operation names when space permits.
