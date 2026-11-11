---
title: cancel_task returns the task after cancellation
description: The cancel_task tool now returns the task's state after the cancel request plus its recent messages, the same shape as get_task, with no follow-up read.
date: 2026-11-09
kind: fix
significance: minor
products: [framework, platform, cloud]
release: "0.52.0"
surfaces: [agent]
---

The `cancel_task` tool returned the task as it was before the cancel request
reached its executor, so a task that stopped at once still read `running`.
Hosts worked around it by calling `get_task` right after every cancel.

The result now reflects the task after the executor's wind-down and adds the
same `task` snapshot and recent `messages` tail that `get_task` returns. The
existing fields stay, and `state` now reports the task's current state:

```json
{
  "task_id": "task_01",
  "state": "canceled",
  "cancel_requested": true,
  "executor": "cancellation requested",
  "task": {
    "id": "task_01",
    "state": "canceled",
    "summary": "Stopped after 2 of 5 steps"
  },
  "messages": [
    {
      "direction": "outbound",
      "content": [
        {
          "type": "text",
          "text": "Stopping"
        }
      ]
    }
  ]
}
```

A task whose remote peer is still winding down reads `running` until it
finishes. See [Sub Agents](/capabilities/sub-agents/#managing-subagents-after-spawn).
