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
agent, could pay for a history summary every few model calls, and on models
without built-in compaction a summary cut off at its output limit failed the
turn with a generic error.

Now, once the model has answered a tool result over 24 KiB, the result is
replaced by a short excerpt, and `query_history` with its `tool_call_id` reads
the original back page by page. A summary runs only when that is not enough, and
a cut-off summary retries once with a larger limit, up to 32k tokens. If it still
fails, the turn reports `context_summary_failed`, and sending the message again
retries. Results over the 64 KiB hard limit keep their last part too. See
[Compaction](/advanced/compaction/).

A turn that was stopped while a tool call was still running, such as a long
`wait_task`, also no longer blocks every later summary. Before, the session
kept growing until each turn failed with "The conversation has become too long".
