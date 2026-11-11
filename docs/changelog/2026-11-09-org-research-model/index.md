---
title: "Point research agents at one organization research model"
description: "A Research model default picks Fable 5.2, else GPT-6 Astra, on its own, and agents can follow it, so moving them to a new flagship is one change."
date: 2026-11-09
kind: feature
significance: minor
products: [platform, cloud]
release: "0.52.0"
surfaces: [api, ui, cli]
links:
  docs: [/providers/#organization-research-model]
---

**Models → Defaults** has a new **Research model** row for the organization's
strongest model. Left on Automatic, it uses Claude Fable 5.2 when Anthropic is
connected, otherwise GPT-6 Astra on OpenAI, otherwise another provider's
research pick or the default model, and shows which one it chose.

Agents now offer **Organization research model** in their model picker.
Those agents follow the setting instead of pinning a model, so a new flagship
reaches all of them from one place. Over the API:

```json
{
  "model_role": "research"
}
```

See [Organization research model](/providers/#organization-research-model).
