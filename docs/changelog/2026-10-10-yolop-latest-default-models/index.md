---
title: "New setups default to the latest model on every provider"
description: "New Yolop setups default to gpt-6.1-sol, claude-opus-5-5, muse-spark-1.3 and gemini-3.8-flash; a saved model setting or the -m flag still takes priority."
date: 2026-10-10
kind: improvement
significance: minor
products: [yolop]
surfaces: [cli]
release: "0.21.0"
links:
  docs: ["https://github.com/everruns/yolop#providers-and-models"]
  prs: [everruns/yolop#834]
---

Yolop's default models move to the newest on each provider: `gpt-6.1-sol` for
`openai`, `codex` and `openrouter`, `claude-opus-5-5` for `anthropic`,
`muse-spark-1.3` for `meta` and `gemini-3.8-flash` for `google`. The default
model menu and the setup picker list the GPT-6, Claude 5.5, Muse 1.3 and
Gemini 3.8 families.

A model saved in your settings, or passed with `-m`, is unaffected.

See [Providers and models](https://github.com/everruns/yolop#providers-and-models) and [#834](https://github.com/everruns/yolop/pull/834).
