---
title: "Workers run thousands of turns at once"
description: "Session turns no longer take a worker's task slots: a worker takes new turns while it has memory and its runtime keeps up, and long tool calls park after 5 minutes."
date: 2026-11-06
kind: improvement
significance: notable
products: [platform, cloud]
surfaces: [operations]
links:
  docs: [/sre/environment-variables/]
---

A turn spends almost all of its time waiting on the model, a sandbox or the network, yet it
held one of its worker's task slots (`MAX_CONCURRENT_TASKS`) for most of that wait. With 20
slots per worker, agents waited for a slot while workers sat idle. Now a worker takes a new turn
while it runs fewer than its ceiling, its memory stays under 85% of its limit, and its async
runtime keeps up. The ceiling comes from the memory limit, about 2 MiB per turn, so a 6 GiB
worker runs up to about 2,600 turns. `MAX_CONCURRENT_TASKS` now caps only background tasks.
The server stops handing out new turns while its database pool is full, so running turns
finish first.

Measured with the new engine simulator (server and three workers, 600 agents, two turns each):

| | 20 turns per worker | from memory |
|---|---:|---:|
| turns running at once | 60 | 600 |
| wait before a turn starts, p99 | 190 s | 14 s |
| time for 1,200 turns | 339 s | 170 s |

A server-side tool call now runs inside its turn for 5 minutes before it parks, up from 30
seconds, since a waiting turn costs only memory. Each setting is described in
[Environment Variables](/sre/environment-variables/#worker_max_running_turns):

```bash
WORKER_MAX_RUNNING_TURNS=0        # 0 derives the ceiling from the memory limit
WORKER_MAX_RUNTIME_LAG_MS=25      # 0 turns the runtime brake off
WORKER_TOOL_DETACH_AFTER_SECS=300
```
