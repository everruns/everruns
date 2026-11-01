---
title: Monitoring shows parallel sessions and turns over time
description: A new Timeline tab in Monitoring charts parallel sessions by phase, turns by outcome, waits for a worker, spend and worker load over any range up to 30 days.
date: 2026-11-01
kind: feature
significance: notable
products: [platform, cloud]
surfaces: [ui, api]
links:
  docs: [/observability/monitoring/]
---

**Monitoring → Timeline** answers "how many sessions were running at 4 PM
yesterday, and what happened to their turns". Pick a range from 15 minutes to
30 days and see parallel sessions split by phase with their peak, turns by
outcome, the longest wait for a worker, tool calls, model spend, errors and
model latency. Deploys are marked on the charts.

Click a bucket or drag across a chart to see that window's numbers, its busiest
agents and why turns failed, then zoom in. The platform view adds total worker
slots, a load strip per worker and lease takeovers.

The server records a sample every 10 seconds and keeps history permanently.
History from before the upgrade is rebuilt from turn records, without the phase
split. `GET /api/v1/monitoring/timeline` returns the same data.
