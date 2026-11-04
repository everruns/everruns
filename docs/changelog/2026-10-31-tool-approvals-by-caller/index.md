---
title: The person using an Agent approves its tools
description: Tool calls an Agent holds for approval are answered by the person using it, in Work Chat or through the Agent API. Channels no longer pick who approves.
date: 2026-10-31
kind: improvement
significance: notable
products: [platform, cloud]
surfaces: [ui, api]
links:
  docs: [/features/agent-api/]
---

The Agent decides which tool calls wait for approval, and the person using it answers them. In Work Chat the signed-in person approves or rejects in the chat. Through the Agent API the caller sees the tool, its arguments and risk in `pending_approvals`, and answers with `POST /sessions/{session_id}/tool-approvals`:

```json
{
  "decisions": [
    {
      "tool_call_id": "tool_approval_toolu_01",
      "decision": "allow"
    }
  ]
}
```

Channels no longer have a **Who approves a tool before it runs** setting, and the route no longer answers `403`. A saved `tool_approvals` value still loads and is dropped on the next save. See [Agent API](/features/agent-api/).
