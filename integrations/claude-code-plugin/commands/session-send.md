---
description: Send a follow-up message to an existing Everruns session
argument-hint: "<session_id> <message>"
---

Call `session_send_message` with the first argument as `session_id` and the rest as `message`. After it returns, poll `session_get_status` once.

Arguments: `$ARGUMENTS`
