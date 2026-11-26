---
title: Long sessions summarize less and no longer fail on a short summary
description: Large answered tool results are hidden but readable instead of summarized, and a cut-off summary always retries with a larger output limit.
date: 2026-11-25
kind: fix
significance: minor
products: [platform, cloud]
surfaces: [ui, api]
links:
  docs: [/advanced/compaction/]
---

A session that keeps one conversation for weeks, such as a daily scheduled
agent, could pay for a history summary every few model calls. On models without
built-in compaction, a summary cut off at its output limit failed the turn.

Now a tool result over 24 KiB that the model already answered is replaced by a
short excerpt, and `query_history` with its `tool_call_id` reads the original
back. A summary runs only when that is not enough, and a cut-off summary retries
once with a larger limit, up to 32k tokens; if it still fails, the turn reports
`context_summary_failed`. A turn stopped mid tool call, such as a long
`wait_task`, no longer blocks every later summary either. See
[Compaction](/advanced/compaction/).
