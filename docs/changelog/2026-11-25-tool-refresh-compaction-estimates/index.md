---
title: "Tool refreshes are distinguished from history compaction"
description: "OpenRouter request budgets exclude unused inline tool declarations, and tool-only refresh markers explain that conversation messages were retained."
date: 2026-11-25
kind: fix
significance: minor
products: [platform, framework, cloud]
surfaces: [agent, ui]
links:
  docs: [/advanced/compaction/]
---

OpenRouter sends the current tool declarations. Request budgeting no longer
counts historical inline tool epochs that this transport does not send. That
could repeatedly report a smaller request even though its conversation and
actual tool declarations stayed the same.

Historical tool-only markers now say **Tool definitions refreshed** and explain
that no conversation messages were removed. For example, this recorded event
represents a tool refresh rather than a saved summary:

```json
{
  "strategy_used": "tool_rebase",
  "checkpoint_id": null,
  "messages_before": 40,
  "messages_after": 40
}
```

Verified first-party inline tool changes retain their existing cache behavior.
[Compaction](/advanced/compaction/) explains the distinction between request
projections and durable continuity.
