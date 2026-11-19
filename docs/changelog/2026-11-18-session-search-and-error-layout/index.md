---
title: "Sessions find exact IDs and keep errors contained"
description: "The Sessions list now finds runs by full public session ID and truncates long failure messages before they overlap duration or date."
date: 2026-11-18
kind: improvement
significance: minor
products: [platform, cloud]
surfaces: [ui, api]
links:
  docs: [/features/ui/]
---

Paste a complete `session_…` ID into the **Sessions** search field to find that
run directly, without depending on its generated title. The ID path is an exact
indexed lookup, while ordinary text keeps the existing title search behavior.

Long failure messages now stay inside the outcome column and end with an
ellipsis instead of overlapping duration and date. Hover the shortened message
to read it in full.

![Sessions list with contained failure text and session ID search](./sessions-search-errors@2x.png)

See [Web UI](/features/ui/) for the Sessions surface.
