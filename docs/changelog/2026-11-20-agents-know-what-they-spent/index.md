---
title: "Agents know what their session cost, in dollars"
description: "get_session_info now returns the session's spend: model calls, background calls, images, sandbox time and subagents, priced as the Cost tab prices them."
date: 2026-11-20
kind: feature
significance: notable
products: [platform, cloud]
surfaces: [api]
links:
  docs: [/capabilities/session/]
---

An agent asked to stay under $5, or to say what a pull request cost, can now
read the answer instead of guessing from tokens. `get_session_info` returns
`spend`:

```json
{
  "total_usd": 0.47,
  "basis": "list_price",
  "current_turn_usd": 0.06,
  "wasted_usd": 0.02,
  "models": {
    "usd": 0.42,
    "calls": 31
  },
  "sandboxes": [
    {
      "provider": "daytona",
      "running_seconds": 360,
      "usd": 0.007
    }
  ],
  "subagents": {
    "sessions": 1,
    "usd": 0.04
  }
}
```

OpenRouter calls carry the cost the provider charged. Sandboxes are priced
by reserved CPU, memory and disk at Daytona's, E2B's or Modal's published
rate, or by your own `sandbox.time` rate. The figure covers only that session
and its subagents, never org totals or budgets. See
[Session](/capabilities/session/).
