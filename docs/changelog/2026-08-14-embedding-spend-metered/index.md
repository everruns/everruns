---
title: Knowledge index embedding spend is metered and ledgered
description: Query embeddings made by search_index now count toward session cost and budgets, and index-sync embeddings are recorded against the organization.
date: 2026-08-14
kind: improvement
significance: notable
products: [platform, cloud]
surfaces: [api, operations]
release: "0.18.0"
links:
  docs: [/capabilities/knowledge-index/, /advanced/budgets/]
  prs: [everruns/everruns#3149, everruns/everruns#3163]
---

Embedding calls are now part of usage reporting. Each query embedding the `search_index` tool makes emits an `llm.generation` event, so it reaches the session and agent totals and is debited from budgets like chat generations. Usage is recorded even when a search matches nothing.

Knowledge index sync, which embeds every indexed chunk in the background, now writes its spend to the usage ledger against the organization. Sync spend is not debited from any user's budget, because no session triggered it.

See [Knowledge Index](/capabilities/knowledge-index/), [Budgets](/advanced/budgets/), [#3149](https://github.com/everruns/everruns/pull/3149) and [#3163](https://github.com/everruns/everruns/pull/3163).
