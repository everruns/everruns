---
title: "Model switching uses a curated list you manage"
description: "/model in Yolop now opens an ordered cross-provider model list managed with yolop models, and the local weights commands move to yolop weights."
date: 2026-08-20
kind: breaking
significance: notable
products: [yolop]
surfaces: [cli]
release: "0.17.0"
links:
  docs: ["https://github.com/everruns/yolop#providers-and-models"]
  prs: [everruns/yolop#618]
---

Switching models in Yolop now offers an ordered list across providers, instead of
whatever one provider's models API returns. `/model` and the status bar open
the list, and **Browse all models…** still reaches the per-provider catalog.
Each entry pairs a provider with a model, plus an optional effort and label, so
one pick switches both. Manage the list with `yolop models list`, `add`, `rm`,
`move`, `use` and `reset`.

**Upgrade notes:** the local weights commands moved to `yolop weights`, so
`yolop models pull` is now `yolop weights pull`. The per-provider `models`
config key is now `default_models`; an existing `models` table is still read.

See [Providers and models](https://github.com/everruns/yolop#providers-and-models) and [#618](https://github.com/everruns/yolop/pull/618).
