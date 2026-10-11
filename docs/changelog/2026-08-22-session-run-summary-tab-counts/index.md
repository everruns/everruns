---
title: Session pages show a generated run summary and tab counts
description: A finished run gets a one-sentence summary in the session header, and the Work, Events and Workspace tabs show how many items each holds.
date: 2026-08-22
kind: improvement
significance: notable
products: [platform, cloud]
surfaces: [ui, api]
release: "0.21.0"
visual: "A finished session at /sessions/{id} showing the generated summary under the title and count badges on the Work, Events and Workspace tabs; crop to the header and tab bar"
links:
  docs: [/features/ui/, /sre/environment-variables/]
  prs: [everruns/everruns#3260, everruns/everruns#3266]
---

When a run finishes, Everruns writes a one-sentence summary of what it did, or which step failed and why, and shows it under the session title instead of the start time. The summary is also on the session as `run_summary`. It needs a utility model (`UTILITY_*` keys); without one, the header keeps the start time. Summary spend does not count against the session budget.

The **Work**, **Events** and **Workspace** tabs now carry counts. Empty tabs show no badge, and large counts are shortened, for example `12.4K`.

See [Web UI](/features/ui/), [Environment variables](/sre/environment-variables/), [#3260](https://github.com/everruns/everruns/pull/3260) and [#3266](https://github.com/everruns/everruns/pull/3266).
