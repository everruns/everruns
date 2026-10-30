---
title: Claude judges use model sampling defaults
description: Mira's Claude judges omit the temperature override so models that reject it can grade runs instead of returning HTTP 400 and an N/A score.
date: 2026-10-30
kind: fix
significance: minor
products: [mira]
surfaces: [code]
links:
  docs: [https://github.com/everruns/mira/blob/main/docs/scorers.md#llm-as-judge]
  prs: [everruns/mira#86]
---

`mira-judge` no longer sends a `temperature` override to Anthropic's Messages
API. Models that reject that parameter can now grade runs instead of returning
HTTP 400 and an N/A score.

Use `LlmJudge::claude(model)` as before; no configuration changes are required.
