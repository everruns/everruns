---
title: "Tool refreshes are distinguished from history compaction"
description: "Provider-owned request projections exclude unused tool declarations, and tool-only refresh markers explain that conversation messages were retained."
date: 2026-11-25
kind: fix
significance: minor
products: [platform, framework, cloud]
surfaces: [agent, ui]
links:
  docs: [/advanced/compaction/]
---

Compaction budgeting now follows the declarations and prompt each driver sends.
OpenRouter budgets current tools; verified native inline routes include their
baseline and changes. Custom Framework drivers keep conservative estimates until
they opt in.

Tool-only refresh markers say **Tool definitions refreshed**, making clear that
conversation messages were retained. This event records a refresh, not a saved summary:

```json
{
  "strategy_used": "tool_rebase",
  "checkpoint_id": null,
  "messages_before": 40,
  "messages_after": 40
}
```

Inline prompt updates count once, and epoch resets survive worker restarts.
Runtime fallback diagnostics remain in traces instead of provider options.
Only verified complete, nonempty, tool-free summaries can replace history;
unknown, paused and truncated endings are rejected. Incomplete Responses remain
incomplete even when their reason is missing or contradictory.

See [Compaction](/advanced/compaction/) for request budgets and durable continuity.
