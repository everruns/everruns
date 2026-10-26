---
title: Live event delivery recovers after a NATS restart
description: Session streaming restores its JetStream replay buffer after a broker restart, without an API restart or duplicate retries after uncertain acknowledgements.
date: 2026-10-25
kind: fix
significance: minor
products: [platform]
release: "0.49.0"
surfaces: [api, ui]
links:
  prs: [everruns/everruns#4918]
---

Live session updates resume after a NATS restart without requiring an API restart.
The API restores the missing in-memory JetStream stream when publishing an event
or accepting a new live subscription. Streaming clients reconnect normally;
completed messages remain available in durable session history.

Token deltas remain ephemeral. The recovery does not rewrite an existing stream's
subject policy or retry uncertain publish timeouts.
