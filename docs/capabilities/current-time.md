---
title: Current Time
description: Get the current date and time in various formats and timezones. Agents can check wall-clock time for scheduling decisions, logging, and time-aware responses.
---

| | |
|---|---|
| **ID** | `current_time` |
| **Category** | Core |
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

## See Also

- [Schedules](/capabilities/session-schedules/), schedule future tasks
- [Capabilities Overview](/capabilities/)
