---
title: "GPT-6.1 Sol, the GPT-6 family and Claude 5.5 models"
description: "Yolop's model menus list GPT-6.1 Sol with a speed setting for service tiers, the rest of the GPT-6 family, Claude Opus 5.5, Sonnet 5.5 and Fable 5.1."
date: 2026-10-01
kind: feature
significance: notable
products: [yolop]
surfaces: [cli]
release: "0.19.0"
links:
  docs: ["https://github.com/everruns/yolop#providers-and-models"]
  prs: [everruns/yolop#804, everruns/yolop#805]
---

Yolop's model suggestions for OpenAI and OpenRouter now include `gpt-6.1-sol`,
`gpt-6-sol`, `gpt-6-luna` and `gpt-6-astra`. Anthropic and OpenRouter
suggestions add `claude-opus-5-5`, `claude-sonnet-5-5` and `claude-fable-5-1`,
with their 1M-context variants where the registry has them.

A new `speed` setting picks OpenAI's service tier for models that offer one, and
a tier the model does not offer is skipped:

```bash
yolop config set speed fast
```

`YOLOP_SPEED` sets it for one run.

See [Providers and models](https://github.com/everruns/yolop#providers-and-models), [#804](https://github.com/everruns/yolop/pull/804) and [#805](https://github.com/everruns/yolop/pull/805).
