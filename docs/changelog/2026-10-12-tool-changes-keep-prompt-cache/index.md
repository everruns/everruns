---
title: "Tool reveals keep the prompt cache on current Claude and GPT-6 models"
description: "Tool search reveals keep the cached tool list on current Claude models and on GPT-6 and GPT-5.6 through the OpenAI API, and Session Trace shows each change."
date: 2026-10-12
kind: improvement
significance: notable
products: [platform, cloud, framework]
surfaces: [agent, ui, api]
links:
  docs: [/capabilities/prompt-caching/]
---

**For agents:** sessions record their first tool list and each later change.
On current Claude models through the Claude API, and on
GPT-6 and GPT-5.6 through the OpenAI API, the request keeps that first tool list and sends each change (a tool revealed by tool search, or
instructions a capability adds mid-session) in the conversation, so the next
call still reads the cached prompt (see
[Prompt Caching](/capabilities/prompt-caching/#tool-changes-during-a-session)).
Elsewhere, a removed tool stays listed until the next change or cache expiry,
and calls to it are refused.

**In the UI:** a model call in Session Trace lists the tools it added, updated
or removed. A cache reset is blamed on the tools only when the tools sent to
the provider changed.

**In code:** recorded changes are `everruns_contracts::tool_changes::ToolChanges`,
stored on each answer and on `llm.generation` as `metadata.tool_changes`; the
trace request summary carries `tool_changes`:

```json
{
  "tool_changes": {
    "baseline_tools": null,
    "added": [
      "web_fetch"
    ],
    "redefined": [],
    "removed": []
  }
}
```
