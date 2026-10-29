---
title: Rolling deploys no longer interrupt running agents
description: Servers drain connections and workers hand running turns to another worker on shutdown, so a rolling restart no longer cuts streams or stalls turns.
date: 2026-10-29
kind: improvement
significance: notable
products: [platform, cloud]
surfaces: [operations, api]
---

Restarting a server now drains it. It keeps serving for a few seconds while the
load balancer moves traffic away, then ends event streams with a `disconnecting`
event (reason `shutdown`) so clients reconnect to another replica, and lets
in-flight requests finish. Tune it with `SERVER_SHUTDOWN_DELAY_SECS` (default 5)
and `SERVER_SHUTDOWN_GRACE_SECS` (default 20).

Stopping a worker now hands each running turn to another worker at its next
step, instead of holding it until the worker is killed and its lease expires.
The turn continues within about a second, and finished tool calls do not run
again.

Requests that hit a database timeout while authenticating now get
`503 Service Unavailable` with `Retry-After`, not `401 Unauthorized`.

Keep the orchestrator's kill timeout above these grace periods.
