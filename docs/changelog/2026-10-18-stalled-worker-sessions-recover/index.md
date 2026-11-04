---
title: Sessions no longer hang when a worker stalls before a turn
description: A session whose worker stalls before its turn starts, or after it ends, is now picked up by another worker within minutes instead of staying active forever.
date: 2026-10-18
kind: fix
significance: notable
products: [platform, cloud]
release: "0.49.0"
surfaces: [operations]
links:
  docs: [/sre/environment-variables/]
---

A message could leave a session **active** with nothing running: the worker
that picked it up stalled on a control-plane call before the turn started, and
kept its claim on the session alive indefinitely. No turn events appeared, and
the session's sandbox stayed up because idle cleanup skips active sessions.

Workers now give up a session that makes no progress before its turn starts or
after it ends, and every worker call to the control plane has a deadline.
Another worker then runs the turn; one that keeps stalling is stopped with a
processing error. Both bounds are configurable on workers:

```bash
WORKER_ACTOR_STALL_TIMEOUT_SECS=300
WORKER_GRPC_REQUEST_TIMEOUT_SECS=120
```

See [Environment variables](/sre/environment-variables/).
