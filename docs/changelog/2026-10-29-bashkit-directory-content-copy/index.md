---
title: Bashkit copies directory contents without an extra wrapper
description: Recursive copies using source/. preserve nested companion files, hidden files and empty directories directly inside the destination.
date: 2026-10-29
kind: fix
significance: notable
products: [bashkit]
surfaces: [code, agent]
links:
  prs: [everruns/bashkit#2796]
---

Bashkit now handles `cp -r source/. destination/` as a directory-content copy.
Files and subdirectories go directly inside the destination, preserving nested
companion files, hidden files and empty directories. Previously, an existing
destination could receive an extra directory named after the source.

Ordinary recursive copies retain their source-directory behavior. Copying a
directory into itself or one of its descendants remains blocked.
