---
title: "Coordinators run threads as separate processes"
description: "Yolop now uses the Everruns coordination capability: each thread runs as a headless Yolop process in its own worktree and shows on a THREADS board in the TUI."
date: 2026-10-10
kind: feature
significance: major
products: [yolop]
surfaces: [cli, agent]
release: "0.21.0"
links:
  docs: ["https://github.com/everruns/yolop/blob/main/docs/session-coordination.md"]
  prs: [everruns/yolop#835, everruns/yolop#837]
---

Yolop's session coordination now uses the Everruns `coordination` capability,
the same one Everruns Platform Chat runs, in place of Yolop's own worker pool.
Each thread runs as a headless `yolop --serve` process in its own worktree,
supervised and restarted by its coordinator, so threads keep working while the
coordinator is closed.

Enable `coordination` in a profile and start a session with it. The TUI opens a
**THREADS** board in the activity rail; Enter on a row opens that thread, and
quitting it returns to the coordinator. A thread's sandbox escalations are asked
in the coordinator's approval prompt. `yolop sessions live` lists running
sessions and threads.

See [Session coordination](https://github.com/everruns/yolop/blob/main/docs/session-coordination.md), [#835](https://github.com/everruns/yolop/pull/835) and [#837](https://github.com/everruns/yolop/pull/837).
