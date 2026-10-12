---
title: "Claude Haiku 5.5 model added for Anthropic providers"
description: "The claude-haiku-5-5 model and its 1M-context twin are seeded as an enabled favorite, with adaptive thinking, tool search and the computer toolset."
date: 2026-10-07
kind: feature
significance: minor
products: [platform, cloud, framework]
surfaces: [api, ui]
release: "0.44.0"
links:
  docs: [/providers/anthropic/]
  prs: [everruns/everruns#4359]
---

Anthropic providers now offer `claude-haiku-5-5` and its `[1m]` long-context
twin. The model is seeded as an enabled favorite, so it appears in model
pickers without extra setup.

Its profile has tiered pricing, up to 128K output tokens, always-on adaptive
thinking with medium effort by default, preserved thinking, tool search and the
computer toolset. Select it in an agent's model picker or reference
`claude-haiku-5-5` wherever a model ID is accepted.

See [Anthropic](/providers/anthropic/) and [#4359](https://github.com/everruns/everruns/pull/4359).
