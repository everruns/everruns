---
title: Image generation adds the Muse image model with a fallback
description: The image generation capability can now use Meta's Muse image model through Meta or OpenRouter, and falls back to it when only those credentials exist.
date: 2026-09-11
kind: feature
significance: minor
products: [platform, cloud]
surfaces: [agent]
release: "0.26.0"
links:
  docs: [/capabilities/openai-image-generation/]
  prs: [everruns/everruns#3523]
---

The image generation capability supports `muse-image-1.0`, served through a Meta
provider or through OpenRouter. A new `fallback` option controls what happens
when a GPT image model is configured but only Meta or OpenRouter credentials
exist: `auto` (the default) uses Muse, `off` keeps GPT strictly on OpenAI or
Azure OpenAI.

```json
{ "config": { "fallback": "auto" } }
```

Credentials now come only from configured providers. The capability no longer
reads `OPENAI_API_KEY` or `OPENAI_BASE_URL` from session secrets.

See [OpenAI image generation](/capabilities/openai-image-generation/). [#3523](https://github.com/everruns/everruns/pull/3523)
