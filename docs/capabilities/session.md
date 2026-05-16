---
title: Session Metadata Tools for Agents
description: Let agents inspect and update current session metadata, including session IDs, titles, agent names, and context used for logging or conversation organization.
sidebar:
  label: Session
---

| | |
|---|---|
| **ID** | `session` |
| **Category** | Session |
| **Features** | None |
| **Dependencies** | None |

Tools to read and update session metadata like title and agent information.

## Tools

### `get_session_info`

Get current session metadata.

Returns: session ID, title, agent name.

### `write_session_title`

Update the session title.

| Parameter | Type | Required | Description |
|---|---|---|---|
| `title` | string | yes | New session title |

## See Also

- [Storage](/capabilities/session-storage/) — persist data within the session
- [Schedules](/capabilities/session-schedules/) — schedule future tasks
- [Capabilities Overview](/capabilities/)
