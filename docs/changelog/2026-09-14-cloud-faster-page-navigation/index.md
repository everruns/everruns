---
title: Everruns Cloud loads Sessions, Harnesses and Skills pages faster
description: Moving between the Sessions, Harnesses and Skills pages on Everruns Cloud no longer stalls for seconds while the server renders the page.
date: 2026-09-14
kind: improvement
significance: minor
products: [cloud]
surfaces: [ui]
links:
  docs: [/getting-started/cloud/]
---

Opening **Sessions**, **Harnesses** or **Skills** in
[Everruns Cloud](https://app.everruns.com) could take several seconds, with
`/sessions` observed at about 6 seconds, even though the underlying API calls
answered in under 100 ms. Server rendering of these pages now fetches its data
directly instead of going back out through the public edge, and hovering a
sidebar link no longer starts that work before you click.

See [Everruns Cloud quickstart](/getting-started/cloud/).
