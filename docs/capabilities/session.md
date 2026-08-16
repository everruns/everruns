---
title: Session Metadata for Agent Workflow Context
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

## Automatic titles

Automatic title maintenance is opt-in. Set `auto_title` to `true` in the
capability configuration to have the agent create a concise 3–7 word title
before handling the first substantive request. The title is a required pre-work
update. The agent updates it later, also before other work or a response, only
when the conversation's primary theme materially changes, not for minor
follow-ups or subtopics. Title writes update session metadata and do not count
as project or workspace file changes.

Title changes emit `session.title.updated` with the previous and new title. A
repeated write of the current title is a no-op and emits no event.

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

- [Storage](/capabilities/session-storage/), persist data within the session
- [Schedules](/capabilities/session-schedules/), schedule future tasks
- [Capabilities Overview](/capabilities/)
