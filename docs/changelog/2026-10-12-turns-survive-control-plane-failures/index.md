---
title: "Messages and turns survive brief database failures"
description: "A failed send no longer stores the message, and a brief database failure no longer leaves a session stuck as active or fails its turn."
date: 2026-10-12
kind: fix
significance: notable
products: [platform, cloud]
surfaces: [api]
links:
  docs: [/event-reference/]
  prs: [everruns/everruns#4663]
---

Sending a message could answer `503` while the message had in fact been
stored, when the database failed between storing it and scheduling its turn.
A message and the start of its turn are now stored
together: a failed send stores nothing, and retrying it with the same
`client_message_id` either sends it or, when it was already stored, makes
sure its turn runs.

A session whose worker failed right after its turn ended could stay `active`
with nothing running, since [`session.idled`](/event-reference/) was never written. The next worker
now idles it, and never answers that turn's message a second time.

A brief database failure while a turn prepared its model call, such as a
timeout loading the compaction checkpoint, failed the turn although the model
was never called. The turn is now
retried and answered.
