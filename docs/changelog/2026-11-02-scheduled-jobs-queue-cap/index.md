---
title: Scheduled jobs no longer stop when one schedule has no runner
description: A schedule nothing runs now holds one queued task instead of filling the queue, so Memory sync, sandbox template builds and cleanup jobs keep running.
date: 2026-11-02
kind: fix
significance: notable
products: [platform, cloud]
surfaces: [operations]
---

The retired Knowledge index sync kept its schedule after the code that ran it
was removed. It queued a task every minute that no worker picked up, until
the queue of background tasks reached its 10,000-task limit. From then on
every scheduled job failed to start: source-backed Memories stopped syncing,
Sandbox Template builds stopped, leased provider resources such as Daytona
sandboxes were no longer cleaned up, and sessions left without an actor were
no longer resumed. The server logged it as:

```text
schedule trigger failed error=store error: standalone task queue limit exceeded: 10000/10000 pending tasks
```

The upgrade disables that schedule and cancels the tasks it left queued, so
scheduled jobs start again right after the server restarts. A schedule now
also skips its turn while its previous task is still waiting for a worker, so
a schedule nothing runs holds one queued task instead of filling the queue.
