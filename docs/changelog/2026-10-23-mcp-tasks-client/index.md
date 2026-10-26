---
title: Agents wait on MCP servers that run tools as tasks
description: Everruns agents now accept MCP Tasks from the servers they call, checking on long tool calls until they finish instead of hitting the request timeout.
date: 2026-10-23
kind: feature
significance: notable
products: [platform, cloud, framework]
release: "0.49.0"
surfaces: [agent]
links:
  docs: [/features/mcp/]
---

Everruns now declares the MCP Tasks extension when an agent calls a tool on
an MCP server. A server that supports it can answer a slow call, such as a
build or a report, with a task instead of holding the request open:

```json
{
  "resultType": "task",
  "taskId": "build-7f3a",
  "status": "working",
  "pollIntervalMs": 2000
}
```

The agent checks on the task at the interval the server suggests, for up to
30 minutes, and gets the tool's result when it finishes. The server's status
message shows on the tool call in chat and in the session's events while it runs. A question the task
asks goes to the agent's user like any other
[MCP form question](/features/mcp/#long-running-tools), and the answer goes
back with `tasks/update`. If the turn is cancelled or the wait runs out,
Everruns cancels the task on the server.
