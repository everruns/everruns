---
title: Yolop restores saved sessions with late budget events
description: Yolop restores older conversations whose budget diagnostics arrived after a turn ended, preserving their history and rewind branches.
date: 2026-11-03
kind: fix
significance: notable
products: [yolop]
surfaces: [cli, agent]
links:
  prs: [everruns/yolop#911]
---

Yolop restores older saved conversations when budget warnings or pauses were
recorded after a turn's checkpoint closed. These sessions could fail to reopen
in the terminal or show “Couldn't refresh agent history” in Paseo after upgrading
to Yolop 0.23.0.

The importer now keeps each budget diagnostic with its owning turn, including
when history was rewound or a different branch was continued. Original events
and message identities remain intact, and incomplete conversation turns still
fail validation. Reopen the existing session with a build containing this fix;
no manual edits to its history files are needed.
