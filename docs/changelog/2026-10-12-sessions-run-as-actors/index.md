---
title: Agent sessions now run as actors instead of workflows
description: One worker leases a session and runs its whole turn from the session's event log, replacing the per-step turn workflow, its queue hops and checkpoints.
date: 2026-10-12
kind: improvement
significance: major
products: [platform, cloud, framework]
surfaces: [api, code, operations]
release: "0.48.0"
links:
  docs: [/explanation/durable-execution/, /framework/upgrade-notes/]
  prs: [everruns/everruns#4485, everruns/everruns#4510, everruns/everruns#4512, everruns/everruns#4517, everruns/everruns#4530, everruns/everruns#4537, everruns/everruns#4608, everruns/everruns#4629]
---

Each agent session is now an actor. Storing a message marks the session
runnable in the same transaction, one worker leases it and runs the whole turn,
and the session's event log is the only turn state. Before, every step of a
turn was its own task on a durable queue, checkpointed after each step, with
steering sent as signals.

![Before: a turn workflow puts input, reason and act on a queue as separate tasks that any worker claims, with a checkpoint after each. After: the server marks the session runnable, one worker leases it, runs the whole turn and appends to the event log, and a crashed worker's lease expires so the next worker resumes from the log.](./workflow-to-actor.svg)

A cancel ends only its own turn, a message sent after it always gets an
answer, and a brief database error no longer fails a turn. Turns ran
exactly once in chaos runs with 2 servers, 3 workers and repeated kills.
[Durable execution](/explanation/durable-execution/) explains the model.

Framework apps drop the experimental `durable` feature
([upgrade notes](/framework/upgrade-notes/)):

```toml
# Before
everruns = { version = "0.47", features = ["durable"] }
# After
everruns = { version = "0.48", features = ["local"] }
```
