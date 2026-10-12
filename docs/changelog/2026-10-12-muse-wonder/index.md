---
title: Muse Wonder and reasoning effort for Muse models
description: Meta's Muse Wonder joins the model catalog with pricing and limits, and every Muse model now offers a reasoning effort picker, up to Max where Meta allows it.
date: 2026-10-12
kind: feature
significance: notable
products: [platform, cloud, framework]
surfaces: [ui, api, code]
links:
  docs: [/providers/meta/]
  prs: [everruns/everruns#4637]
---

**Muse Wonder** (`muse-wonder-1.0`, preview) is Meta's most capable Muse model, built for long
agent runs and complex coding. Sync models on a Meta provider and it shows up with its 1M-token
context window and pricing ($4.00 input, $0.20 cached input, $20.00 output per million tokens),
so usage and cost tracking work from the first run.

Every Muse model now has a reasoning effort picker: Minimal through Extra High, plus Max on
Muse Wonder and Muse Spark 1.3. Before this, Muse ran only at the level Meta picks by default.

See [Meta](/providers/meta/). [#4637](https://github.com/everruns/everruns/pull/4637)
