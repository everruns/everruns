---
title: Scheduled and triggered messages no longer look like yours
description: Messages written by schedules, triggers, thread updates and notices are marked with who wrote them in Chat, Work Chat, the session trace and the API.
date: 2026-11-01
kind: improvement
significance: minor
products: [platform, cloud]
surfaces: [ui, api]
links:
  docs: [/features/ui/#sessions, /features/agent-triggers/]
---

Schedules, [agent triggers](/features/agent-triggers/), channel webhooks, thread updates and cancellation
notices write messages into a session. They used to look like messages you
typed, and the session trace labeled each of their turns "user".

**In the UI:** Chat shows each one as a muted row with an icon, the schedule or
trigger name and its run counter, with the prompt folded. Work Chat shows a
single line and collapses repeated silent runs. In the session trace (see
[Sessions](/features/ui/#sessions)), the turn header names who started it,
and automatic turns get a gray background.

**In the API:** message metadata carries `everruns_origin`, and each trace turn
carries the same object as `origin`:

```json
{
  "everruns_origin": {
    "kind": "schedule",
    "source_id": "sched_01a124cd7e9571ac95c312b3c6d0cd82",
    "name": "Shipmail claim poll",
    "run": {
      "number": 12,
      "limit": 96
    }
  }
}
```
