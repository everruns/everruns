---
description: Poll status and recent events for an Everruns session
argument-hint: "<session_id> [--since <event_id>] [--types <type,...>]"
---

Check the current status of an Everruns session using the `session_get_status` MCP tool.

Arguments: `$ARGUMENTS`

- Require a `session_id`. If missing, ask the user.
- Forward `--since_event_id` and `--event_types` if provided by the user. Useful event types to suggest: `turn.completed`, `output.message.completed`, `tool.completed`, `session.idled`.
- Render:
  - Top-level session status (started / active / idle).
  - Latest agent message if available.
  - A short list of the most recent events with their timestamps.
- If the session is still active, remind the user they can run this command again to poll, or `/everruns:session-send` to push more input.
