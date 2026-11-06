---
title: Long tool calls no longer hold a worker slot
description: A server-side tool call running past 5 minutes keeps running on the worker while its turn parks and gives back its memory and session lease.
date: 2026-10-27
kind: improvement
significance: minor
products: [platform, cloud]
release: "0.50.1"
surfaces: [api]
links:
  docs: [/sre/environment-variables/]
  prs: [everruns/everruns#4954]
---

A worker used to hold one of its execution slots, the session lease and the whole turn in memory
for as long as a tool call ran. Fifty sessions waiting on long sandbox builds or slow MCP calls
filled a worker. Now a server-side call still running after 5 minutes keeps running on its own,
and its turn parks until the call finishes, then resumes on any worker. (The threshold was 30
seconds until session turns stopped using execution slots at all, see
[Workers run thousands of turns at once](/changelog/2026-11-06-workers-run-thousands-of-turns/).) One worker can keep about
a thousand such calls waiting. If the worker is lost, the call is rerun when it is safe to repeat
and reported as interrupted otherwise.

Self-hosted deployments tune it per worker:

```bash
WORKER_TOOL_DETACH_AFTER_SECS=300  # 0 keeps every call inside its turn
MAX_DETACHED_TOOL_CALLS=1000
```

See [Environment variables](/sre/environment-variables/).
