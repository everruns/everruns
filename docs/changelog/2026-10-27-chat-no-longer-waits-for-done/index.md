---
title: Chats no longer hang waiting for a finished turn
description: A live session stream that missed an update now picks it up from the event log within seconds, so a finished answer no longer looks stuck.
date: 2026-10-27
kind: fix
significance: minor
products: [platform, cloud]
release: "0.50.1"
surfaces: [api, ui]
links:
  prs: [everruns/everruns#4953]
---

The live session stream (`/sse`) delivers each event as it is written. If one
delivery was lost on the way, a client could wait for an update, such as the
turn finishing, that had already happened. The stream now checks the session's
event log whenever it has been quiet for a few seconds and sends anything it
missed, so the chat moves on by itself.
