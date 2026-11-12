---
title: Yolop speaks Agent Client Protocol v2 for Zed's Delta
description: yolop --acp now serves the draft Agent Client Protocol v2 beside v1, the version Zed's Delta requires of external agents.
date: 2026-11-12
kind: feature
significance: notable
products: [yolop]
surfaces: [cli, agent]
links:
  prs: [everruns/yolop#941]
---

`yolop --acp` now answers clients that ask for the draft Agent Client Protocol v2 and keeps serving v1 to everyone else. The client's `initialize` request picks the version, so v2-only editors such as Zed's Delta can host Yolop, while Paseo, Zed and Buzz see no change.

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

Under v2 a prompt is acknowledged with a message id once accepted, and the session reports `running`, `requires_action` while an approval waits, and `idle` with the stop reason, including for background completions that wake the agent between prompts. Sessions can be listed, resumed with or without history, and closed. The approval level is also a `mode` config option in both versions. ACP v2 is a draft, so its wire format may still change.
