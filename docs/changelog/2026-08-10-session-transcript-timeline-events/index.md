---
title: Session recordings split into Transcript, Timeline and Events
description: "A session page now offers three full-width views of a run: a read-only conversation replay, a curated execution history and the raw event ledger."
date: 2026-08-10
kind: improvement
significance: notable
products: [platform, cloud]
surfaces: [ui]
release: "0.18.0"
visual: "A completed session at /sessions/{id} showing the Transcript, Timeline and Events tabs with Transcript selected; crop to the tab bar and the first few transcript messages"
links:
  docs: [/features/ui/]
  prs: [everruns/everruns#3113]
---

A session page now shows a run in three separate tabs instead of one combined Timeline:

- **Transcript** is the default view: a full-width, read-only replay of the conversation.
- **Timeline** is the execution history, with noisy worker details collapsed.
- **Events** is the complete raw event ledger.

Session links, global search and **Fork into chat** open the Transcript. Work, Workspace and Cost behave as before.

See [Web UI](/features/ui/) and [#3113](https://github.com/everruns/everruns/pull/3113).
