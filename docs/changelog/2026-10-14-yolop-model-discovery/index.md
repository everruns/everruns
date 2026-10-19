---
title: Search model catalogs and test a model from Yolop
description: "Yolop searches provider catalogs, reports model ping latency and errors, and accepts qualified model references without silently choosing another model."
date: 2026-10-14
kind: feature
significance: notable
products: [yolop]
surfaces: [cli]
links:
  docs: ["https://github.com/everruns/yolop#providers-and-models"]
  prs: [everruns/yolop#852]
---

Search a provider's model catalog without leaving Yolop, then send a small ping
to check the exact model before adding it to your menu.

```bash
yolop config models search nemotron --provider openrouter
yolop config models test openrouter/nvidia/nemotron-3-ultra-550b-a55b:free
```

Search results include provider and model arguments for `config models add`,
names, and context lengths when available. The test uses the selected provider
and reasoning settings, sends no tools or session history, reports latency,
and exits nonzero on failure. Provider charges may apply.

`-m` accepts a menu label or `provider/model` reference. Invalid explicit
selections fail without substituting another model, including when a fetched
catalog does not contain the requested model.

See [Providers and models](https://github.com/everruns/yolop#providers-and-models).
