---
title: "Agents can cap output tokens per response"
description: "A per-agent Max output tokens setting caps every model response, so agents keep running on a provider balance too low for the model's full output limit."
date: 2026-10-12
kind: feature
significance: minor
products: [platform, cloud]
surfaces: [ui, api, cli]
release: "0.48.0"
links:
  docs: [/providers/openrouter/]
---

Set **Max output tokens** in the agent's settings, next to the default model, to cap how many
tokens the model may write in one response. Leave it empty to keep the provider default.

```bash
everruns agents update agent_01h9 --max-output-tokens 4096 --reason 'Fit the OpenRouter balance'
```

Everruns sends the value as the request's `max_tokens` and lowers it to the model's own output
limit when that is known. Without it [OpenRouter](/providers/openrouter/) reserves the model's full output limit against
your credits and returns HTTP 402 ("can only afford N tokens") on a low balance; a ceiling below
that number lets the call through. The field is `max_output_tokens` on the agent API, and agent
packages export and import it.
