---
title: "Monitoring Timeline: sessions, agents, cells and errors on one axis"
description: "Monitoring's Timeline now shows open sessions, agents, errors and spend together, explains what failed, and in the platform view adds servers, workers, cells, nodes and a Cells tab."
date: 2026-11-16
kind: feature
significance: notable
products: [platform, cloud]
surfaces: [ui, api, operations]
links:
  docs: [/observability/monitoring/, /sre/environment-variables/]
  prs: ["https://github.com/everruns/everruns/pull/5419"]
---

![Monitoring Timeline in the platform view](./timeline@2x.png)

Open **Monitoring → Timeline** and the question "what happened at 3 PM" has one
answer on one screen: open sessions, agents and agents working, turns, failed
turns, and model spend share a time axis with deploy and leader-change markers.
**What failed** opens the range's failed turns by cause and failed tool calls by
tool, so a spike in errors names its source.

An organization sees only the execution path it runs on. On the cell-based data
plane that is its sessions on cells, without the Workers and Actors tabs;
otherwise the familiar worker view. The platform view has both, with an
**All / Workers / Cells** switch, a fleet chart of servers, workers, cells and
cell nodes, the fullest node's CPU and memory, and a new **Cells** tab: each
cell's latest minute, a silent flag after three minutes, and its history.

All of it is kept in PostgreSQL, one row a minute, permanently, with no metrics
database to run. For a closer look at a cell, point the server at VictoriaMetrics
or Prometheus and the Cells tab adds the last hour by node, every 10 seconds:

```bash
MONITORING_DETAIL=prometheus
MONITORING_DETAIL_URL=http://victoriametrics:8428
```

See [Monitoring](/observability/monitoring/) and
[environment variables](/sre/environment-variables/#monitoring-detail).
