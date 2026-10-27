---
title: Platform Chat uses native functions for platform operations
description: Platform Chat discovers typed platform operations on demand and uses its shell to aggregate results across many resources.
date: 2026-10-27
kind: improvement
significance: notable
products: [platform, cloud]
surfaces: [agent]
links:
  docs: [/capabilities/platform/]
---

Platform Chat now calls individual platform operations through typed native
functions. Bounded `tool_search` reveals schemas as needed, while the Bashkit
shell remains available for loops, filtering, and aggregation. Attach the
Platform capability with its native surface and bounded search:

```json
{
  "capabilities": [
    {
      "ref": "platform",
      "config": {
        "surface": "native"
      }
    },
    {
      "ref": "tool_search",
      "config": {
        "bounded": true,
        "threshold": 1,
        "max_active_tools": 32,
        "max_active_bytes": 65536,
        "max_result_bytes": 32768
      }
    }
  ]
}
```

Search also matches declared capability names and descriptions, alongside
native operation names, so Platform Chat can narrow from a capability to an
operation without loading unrelated schemas.

Native calls use the same authorized dispatch as the shell, and current feature
flags and permissions are checked when each call runs. See [the Platform
capability reference](/capabilities/platform/).
