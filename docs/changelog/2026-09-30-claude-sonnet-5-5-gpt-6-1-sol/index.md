---
title: Claude Sonnet 5.5 and GPT-6.1 Sol with fast and ultrafast tiers
description: Claude Sonnet 5.5 becomes the favorite Sonnet model, GPT-6.1 Sol adds fast and ultrafast speed tiers, and the sunset Claude Sonnet 4.5 profile is removed.
date: 2026-09-30
kind: feature
significance: notable
products: [platform, cloud, framework]
surfaces: [ui, api]
release: "0.33.0"
links:
  docs: [/providers/anthropic/, /providers/openai/]
  prs: [everruns/everruns#3902, everruns/everruns#3925, everruns/everruns#3952]
---

Claude Sonnet 5.5 (`claude-sonnet-5-5`, plus a `[1m]` 1M-context twin) is
seeded as an enabled Anthropic favorite. It always thinks, so an explicit
`none` effort sends `low`.

GPT-6.1 Sol joins the OpenAI models. Its `controls.speed` setting accepts
`fast` and `ultrafast`. The OpenAI drivers reject a speed tier the model does
not list before sending, and usage is priced at the tier that served the call.

Anthropic sunset Claude Sonnet 4.5, so its model profile and catalog entry are
removed.

See [Anthropic](/providers/anthropic/) and [OpenAI](/providers/openai/). PRs
[#3902](https://github.com/everruns/everruns/pull/3902), [#3925](https://github.com/everruns/everruns/pull/3925), [#3952](https://github.com/everruns/everruns/pull/3952).
