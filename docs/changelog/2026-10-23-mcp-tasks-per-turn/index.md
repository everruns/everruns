---
title: MCP Tasks run one agent turn per task
description: Agent runs over /mcp follow the published MCP Tasks extension, with one task per message, the reply as the task result, and official client support.
date: 2026-10-23
kind: improvement
significance: notable
products: [platform, cloud]
surfaces: [agent]
links:
  docs: [/features/mcp/]
---

`agent_run` and `session_send_message` over `/mcp` now return a standard MCP
Tasks task when the client declares the Tasks extension. Each message is its
own task, so a finished task keeps its reply and never moves on to a later turn.
Before, the task id was the session, and its status changed with every new
message.

`tasks/get` returns the agent's reply as the tool result:

```json
{
  "resultType": "complete",
  "taskId": "org_….019a….019a…",
  "status": "completed",
  "result": {
    "resultType": "complete",
    "content": [
      {
        "type": "text",
        "text": "The release notes are ready."
      }
    ]
  }
}
```

A failed turn is a tool error with a status message, and an Ask User question
appears as a form to answer with `tasks/update`. The responses are checked
against the official MCP TypeScript client in CI. See
[Long agent turns as tasks](/features/mcp/#long-agent-turns-as-tasks).
