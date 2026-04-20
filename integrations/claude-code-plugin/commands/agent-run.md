---
description: Start a new session on an Everruns agent and send the first message
argument-hint: "<agent_id> <message>"
---

Start a new session on Everruns and send the first message by calling the `agent_run` MCP tool.

Arguments: `$ARGUMENTS`

Steps:
1. Parse `<agent_id>` (format `agent_{32-hex}`) and the message text. The message may span multiple words; treat everything after the id as the message.
2. If the agent id is missing, call `list_agents` and ask the user which agent to use — do not guess.
3. Call `agent_run` with `agent_id` and `message`. Forward `--title`, `--model_id`, `--budget_limit`, `--budget_soft_limit`, `--budget_currency`, `--organization_id` if the user supplied them.
4. On success, print the returned `session_id` and `message_id`.
5. Immediately follow up by polling `session_get_status` once so the user sees the session state. Suggest `/everruns:session-status <session_id>` for subsequent polls and `/everruns:session-send <session_id> "<msg>"` for follow-ups.

Do not retry automatically on errors — surface the error and let the user decide.
