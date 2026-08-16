---
title: Schedules
description: Schedule one-shot and recurring cron-based tasks within a session. Agents can set timers, run periodic checks, and automate follow-up actions on a schedule.
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
- Cron uses standard 5-field format (minute, hour, day, month, weekday)
- Scheduled messages are sent to the session as if the user sent them
- Use [Current Time](/capabilities/current-time/) to determine "now" before scheduling

## See Also

- [Current Time](/capabilities/current-time/), get current time for scheduling context
- [Session](/capabilities/session/), session metadata
- [Capabilities Overview](/capabilities/)
