---
title: Connect an LLM provider with one pasted key, checked before saving
description: LLM provider setup now offers a one-click grid of common providers, verifies the API key with the provider, and enables models and a default for you.
date: 2026-09-11
kind: improvement
significance: notable
products: [platform, cloud, cli]
surfaces: [ui, api, cli]
release: "0.26.0"
links:
  docs: [/providers/, /reference/cli/providers/]
  prs: [everruns/everruns#3526, everruns/everruns#3516, everruns/everruns#3515, everruns/everruns#3517, everruns/everruns#3512]
visual: "Settings provider connect grid with the provider tiles (/settings/providers or equivalent); crop to the grid"
---

**In the UI:** a **Connect** grid offers one tile each for Anthropic, OpenAI,
Gemini, OpenRouter and Meta, so connecting one takes a pasted key. Providers that
need more than a key stay behind **Another provider**. Keys are checked as you
type, and only an outright rejection is shown. Once a key is saved, Everruns
discovers the provider's models, enables a shortlist, stars a few favorites and
sets a default model if none exists.

**In the API and CLI:** `POST /v1/providers/check-credentials` checks a key
without storing it:

```bash
everruns providers check-credentials check --provider-type openai --api-key "$OPENAI_API_KEY"
```

Disabled models can be re-enabled. See [Providers](/providers/) and
[CLI providers](/reference/cli/providers/). [#3526](https://github.com/everruns/everruns/pull/3526), [#3516](https://github.com/everruns/everruns/pull/3516), [#3515](https://github.com/everruns/everruns/pull/3515)
