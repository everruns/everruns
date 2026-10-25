---
title: Live event delivery recovers after a NATS restart
description: JetStream-backed session streaming restores its replay stream without restarting the API.
date: 2026-10-25
kind: fix
significance: minor
products: [platform]
surfaces: [api, ui]
---

Live session updates resume after a NATS restart without requiring an API restart.
The API restores the missing in-memory JetStream stream when publishing an event
or accepting a new live subscription. Streaming clients reconnect normally;
completed messages remain available in durable session history.

Token deltas remain ephemeral. The recovery does not rewrite an existing stream's
subject policy or retry uncertain publish timeouts.
