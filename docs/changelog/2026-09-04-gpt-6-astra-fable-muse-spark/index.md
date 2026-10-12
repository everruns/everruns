---
title: GPT-6 Astra, Claude Fable 5.1 and Muse Spark 1.3 model profiles
description: Everruns adds model profiles for OpenAI GPT-6 Astra, Claude Fable 5.1 and Muse Spark 1.3, seeds them for new organizations and covers them in live tests.
date: 2026-09-04
kind: feature
significance: notable
products: [platform, cloud, framework]
surfaces: [ui, api, code]
release: "0.24.0"
links:
  docs: [/providers/, /providers/meta/, /providers/anthropic/]
  prs: [everruns/everruns#3363, everruns/everruns#3351, everruns/everruns#3352]
---

Three new models are available with full profiles (pricing, limits, modalities):

- **OpenAI GPT-6 Astra** (`gpt-6-astra`), seeded as an enabled favorite. Its
  reasoning effort accepts a new `max` tier above `xhigh`.
- **Claude Fable 5.1** (`claude-fable-5-1` and its 1M-context variant), seeded as
  enabled favorites with adaptive thinking.
- **Muse Spark 1.3** and its Contributor tier on the Meta Model API. Muse Spark 1.2
  stays selectable.

New organizations get them on the Models page automatically.

See [Providers](/providers/), [Meta](/providers/meta/) and
[Anthropic](/providers/anthropic/). [#3363](https://github.com/everruns/everruns/pull/3363), [#3351](https://github.com/everruns/everruns/pull/3351), [#3352](https://github.com/everruns/everruns/pull/3352)
