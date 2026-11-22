---
title: "Monitoring Timeline: sessions, agents, shards and errors on one axis"
description: "Monitoring's Timeline shows open sessions, agents, errors and spend together, explains what failed, and adds servers, shards and nodes for the platform."
date: 2026-11-16
kind: feature
significance: notable
products: [platform, cloud]
release: "0.53.0"
surfaces: [ui, api, operations]
links:
  docs: [/observability/monitoring/, /sre/environment-variables/]
  prs: [everruns/everruns#5419, everruns/everruns#5437]
---

![Monitoring Timeline in the platform view](./timeline@2x.png)

**Monitoring → Timeline** answers "what happened at 3 PM" on one screen: open
sessions, agents, turns, failed turns and spend share a time axis with deploy
and leader-change markers. **What failed** lists failed turns by cause and failed
tool calls by tool.

An organization sees only the path it runs on: its sessions on shards, or the
worker view. The platform view has both, a fleet chart of servers, workers,
shards and nodes, and a **Shards** tab with each shard's latest minute and history.

History is kept in PostgreSQL, one row a minute, with no metrics database. For
10-second, per-node detail, point the server at VictoriaMetrics:

```bash
MONITORING_DETAIL=prometheus
MONITORING_DETAIL_URL=http://victoriametrics:8428
```

See [Monitoring](/observability/monitoring/) and [environment variables](/sre/environment-variables/).
