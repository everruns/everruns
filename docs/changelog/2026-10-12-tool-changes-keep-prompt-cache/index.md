---
title: "Tool reveals keep the prompt cache on current Claude and GPT-6 models"
description: "Tool search reveals keep the cached tool list on current Claude models and on GPT-6 through the OpenAI API, and Session Trace shows each change."
date: 2026-10-12
kind: improvement
significance: notable
products: [platform, cloud, framework]
surfaces: [agent, ui, api]
links:
  docs: [/capabilities/prompt-caching/]
---

**For agents:** sessions record their tool list on the first model call and
every later change as an entry in the conversation. On Claude Opus 5.5, Opus
5, Opus 4.8, Sonnet 5.5, Haiku 5.5 and Fable 5.1 through the Claude API, and
on GPT-6 models through the OpenAI API, the request keeps that first tool list
and sends each change (a tool revealed by tool search) in the conversation,
so the next call still reads the cached prompt (see
[Prompt Caching](/capabilities/prompt-caching/#tool-changes-during-a-session)). Other models and endpoints
receive the current tool list, as before.

**In the UI:** a model call in Session Trace lists the tools it added, updated
or removed. A cache reset is blamed on the tools only when the tools sent to
the provider changed.

**In code:** recorded changes are `everruns_contracts::tool_changes::ToolChanges`,
stored on each answer and on `llm.generation` as `metadata.tool_changes`; the
trace request summary carries `tool_changes`:

```json
{ "tool_changes": { "baseline_tools": null,
  "added": ["web_fetch"], "redefined": [], "removed": [] } }
```
