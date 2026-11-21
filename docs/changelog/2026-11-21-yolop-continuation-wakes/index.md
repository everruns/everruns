---
title: Yolop continuations no longer trigger wake errors
description: Yolop keeps automatic completion continuations in their existing turn, preventing phantom background wakes and missing Engine admission errors.
date: 2026-11-21
kind: fix
significance: minor
products: [yolop]
surfaces: [cli]
links:
  docs: [https://github.com/everruns/yolop]
  prs: [everruns/yolop#959]
---

Yolop no longer reports `Engine admission not found` when its completion check
continues unfinished work inside an existing turn. These continuations could
previously appear as a background task finishing, followed by a failed attempt
to wake the agent, even when no background task had finished.

The agent now continues the current turn without queuing another wake. Actual
background completions still notify the frontend and let the agent review their
results. No configuration change is required. See the
[Yolop documentation](https://github.com/everruns/yolop).
