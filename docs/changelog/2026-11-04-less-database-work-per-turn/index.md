---
title: "Agent turns put less load on PostgreSQL"
description: "Servers share event commits across turns, keep a session on the worker that ran it, batch lease renewals and build reports outside the turn."
date: 2026-11-04
kind: improvement
significance: notable
products: [platform, cloud]
surfaces: [operations]
links:
  docs: [/sre/environment-variables/]
  prs: [everruns/everruns#5186, everruns/everruns#5194, everruns/everruns#5198, everruns/everruns#5214, everruns/everruns#5217, everruns/everruns#5219]
---

Each agent turn now costs the database less, so one PostgreSQL carries more
parallel agents. Measured with the local stress bench:

- Event writes from many turns share one commit. At 500 parallel agents, WAL
  flushes per turn fell from 20 to 11.5.
- A session's next turn waits up to 300 ms for the worker that ran its last
  one and already holds its history: 77% fewer history rows read.
- A worker renews all its session leases in one statement: about 4 renewal
  statements per turn instead of 14.
- Reports are built from committed events read back from the database, not
  inside the turn: with reports on, a turn runs a third fewer statements.

Latency per model call is unchanged. Each part has a switch in
[Environment Variables](/sre/environment-variables/#throughput-tuning):

```bash
SESSION_ACTOR_STICKY_GRACE_MS=0   # any worker takes the next turn at once
EVENT_GROUP_COMMIT=off            # one transaction per event write
```
