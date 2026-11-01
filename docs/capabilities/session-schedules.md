---
title: Schedules
description: Schedule one-shot and recurring cron-based tasks within a session.
appliesTo: [platform, cloud]
---

| | |
|---|---|
| **ID** | `session_schedule` |
| **Category** | Core |
| **Features** | `schedules` (Files and Work tabs list registered, canceled, and deleted schedules) |
| **Dependencies** | None |

Schedule future tasks within the current session. Supports one-shot (run once at a specific time) and recurring (cron expression) schedules.

Recurring cron is interpreted in the schedule's IANA timezone. The next trigger
must be in the future in UTC, including when daylight-saving time repeats a
local hour; an already-past occurrence cannot make the schedule immediately due
again.

## Tools

### `create_schedule`

Create a new scheduled task.

| Parameter | Type | Required | Description |
|---|---|---|---|
| `description` | string | yes | The message/task to execute |
| `scheduled_at` | string | conditional | ISO 8601 datetime for one-shot schedules |
| `cron_expression` | string | conditional | Cron expression for recurring schedules |
| `timezone` | string | no | IANA timezone the cron is read in. Default UTC |
| `ends_at` | string | recurring | RFC 3339 time after which the schedule stops, at most 7 days ahead |
| `max_runs` | integer | recurring | Most times the schedule fires, at most 96 |
| `stop_when` | string | recurring | One sentence on when the work is done |
| `budget_usd` | number | no | Spend cap for the schedule's runs. Default $2, at most $10 |

Provide either `scheduled_at` or `cron_expression`, not both. A recurring
schedule needs `stop_when` and at least one of `ends_at` and `max_runs`;
whichever end comes first stops it.

### `cancel_schedule`

Cancel an active schedule.

| Parameter | Type | Required | Description |
|---|---|---|---|
| `schedule_id` | string | yes | ID of the schedule to cancel |

### `list_schedules`

List all schedules for the current session.

## Recurring background tasks

Recurrence is built on schedules, there is no separate "recurring task" object to
configure. A recurring (`cron_expression`) schedule either delivers a scheduled
turn to the session, or, when paired with a background **monitor** task, runs a
probe on each fire and records the result on the task's thread. This composition
(recurring schedule + monitor) is the supported way to run periodic background
work, there is no separate recurring-task primitive to configure.

To poll a URL without spending model tokens, an agent calls `spawn_background`
with `web_fetch`, a `schedule` and a `schedule.wake_when` condition. Each fire
fetches the URL with the session's network rules and starts no turn; the agent
gets one turn, with the response, when the condition becomes true. The
condition reads the result, with a JSON body parsed under `/json`:

```json
{
  "tool": "web_fetch",
  "args": {
    "url": "https://claims.example/token",
    "method": "POST",
    "json": {
      "claim": "clm_1"
    }
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

Use `equals` to wait for a value, `not_equals` to wait for one to end, or
`"changed": true` to hear about every new value.

## Limits on unattended runs

A schedule runs with no person watching, so it always has an end and a
budget:

- An agent's recurring schedule lasts at most 7 days and 96 runs. At the
  5 minute minimum interval 96 runs is 8 hours, so a longer wait needs a longer
  interval. When it reaches its end it stops, with the reason `ended`.
- Each schedule's runs may spend $2 unless the agent passes `budget_usd`, up to
  $10. When that is spent the schedule stops, with the reason
  `budget_exhausted`. Turns a person starts in the same session do not count.
- All unattended work in an organization, schedule runs and sessions an
  [agent trigger](/features/agent-triggers/) started, may spend $20 per day.
  When that is spent, schedules skip their runs until the next day.
- Every run's message tells the agent which run it is, when the schedule ends,
  its stop condition and what it has spent, so it can cancel the schedule once
  the work is done.
- A schedule whose runs keep ending with the same answer is achieving nothing.
  After 24 identical runs in a row, each run's message warns the agent; after
  48 the schedule stops, with the reason `runaway`. Resuming it starts the
  count over.
- **Settings → Health** reports schedules that repeat or hit their budget, the
  organization's daily cap, a jump in an agent's unattended spend, and sessions
  that send a large, mostly uncached history. The schedule's owner and the
  agent's authors are notified; organization owners hear about the daily cap.
- Organization admins change these limits in **Settings → Limits**. A person
  can extend one schedule's end, runs or budget, or resume it, from the
  session's schedules, without the agent limits.

## Notes

- Maximum 5 active schedules per session
- Cron uses standard 5-field format (minute, hour, day, month, weekday), with
  weekdays counted 0-6 from Sunday (7 is also Sunday, so `5-7` is Friday through
  Sunday and `0 18 * * 0` is Sundays at 18:00), read in the schedule's timezone
- A recurring schedule fires at most once every 5 minutes by default. Admission
  rejects invalid cron and expressions with fewer than two future occurrences,
  because their minimum interval cannot be verified. Use a one-shot schedule for
  a single future run
- An invalid cron expression or timezone, or a schedule over a limit, fails the
  tool call with the reason, so the agent can correct it
- Scheduled messages are sent to the session as if the user sent them
- Deleting a schedule keeps it on the session. The Files and Work tabs show it
  as deleted, with when it was registered and when it was removed. It does not
  fire again, and `list_schedules` omits it. `cancel_schedule` disables a
  schedule without deleting it
- Use [Current Time](/capabilities/current-time/) to determine "now" before scheduling

## See Also

- [Current Time](/capabilities/current-time/), get current time for scheduling context
- [Session](/capabilities/session/), session metadata
- [Capabilities Overview](/capabilities/)
