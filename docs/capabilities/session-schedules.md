---
title: Schedules
description: Schedule one-shot and recurring cron-based tasks within a session.
appliesTo: [platform, cloud]
---

| | |
|---|---|
| **ID** | `session_schedule` |
| **Category** | Core |
| **Features** | `schedules` (enables Schedules tab) |
| **Dependencies** | None |

Schedule future tasks within the current session. Supports one-shot (run once at a specific time) and recurring (cron expression) schedules.

## Tools

### `create_schedule`

Create a new scheduled task.

| Parameter | Type | Required | Description |
|---|---|---|---|
| `message` | string | yes | The message/task to execute |
| `scheduled_at` | string | conditional | ISO 8601 datetime for one-shot schedules |
| `cron_expression` | string | conditional | Cron expression for recurring schedules |

Provide either `scheduled_at` or `cron_expression`, not both.

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

## Notes

- Maximum 5 active schedules per session
- Cron uses standard 5-field format (minute, hour, day, month, weekday), with
  weekdays counted 0-6 from Sunday (7 is also Sunday)
- A recurring schedule fires at most once every 5 minutes by default. Admission
  rejects invalid cron and expressions with fewer than two future occurrences,
  because their minimum interval cannot be verified. Use a one-shot schedule for
  a single future run
- An invalid cron expression or timezone, or a schedule over a limit, fails the
  tool call with the reason, so the agent can correct it
- Scheduled messages are sent to the session as if the user sent them
- Use [Current Time](/capabilities/current-time/) to determine "now" before scheduling

## See Also

- [Current Time](/capabilities/current-time/), get current time for scheduling context
- [Session](/capabilities/session/), session metadata
- [Capabilities Overview](/capabilities/)
