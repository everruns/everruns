---
title: Live event delivery recovers from startup fallback
description: Servers that start before NATS is ready return from PostgreSQL polling to live delivery without another restart, with durable cursor replay.
date: 2026-11-16
kind: fix
significance: minor
products: [platform]
release: "0.53.0"
surfaces: [api, ui]
links:
  docs: [/advanced/physical-architecture/]
---

When NATS is unavailable during startup, the server keeps serving session events
through PostgreSQL polling and retries the configured broker in the background.
After it connects, new subscriptions use NATS and existing polling streams receive
a reconnect notice. Clients resume from their durable cursor; replay and live
updates are deduplicated. Health reporting reflects the recovered backend.

See [physical architecture](/advanced/physical-architecture/) for deployment behavior.
Durable events remain in PostgreSQL. Token deltas remain best-effort during the
reconnect. This does not change the fallback for deployments without a configured
NATS URL or the reconnection behavior of an already-established NATS connection.
