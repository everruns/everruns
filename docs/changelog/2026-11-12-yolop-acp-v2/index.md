---
title: Yolop speaks Agent Client Protocol v2 for Zed's Delta
description: yolop --acp now serves the draft Agent Client Protocol v2 beside v1, the version Zed's Delta requires of external agents.
date: 2026-11-12
kind: feature
significance: notable
products: [yolop]
surfaces: [cli, agent]
links:
  prs: [everruns/yolop#941, everruns/yolop#942]
---

`yolop --acp` now serves the draft Agent Client Protocol v2 to clients that ask for it and v1 to everyone else, so v2-only editors such as Zed's Delta can host Yolop.

```json
{
  "jsonrpc": "2.0",
  "id": 0,
  "method": "initialize",
  "params": {
    "protocolVersion": 2,
    "info": {
      "name": "my-editor",
      "version": "1.0"
    },
    "capabilities": {}
  }
}
```

Under v2 a prompt is acknowledged once accepted, and the session reports `running`, `requires_action` and `idle` with its stop reason and token usage, including for background work that wakes the agent. Shell commands appear as live terminals the editor can stop without ending the turn, edits as structured diffs, and running background tasks are listed on every state change. Sessions can be listed, resumed, closed and deleted, can include extra workspace folders, and a client can append to or replace the system prompt. ACP v2 is a draft, so its wire format may still change.
