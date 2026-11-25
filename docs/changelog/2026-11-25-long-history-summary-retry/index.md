---
title: Long scheduled sessions no longer fail when their history summary runs short
description: A cut-off history summary now always retries with a larger output limit, and a summary that still fails names itself instead of a generic error.
date: 2026-11-25
kind: fix
significance: minor
products: [platform, cloud]
surfaces: [ui, api]
links:
  docs: [/advanced/compaction/]
---

A session that keeps one conversation for weeks, such as a daily scheduled
agent, eventually has its earlier history summarized before a turn. On models
without built-in compaction, that summary could hit its output limit, often
because hidden reasoning used most of it. The turn then failed with "I
encountered an error while processing your request", and a rerun sometimes
worked.

Now a summary cut off at its limit always retries once with a larger one, up to
32k tokens. Everruns sizes the history it sends so the retry has room. The trace
shows the provider's real finish reason on each summary call. If the retry also
comes back cut off or empty, the turn fails with `context_summary_failed`, which
says the history was not changed and that sending the message again retries. See
[Compaction](/advanced/compaction/).
