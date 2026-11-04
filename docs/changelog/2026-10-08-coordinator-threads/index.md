---
title: "Coordinator threads in Platform Chat and the Framework"
description: "Platform Chat and Framework agents can hand focused work to child threads and keep a board of what is working, what needs you and what is ready for review."
date: 2026-10-08
kind: feature
significance: notable
products: [platform, cloud, framework]
surfaces: [ui, agent, code]
release: "0.45.0"
links:
  docs: [/features/ui/#chat-and-threads, /framework/coordination/]
  prs: [everruns/everruns#4372, everruns/everruns#4377]
---

**In the UI:** with **Chat threads** turned on in **Settings → Features**,
Platform Chat can hand work to threads, which are child sessions. A **Threads**
button lists conversations and ongoing work, and background work shows its
progress, questions and failures. Custom agents use the same pattern through
the `coordination` capability.

![Chat with a finished background work card and the Work panel listing it as ready for review](./chat-work-panel@2x.png)

**In code:** the Framework's `everruns::coordination` capability runs threads
as sessions of the same engine. Threads wake the coordinator with automatic
updates, and `coordination::threads` returns each thread's status, such as
`NeedsYou`, `Working` or `ReadyForReview`.

See [Chat and threads](/features/ui/#chat-and-threads),
[Coordination](/framework/coordination/) and [#4372](https://github.com/everruns/everruns/pull/4372), [#4377](https://github.com/everruns/everruns/pull/4377).
