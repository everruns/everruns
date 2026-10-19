---
title: Model card links on the Models page
description: Models with a published vendor model card or system card now link to it from the Models page and carry the link in their model profile.
date: 2026-10-12
kind: feature
significance: minor
products: [platform, cloud, framework]
surfaces: [ui, api, code]
release: "0.48.0"
links:
  docs: [/providers/]
  prs: [everruns/everruns#4645]
---

![Models page rows for Claude Haiku 5.5 and Claude Opus 4.7, each with a Model card link after the release date](./model-card-link@2x.png)

Each model on the **Models** page now shows a **Model card** link next to its release date when
the vendor has published one: Anthropic's system cards for Claude, OpenAI's system cards for
GPT-6 and GPT-5.6, and Meta's card for Muse Wonder. The card is where the vendor documents
evaluations, safety testing and known limits.

The same link is in the model profile the API returns:

```json
{
  "name": "Muse Wonder",
  "family": "muse-wonder",
  "model_card_url": "https://research.meta.ai/static/muse-wonder-model-card"
}
```

Models without a published card carry no link.

See [Providers](/providers/). [#4645](https://github.com/everruns/everruns/pull/4645)
