---
title: "Agent schedules now have an end, a budget and a daily cap"
description: "Recurring schedules an agent creates must state when they stop, each schedule has its own spend cap, and unattended work has a daily limit per organization."
date: 2026-11-01
kind: improvement
significance: notable
products: [platform, cloud]
surfaces: [ui, api]
links:
  docs: [/capabilities/session-schedules/]
---

**For agents:** a recurring `create_schedule` call must pass `stop_when` and
`ends_at` or `max_runs`, up to 7 days and 96 runs. Each schedule's runs may
spend $2 unless the agent asks for more, up to $10, and every run reminds the
agent of its end, stop condition and spend.

```json
{
  "description": "Check whether the claim was approved",
  "cron_expression": "*/15 * * * *",
  "max_runs": 24,
  "stop_when": "the claim is approved, or the claim token has expired"
}
```

**In the UI:** schedule runs and trigger sessions share a $20 daily cap per
organization. Admins change every limit in **Settings → Limits**, and a
stopped schedule can be extended or resumed from the session. See
[Schedules](/capabilities/session-schedules/).
