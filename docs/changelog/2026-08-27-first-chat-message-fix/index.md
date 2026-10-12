---
title: First message of a new chat no longer drops or shows out of order
description: A new chat thread now replays from its first event when it subscribes, so the opening message is delivered and appears above later replies.
date: 2026-08-27
kind: fix
significance: notable
products: [platform, cloud]
surfaces: [ui, api]
release: "0.24.0"
links:
  docs: [/how-to/stream-events/]
  prs: [everruns/everruns#3296, everruns/everruns#3295]
---

In a fresh chat thread, the first message could be missed by the live stream and then render below every later message. The session SSE stream now accepts `after_sequence`: `after_sequence=0` replays the session from its first event before going live, and the UI sends it when it holds no events yet. `since_id` still handles reconnects; passing both returns `400`.

The chat composer is also locked when no explicit or default model resolves, with the model menu left enabled to fix it.

See [Stream events](/how-to/stream-events/), [#3296](https://github.com/everruns/everruns/pull/3296) and [#3295](https://github.com/everruns/everruns/pull/3295).
