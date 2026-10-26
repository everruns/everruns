---
title: Choose a separate default model for Everruns Chat
description: Set an organization Chat model independently of agents and sessions, with shared provider validation and support for GPT 6.1 Terra on supported APIs.
date: 2026-10-26
kind: feature
significance: minor
products: [platform, cloud]
surfaces: [ui, api]
---

**Settings → Chat** now has its own organization model selector. Changing it leaves
the agents and sessions default unchanged. An unset Chat model inherits that
default; explicit session, agent and harness choices continue to take priority.
Work Chat channels attached to your agents keep those agents' model settings.

Owners and admins can select enabled shared models with an available provider
and tool support. The API accepts an independent patch:

```json
{
  "chat_model_id": null
}
```

This clears only the Chat selection. GPT 6.1 Terra profiles are also available
for OpenAI Responses and OpenRouter, using their respective provider model IDs.
