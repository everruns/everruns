---
title: "Sending a message right after cancelling a turn no longer hangs"
description: "A message sent just after a cancel now always starts its own turn, and a cancelled turn stops writing output once it reports the cancel."
date: 2026-10-12
kind: fix
significance: notable
products: [platform, cloud]
surfaces: [ui, api]
links:
  docs: [/event-reference/]
  prs: [everruns/everruns#4608]
---

Before this fix, cancelling a running turn and sending a new message straight
away could leave the session active with the new message never answered. The
cancelled turn could also keep writing model output after `turn.cancelled`.

Turns now run as leased session actors: one worker at a time owns a session,
a cancel is recorded by that worker before the request answers, and nothing
the cancelled turn produces after `turn.cancelled` reaches the session. The
next message starts a fresh turn. A worker or server that crashes mid-turn is
also recovered from the session's event log without repeating `turn.started`.
