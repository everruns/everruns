---
title: A2A 1.0 endpoint with task results, streaming and push notifications
description: Everruns A2A channels negotiate protocol 1.0, return agent output as task artifacts, support ListTasks and SubscribeToTask, and send push notifications.
date: 2026-10-02
kind: feature
significance: major
products: [platform, cloud, framework]
surfaces: [agent, code]
release: "0.34.0"
links:
  docs: [/features/a2a/, /framework/a2a/]
  prs: [everruns/everruns#3999, everruns/everruns#4010, everruns/everruns#4013, everruns/everruns#4014]
---

**On the platform:** A2A channels now speak A2A 1.0, chosen by the
`A2A-Version` header, with 0.3 still available. Completed tasks carry the
agent's answer as artifacts, so clients such as the `a2a` CLI read the result
back. `SendMessage` waits for the result by default; pass `returnImmediately`
to follow the task with `GetTask`, `SubscribeToTask` or a push notification.
`ListTasks` is supported, and push notification configs receive the full task
when it settles.

**In code:** the `a2a` feature of `everruns-serve` answers A2A 1.0 for every
top-level agent at `POST /v1/e/{agent}/a2a`, the same paths an Everruns A2A
endpoint uses.

See [A2A](/features/a2a/) and [Framework A2A](/framework/a2a/). PRs
[#3999](https://github.com/everruns/everruns/pull/3999), [#4010](https://github.com/everruns/everruns/pull/4010), [#4013](https://github.com/everruns/everruns/pull/4013), [#4014](https://github.com/everruns/everruns/pull/4014).
