---
title: Connect a personal ChatGPT plan as a model provider
description: Self-hosted users can sign in with ChatGPT from provider settings to run on their own ChatGPT plan, with account status, reconnect controls and private models.
date: 2026-10-03
kind: feature
significance: notable
products: [platform]
surfaces: [ui]
release: "0.38.0"
visual: "Providers grid with the ChatGPT tile, and the ChatGPT detail page showing account status and Reconnect; crop to each."
links:
  docs: [/features/chatgpt/]
  prs: [everruns/everruns#4136]
---

![Everruns and OpenAI logos side by side, announcing ChatGPT plan connections](./integration@2x.png)

ChatGPT now appears in the provider grid. Open **Models > Providers > Connect
provider > ChatGPT > Sign in with ChatGPT**, approve plan use in ChatGPT, and
return to Everruns. The connection belongs to you, not the organization: its
models are available only to you, and the detail page shows account status,
quota guidance, and reconnect and disconnect controls.

Self-hosted deployments use OpenAI's Sign in with ChatGPT for open-source
applications. The ChatGPT plan transport is now part of `everruns-drivers`.

See [ChatGPT plan](/features/chatgpt/). PR [#4136](https://github.com/everruns/everruns/pull/4136).
