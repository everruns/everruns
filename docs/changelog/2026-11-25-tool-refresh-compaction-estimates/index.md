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

Drivers now share the model/endpoint layout decision with request budgeting,
including custom Framework provider keys. Gateway and fallback routes budget
current declarations; verified native inline routes keep their baseline and
changes. Inline system-prompt updates are counted once, avoiding false pressure
from counting both the current prompt and its recorded additions. Epoch resets
persist across transcript reloads rather than restoring
obsolete declarations on every turn.

Summary completion and output-limit recovery use typed terminal outcomes. Unknown,
paused or truncated output cannot replace history. An incomplete Responses result
stays incomplete even if its reason is missing or conflicts with its status.
Runtime fallback observations stay outside provider options. Existing custom
drivers retain conservative budgeting until they opt in.

Verified first-party inline tool changes retain their existing cache behavior.
[Compaction](/advanced/compaction/) explains the distinction between request
projections and durable continuity.
