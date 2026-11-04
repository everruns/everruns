---
title: Long sessions with many images no longer freeze
description: Sessions whose history grew past 16 MB, such as long computer-use runs full of screenshots, retried every turn forever. They now run again.
date: 2026-10-26
kind: fix
significance: minor
products: [platform, cloud]
release: "0.50.1"
surfaces: [api]
links:
  prs: [everruns/everruns#4948]
---

Every turn loads the session's message history from the control plane. Once a
history passed 16 MB, typically a long computer-use session full of screenshots,
that load failed, and the turn was retried again and again without an answer.
The limit is now 64 MB. A history that ever exceeds it fails the turn with an
error instead of retrying forever.
