---
title: Several control-plane instances without NATS no longer split live chat
description: Instances that cannot share live events through NATS now poll PostgreSQL for them and report degraded health instead of silently missing events.
date: 2026-10-28
kind: fix
significance: minor
products: [platform]
release: "0.50.0"
surfaces: [operations]
---

Live chat events travel between control-plane instances through NATS. Without
it, each instance only pushed events to its own clients, so a self-hosted
deployment running several instances without NATS showed incomplete chats with
no error anywhere.

An instance now polls PostgreSQL for live events when it cannot use NATS but
other instances may be running: `NATS_URL` is set but unreachable at startup,
or `EXPECTED_INSTANCES` is above 1 with no `NATS_URL`. Chats stay complete, at
the cost of extra database queries. `/health` reports `status: degraded` with
the delivery mode in `event_delivery`, and the system doctor shows a warning, or
a problem when several instances run and any of them only delivers in memory.

Run more than one control-plane instance with NATS, and set
`EXPECTED_INSTANCES` to the instance count.
