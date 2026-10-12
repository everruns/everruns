---
title: Claude Opus 5.5, GPT-6 Sol and GPT-6 Luna models added
description: Claude Opus 5.5 becomes the recommended Opus model, and GPT-6 Luna replaces GPT-5.6 Terra as the platform default model and the default utility model.
date: 2026-09-22
kind: feature
significance: notable
products: [platform, cloud]
surfaces: [ui, api]
release: "0.30.0"
links:
  docs: [/providers/anthropic/, /providers/openai/, /sre/environment-variables/]
  prs: [everruns/everruns#3769, everruns/everruns#3771, everruns/everruns#3774]
---

Three new models are available: Claude Opus 5.5 (`claude-opus-5-5`, plus the
1M-context `claude-opus-5-5[1m]`), GPT-6 Sol (`gpt-6-sol`) and GPT-6 Luna
(`gpt-6-luna`). All are seeded as enabled favorites, and Opus 5.5 is marked as
the recommended Anthropic model. Opus 5 stays enabled for existing agents.

GPT-6 Luna replaces GPT-5.6 Terra as the platform default model. It is also the
default utility model behind Agent Analyze, Health checks and `llm_judge`
guardrails; set `UTILITY_LLM_MODEL` to choose another.

See [Anthropic](/providers/anthropic/), [OpenAI](/providers/openai/) and
[environment variables](/sre/environment-variables/). PRs [#3769](https://github.com/everruns/everruns/pull/3769),
[#3771](https://github.com/everruns/everruns/pull/3771), [#3774](https://github.com/everruns/everruns/pull/3774).
