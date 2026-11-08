---
title: "Workers run thousands of turns at once"
description: "Session turns no longer take worker task slots: a worker takes new turns while it has memory and its runtime keeps up. Long tool calls park after 5 minutes."
date: 2026-11-06
kind: improvement
significance: notable
products: [platform, cloud]
surfaces: [operations]
links:
  docs: [/sre/environment-variables/]
---

A turn spends almost all of its time waiting on the model, a sandbox or the network, yet it held
one of its worker's 20 task slots. Agents waited for a slot while workers sat idle. Now a worker
takes a new turn while its memory stays under 85% of its limit and its async runtime keeps up,
up to about 2 MiB per turn of that limit: some 2,600 turns on a 6 GiB worker. The server stops
handing out turns while its database pool is full, so running turns finish first.

Engine simulator, 600 agents, two turns each:

| | 20 per worker | from memory |
|---|---:|---:|
| turns running at once | 60 | 600 |
| wait before a turn starts, p99 | 190 s | 14 s |

Tool calls now park after 5 minutes instead of 30 seconds. See
[Environment Variables](/sre/environment-variables/#worker_max_running_turns):

```bash
WORKER_MAX_RUNNING_TURNS=0        # 0: from the memory limit
WORKER_TOOL_DETACH_AFTER_SECS=300
```
