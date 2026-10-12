---
title: Set spend budgets on agent endpoints from the Integrations tab
description: Agent and endpoint budgets can be managed on the agent's Integrations tab, and sessions are attributed to the endpoint they arrived through.
date: 2026-09-19
kind: feature
significance: notable
products: [platform, cloud]
surfaces: [ui, operations]
release: "0.30.0"
links:
  docs: [/advanced/budgets/, /how-to/enforce-a-budget/]
  prs: [everruns/everruns#3719, everruns/everruns#3623, everruns/everruns#3625]
visual: "Agent Integrations tab with an endpoint row expanded showing its budget (/agents/<id>, Integrations); crop to the budget card"
---

Sessions now record the endpoint they arrived through, and budgets can target an
agent endpoint. Existing App budgets were converted to one budget per endpoint
with the same limit and balance.

**In the UI:** with the `app_budgets` feature flag on, the agent's
**Integrations** tab manages agent and endpoint budgets, showing the period, its
UTC reset and any refusal. Members without `budget.manage` see them read-only.

Slack and FCP endpoint budgets now bind and are enforced. If you configured one
before and it never took effect, it starts limiting spend.

See [Budgets](/advanced/budgets/) and [Enforce a budget](/how-to/enforce-a-budget/). [#3719](https://github.com/everruns/everruns/pull/3719), [#3623](https://github.com/everruns/everruns/pull/3623), [#3625](https://github.com/everruns/everruns/pull/3625)
