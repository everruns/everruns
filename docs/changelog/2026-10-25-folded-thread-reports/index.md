---
title: Chat folds a thread report into one line
description: When a thread finishes, asks, or fails, Chat shows one folded line. Open it to read the report. The coordinator still gets the full text.
date: 2026-10-25
kind: improvement
significance: minor
products: [platform, cloud]
surfaces: [ui]
links:
  docs: [/features/ui/#chat-and-threads, /framework/coordination/]
---

Chat shows a thread's report as one folded line, such as "Releases is done",
instead of a message filled with the report. Open the line to read it. A
finish with nothing to add is the line only. The coordinator still receives
the full text and answers in the conversation with what matters.

![Folded thread reports in Chat, with the coordinator reply under the first line](./folded-reports@2x.png)

The Threads panel still lists what is working, what needs you, and what is
ready for review. See [Chat and threads](/features/ui/#chat-and-threads) and
[Coordination](/framework/coordination/).
