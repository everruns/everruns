---
description: Send a follow-up message to an existing Everruns session
argument-hint: "<session_id> <message>"
---

Send a follow-up user message to an existing session using the `session_send_message` MCP tool.

Arguments: `$ARGUMENTS`

Steps:
1. First token is the `session_id` (format `session_{32-hex}`). Everything after it is the message body.
2. If either is missing, ask for it; do not invent.
3. Call `session_send_message` with `session_id` and `message`.
4. After the call returns, poll `session_get_status` once and surface the latest agent message or active tool call.
5. Remind the user they can keep iterating with this command, or `/everruns:session-status <session_id>` to re-poll without sending.
