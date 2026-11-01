---
title: "Agent schedules get an end, a budget, a daily cap and health alerts"
description: "Agent-created recurring schedules must say when they stop and carry a spend cap, unattended work has a daily limit, and repeating schedules are stopped."
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

**Health alerts:** a schedule whose runs end the same way 24 times in a row
warns the agent, and stops after 48. **Settings → Health** reports repeating
and budget-stopped schedules, the daily cap, jumps in an agent's unattended
spend, and sessions sending a large, uncached history, and notifies the
schedule's owner and the agent's authors.
