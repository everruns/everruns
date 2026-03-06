---
title: Schedules
description: Schedule one-shot and recurring tasks within a session
---

| | |
|---|---|
| **ID** | `session_schedule` |
| **Category** | Session |
| **Features** | `schedules` (unlocks Schedules tab) |
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

## Use Cases

- **Periodic monitoring** — check system status every hour
- **Reminders** — schedule a follow-up at a specific time
- **Recurring reports** — generate daily summaries
- **Delayed actions** — schedule a task for later execution

## Example

```
User: Check the deployment status every 30 minutes

Agent:
  → create_schedule({
      message: "Check deployment status and report any issues",
      cron_expression: "*/30 * * * *"
    })
  ← { "schedule_id": "sch_01abc...", "cron_expression": "*/30 * * * *", "status": "active" }
```

## Notes

- Maximum 5 active schedules per session
- Cron uses standard 5-field format (minute, hour, day, month, weekday)
- Scheduled messages are sent to the session as if the user sent them
- Use [Current Time](/capabilities/current-time/) to determine "now" before scheduling

## See Also

- [Current Time](/capabilities/current-time/) — get current time for scheduling context
- [Session](/capabilities/session/) — session metadata
- [Capabilities Overview](/capabilities/)
