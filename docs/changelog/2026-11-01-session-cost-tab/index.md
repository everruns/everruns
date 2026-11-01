---
title: Session Cost tab shows spend by category and over time
description: The session Cost tab now shows dollars split into uncached prompt, cache reads, cache writes and output, spend per day or per turn, tool calls, sandbox time and what started each turn.
date: 2026-11-01
kind: improvement
significance: minor
products: [platform, cloud]
surfaces: [ui, api, cli]
links:
  docs: [/features/ui/#cost-tab]
---

The session **Cost** tab now leads with what the session spent, split into
uncached prompt, prompt read from cache, prompt written to cache, and output.
Each category shows its tokens, average rate and share, and the cache row shows
what caching saved. Totals match the provider-reported cost of each model call
where there is one.

Spend is charted per day in your timezone or per turn, with failed turns marked.
Below it are tool calls with failures and running time, sandbox running and
paused time, and which person, schedule, trigger or channel started the turns.
A notice appears when most spend is prompt that was not read from cache.

The same data is available from `GET /v1/sessions/{session_id}/cost` and
`everruns sessions cost get --session-id <id>`. See the
[Cost tab](/features/ui/#cost-tab).
