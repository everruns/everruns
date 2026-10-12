---
title: "Agent budgets stop turns before each model call"
description: "Agent budgets are now checked before every model call in the native agent loop, so an exhausted budget stops the turn with a typed reason and budget events."
date: 2026-10-10
kind: fix
significance: notable
products: [platform, cloud, framework]
surfaces: [agent, api]
links:
  docs: [/advanced/budgets/]
  prs: [everruns/everruns#4477]
---

An agent budget is now enforced before every model call in the native loop
through a shared budget gate, the same gate the Agents API backend uses. When a
budget is exhausted, the turn stops with a typed reason and emits budget events,
instead of continuing to spend.

Budget summaries also carry the budget ID, so a client can tell which of several
stacked budgets stopped the turn. To continue, top up the exhausted budget with
the budget API or CLI.

See [Budgets](/advanced/budgets/) and [#4477](https://github.com/everruns/everruns/pull/4477).
