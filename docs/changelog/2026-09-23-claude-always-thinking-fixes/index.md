---
title: Claude always-thinking models no longer return 400s or empty answers
description: The Anthropic driver rejects prefill early, maps an explicit none effort to low, and keeps requests append-only on Opus 5.5 and other always-thinking models.
date: 2026-09-23
kind: fix
significance: notable
products: [platform, cloud, framework]
surfaces: [api, code]
release: "0.31.0"
links:
  docs: [/providers/anthropic/]
  prs: [everruns/everruns#3782, everruns/everruns#3783]
---

Requests to Claude models that always think, such as Opus 5.5, now behave
predictably:

- A request that ends on an assistant turn (prefill) fails fast with a
  configuration error that names the fix, instead of a provider 400 and a
  retry cycle.
- An explicit `none` reasoning effort maps to `low` on models where thinking
  cannot be turned off. Before, the driver sent nothing and the API ran at its
  own default.
- Each request in a session replays the previous one and only appends to it,
  so preserved thinking stays valid across turns.

See [Anthropic](/providers/anthropic/). PRs [#3782](https://github.com/everruns/everruns/pull/3782),
[#3783](https://github.com/everruns/everruns/pull/3783).
