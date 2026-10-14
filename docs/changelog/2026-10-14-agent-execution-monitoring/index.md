---
title: Monitor agent execution and actor leases at fleet scale
description: Follow thousands of sessions through model iterations and tools, map orgs to workers, and inspect actor leases in live Monitoring views.
date: 2026-10-14
kind: feature
significance: notable
products: [platform, cloud]
surfaces: [ui, api, operations]
links:
  docs: [/observability/monitoring/]
  prs: [everruns/everruns#4732]
---

**Monitoring** shows the execution fleet moving through runnable work, model
iterations, tools, and waiting. Flow is the default; Density shows changes over
time. Snapshots refresh every second, with pause and replay of observed history.

![Monitoring execution flow for five thousand sessions across twelve organizations](./execution-flow@2x.png)

The Workers tab maps organizations to worker ownership. Inspect an actor's lease
expiry, last renewal, fence, cursors, and ownership history; Metrics provides the
conventional counters alongside these views.

![Worker execution mosaic with organization ownership mapping](./worker-ownership@2x.png)

Organization admins opt in through **Settings → Features → Monitoring**. Their
view stays within that org; cross-org monitoring requires platform access.
See the [Monitoring guide](/observability/monitoring/).
