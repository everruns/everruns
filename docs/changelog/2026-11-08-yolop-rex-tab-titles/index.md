---
title: Yolop names your Rex tabs from session titles
description: The main yolop in a Rex session renames its window to the session title, while secondary yolops in split panes label only their own block.
date: 2026-11-08
kind: improvement
significance: minor
products: [yolop]
surfaces: [cli]
links:
  prs: [everruns/yolop#924]
---

When yolop runs inside Rex, session titles now reach the tab chrome. The main
instance renames its window to the session title, so the tab says what the
session is about. A secondary yolop in a split pane renames only its own
block, so helpers never steal the tab title.

Titles update on every effective title change and are restored on resume.
Explicit titles keep winning, and without Rex nothing changes.
