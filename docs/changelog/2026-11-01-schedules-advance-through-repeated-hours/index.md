---
title: Schedules advance during repeated daylight-saving hours
description: Recurring session schedules ignore past UTC candidates during repeated daylight-saving hours, preventing immediate repeated executions.
date: 2026-11-01
kind: fix
significance: notable
products: [platform, cloud]
cloud: 2026-11-01
surfaces: [agent, api]
links:
  docs: [/capabilities/session-schedules/]
---

Recurring session schedules now discard cron candidates that are already in
the past. When daylight-saving time repeats a local hour, the next trigger
advances in UTC instead of making the schedule immediately due again. Existing
timezone settings continue to determine the cron's local calendar fields.

For example, a bounded five-minute schedule can keep its IANA timezone:

```json
{
  "description": "Check the task status",
  "cron_expression": "*/5 * * * *",
  "timezone": "America/Chicago",
  "max_runs": 2,
  "stop_when": "The task is complete"
}
```

See [Schedules](/capabilities/session-schedules/) for creation and cancellation.
