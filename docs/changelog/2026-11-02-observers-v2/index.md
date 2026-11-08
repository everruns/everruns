---
title: Observers score conversations and tool calls, and alert on drops
description: Observers now grade whole conversations and single tool calls, open a health issue when the pass rate drops, and show live quality on Evals pages.
date: 2026-11-02
release: "0.51.0"
kind: feature
significance: notable
products: [platform, cloud]
surfaces: [ui, api, cli]
links:
  docs: [/features/evals/]
  prs: [everruns/everruns#5161]
---

Observers grade more than single answers. A **Conversations** grader reads the
whole transcript once a session has been idle for 10 minutes, and a **Tools**
grader reads one call's arguments and result, optionally only for the tools you
list. An alert opens a health issue, and tells the agent's authors, when the
pass rate falls under your threshold:

```bash
everruns observers update --observer-id observer_01h9 \
  --alert '{"min_pass_rate": 0.8, "window_hours": 24, "min_scored": 20}' \
  --reason 'Page me when support answers slip'
```

Evals pages show each agent's live quality over the last 7 days. A review queue
lets you mark scores the grader got wrong, backfill scores past traffic, and
judge spend counts against org and agent budgets only. See
[Evals](/features/evals/).
