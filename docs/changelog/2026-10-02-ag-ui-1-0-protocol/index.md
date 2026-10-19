---
title: AG-UI 1.0 endpoint and AG-UI streaming for Framework sessions
description: The AG-UI channel moves to protocol 1.0 with interrupts, resume, frontend tools and subagents, and any Framework session can serve AG-UI from your own server.
date: 2026-10-02
kind: feature
significance: major
products: [platform, cloud, framework]
surfaces: [agent, code]
release: "0.34.0"
links:
  docs: [/features/channels/, /framework/ag-ui/]
  prs: [everruns/everruns#3965, everruns/everruns#3967, everruns/everruns#3972, everruns/everruns#3979, everruns/everruns#3976, everruns/everruns#3998, everruns/everruns#4000]
---

**On the platform:** AG-UI channels speak AG-UI 1.0. Clients such as CopilotKit
get interrupts and resume for questions and approvals, token usage, frontend
tools, subagents and run metadata.

**In code:** the `ag-ui` feature lets any Rust host stream a Framework session
to an AG-UI client without the Everruns server. `Session::ag_ui(input)` returns
the run as AG-UI 1.0 events, and `ask_user` questions and tool approvals become
interrupts. A durable thread store keeps history between runs, and the
`ag-ui-axum` feature adds a ready-made route:

```bash
cargo add everruns --features ag-ui-axum
```

See [AG-UI channels](/features/channels/#ag-ui) and
[Serve AG-UI](/framework/ag-ui/). PRs [#3965](https://github.com/everruns/everruns/pull/3965), [#3967](https://github.com/everruns/everruns/pull/3967),
[#3972](https://github.com/everruns/everruns/pull/3972), [#3979](https://github.com/everruns/everruns/pull/3979), [#3976](https://github.com/everruns/everruns/pull/3976), [#3998](https://github.com/everruns/everruns/pull/3998),
[#4000](https://github.com/everruns/everruns/pull/4000).
