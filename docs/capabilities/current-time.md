---
title: Current Time
description: Get the current date and time in various formats and timezones
---

| | |
|---|---|
| **ID** | `current_time` |
| **Category** | Utilities |
| **Risk** | Low |
| **Features** | None |
| **Dependencies** | None |

Provides a tool to get the current date and time. Supports multiple formats and timezones.

## Tools

### `get_current_time`

Get the current date and time.

| Parameter | Type | Required | Description |
|---|---|---|---|
| `timezone` | string | no | IANA timezone (e.g., `America/New_York`, `Europe/London`) |
| `format` | string | no | Output format: `iso8601`, `unix`, `human` |

## Use Cases

- **Time-aware agents** — agents that need to reference "today" or "right now"
- **Scheduling context** — determine current time before creating [schedules](/capabilities/session-schedules/)
- **Logging** — timestamp entries in reports or logs
- **Timezone conversion** — answer questions about current time in different regions

## Example

```
User: What time is it in Tokyo?

Agent:
  → get_current_time({ timezone: "Asia/Tokyo", format: "human" })
  ← "Wednesday, March 6, 2026, 2:30 PM JST"
```

## See Also

- [Session Schedules](/capabilities/session-schedules/) — schedule future tasks
- [Capabilities Overview](/capabilities/)
