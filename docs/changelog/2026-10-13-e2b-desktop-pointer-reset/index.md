---
title: "E2B desktop computer use keeps the pointer where it was moved"
description: "On an E2B desktop, a mouse move or held button no longer snaps back to the screen center when the desktop session has not connected yet."
date: 2026-10-13
kind: fix
significance: minor
products: [platform, cloud]
release: "0.49.0"
surfaces: [agent]
links:
  docs: [/capabilities/computer-use/]
  prs: [everruns/everruns#4715]
---

With the experimental `computer_use_desktop` capability, the virtual display on an E2B desktop
reset itself whenever no other program was connected to it, which happens right after the desktop
starts. A `mouse_move` then reported success while the pointer sat back at the screen center, and
a held mouse button or key could be released between actions. The display now keeps its state
between actions, so moves, drags and held keys land where the agent put them. See
[Computer use](/capabilities/computer-use/).
