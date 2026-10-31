---
title: "OpenRouter and TypeSafe decision models appear on the Models page"
description: "Model sync now lists the decision models OpenRouter and TypeSafe publish, and Jev Latest through OpenRouter answers again instead of failing on a renamed id."
date: 2026-10-31
kind: improvement
significance: notable
products: [platform, cloud]
surfaces: [ui, api]
links:
  docs: [/framework/models-and-providers/]
---

**In the UI:** the **Decisions** tab under **Registries → Models** now lists
every decision model your OpenRouter or TypeSafe key can reach, not only the
Jev versions built into Everruns. OpenRouter publishes about thirty, from
Liquid, Perplexity, Baseten, Cloudflare and others. They arrive disabled with
their pricing, like any newly discovered model. Built-in decision models
also show up the next time the page loads, so you don't wait for the daily sync.

Jev Latest through OpenRouter works again. OpenRouter renamed the alias to
`~typesafe/jev-latest`, and requests for the old id failed.

**In the API:** list the decision models for an org with

```bash
everruns models list --service decisions
```

See the driver table in [Models and providers](/framework/models-and-providers/).
