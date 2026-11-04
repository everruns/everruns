---
title: "Agents can poll a URL without spending model tokens"
description: "A scheduled web_fetch monitor with a wake condition fetches on every run without an agent turn, and wakes the agent once when the response changes."
date: 2026-11-01
kind: feature
significance: notable
products: [platform, cloud]
surfaces: [api]
links:
  docs: [/capabilities/session-schedules/]
---

**For agents:** `spawn_background` with `web_fetch`, a `schedule` and a
`schedule.wake_when` condition creates a probe monitor. Each run fetches the URL
with the session's network rules and starts no turn, so waiting costs no model
tokens. The agent gets one turn, with the response, when the condition becomes
true.

```json
{
  "tool": "web_fetch",
  "args": {
    "url": "https://claims.example/token",
    "method": "POST"
  },
  "schedule": {
    "cron_expression": "*/5 * * * *",
    "max_runs": 96,
    "stop_when": "the claim is approved or has expired",
    "wake_when": {
      "path": "/json/error",
      "not_equals": "authorization_pending"
    }
  }
}
```

The condition reads the response, with a JSON body under `/json`, and takes
`equals`, `not_equals` or `"changed": true`. A failed fetch wakes the agent once
with the error. See [Schedules](/capabilities/session-schedules/).
