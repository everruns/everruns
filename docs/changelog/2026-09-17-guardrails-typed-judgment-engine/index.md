---
title: Guardrails can use typed judgments as their check engine
description: LLM judge and moderation guardrails can run on calibrated probabilities from TypeSafe judgments, batching every check on a stage into one request.
date: 2026-09-17
kind: feature
significance: minor
products: [platform]
surfaces: [agent, operations]
release: "0.29.0"
links:
  docs: [/capabilities/guardrails/, /integrations/typesafe/]
  prs: [everruns/everruns#3644]
---

The `llm_judge` and `moderation` guardrails take an `engine` setting:
`utility_llm` (the default, unchanged) or `jev`. With `jev`, every check on a
stage is answered in one request, and the verdict is the configured `threshold`
applied to a calibrated probability rather than a JSON verdict parsed back from
text. A check can no longer pass because a response failed to parse.

Operators enable it with the `UTILITY_TYPESAFE_API_KEY` deployment credential.
When it is unset, jev checks are skipped with a log line and the turn proceeds.
An experimental `jev` capability with a `jev_evaluate` tool is available in
development registries.

See [Guardrails](/capabilities/guardrails/) and [TypeSafe](/integrations/typesafe/). [#3644](https://github.com/everruns/everruns/pull/3644)
