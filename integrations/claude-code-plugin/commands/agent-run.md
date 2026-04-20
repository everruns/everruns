---
description: Start a new session on an Everruns agent and send the first message
argument-hint: "<agent_id_or_name> <message>"
---

Start a session with `agent_run`. First token is an agent id (`agent_{32-hex}`) or a name; if it is a name, resolve it via `list_agents` through `execute` first. Everything after is the message. On success, print `session_id` and poll `session_get_status` once.

Arguments: `$ARGUMENTS`
