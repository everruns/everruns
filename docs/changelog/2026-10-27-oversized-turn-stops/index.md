---
title: A turn too large to resume is stopped instead of retried forever
description: A turn whose stored history grew past the worker link's message limit now ends with an error after a few attempts, instead of being retried every few seconds.
date: 2026-10-27
kind: fix
significance: minor
products: [platform, cloud]
release: "0.50.0"
surfaces: [api, ui]
links:
  prs: [everruns/everruns#4960]
---

When a worker picks up a session it first reads the open turn's events. A turn
whose events had grown past the 64 MiB limit between server and worker could
not be read, so every attempt failed before the check that stops a turn that
keeps failing, and the session was retried every few seconds without end.
That check now runs before the read, so such a turn ends after a few attempts
with "This turn was stopped because it repeatedly failed without making
progress." The session stays usable for the next message. The events are also
sent in pages of at most 16 MiB, so a large turn that fits in memory resumes
normally.
