---
title: Session
description: Read and update current session metadata
---

| | |
|---|---|
| **ID** | `session` |
| **Category** | Session |
| **Risk** | Low |
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

## Use Cases

- **Auto-titling** — agent sets a descriptive title based on the conversation topic
- **Context awareness** — agent reads its own session ID for logging or references

## Example

```
User: Help me debug the login issue

Agent:
  → write_session_title("Debug: Login authentication failure")
  → get_session_info()
  ← { "session_id": "ses_01abc...", "title": "Debug: Login authentication failure", "agent_name": "DevOps Agent" }
```

## See Also

- [Session Storage](/capabilities/session-storage/) — persist data within the session
- [Session Schedules](/capabilities/session-schedules/) — schedule future tasks
- [Capabilities Overview](/capabilities/)
