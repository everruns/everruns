---
title: "Messages and turns survive brief database failures"
description: "A message that fails to send is no longer stored and answered anyway, and its retry is safe. A brief database failure no longer leaves a session stuck or fails its turn."
date: 2026-10-12
kind: fix
significance: notable
products: [platform, cloud]
surfaces: [api]
links:
  docs: [/event-reference/]
  prs: [everruns/everruns#4642]
---

Sending a message could answer `503` while the message had in fact been
stored, when the database failed between storing it and scheduling its turn.
The message then either ran later anyway or sat unanswered, and a client retry
could not tell which. A message and the start of its turn are now stored
together: a failed send stores nothing, and retrying it with the same
`client_message_id` either sends it or, when it was already stored, makes
sure its turn runs.

A session whose worker failed right after its turn ended could stay `active`
with nothing running, since `session.idled` was never written. The next worker
now idles it, and never answers that turn's message a second time.
