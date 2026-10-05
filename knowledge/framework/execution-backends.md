---
type: Decision
title: "Execution Backends"
description: "Why turns run through one experimental TurnBackend seam, why the in-process runtime is its default, and why the planned durable backend keeps queue plus per-step checkpoint instead of Workflow replay."
tags:
  - everruns
  - framework
  - durable
  - experimental
---

# Execution Backends

**Status: experimental.** The seam exists and the facade runs on it; only the
in-process backend implements it so far.

## Problem

The framework and the platform already run the same turn. The facade's
`InProcessRuntime` implements `RuntimeHostAdapter`, and the worker drives the
same input, reason and act activities over that adapter
([`host.rs`](../../crates/core/src/host/host.rs)), both planned by the sans-IO
`Execution` state machine. What differs is only where the steps run and what
survives a crash: in process the turn lives and dies with the caller's task; on
the platform each step is a task on `everruns-durable`'s queue, checkpointed in
PostgreSQL.

An application that outgrows in-process execution has no way across that line
today. The worker's entry (`AgentRunner` in
[`runner.rs`](../../crates/durable-engine/src/runner.rs)) and the facade's
session actor ([`session.rs`](../../crates/everruns/src/session.rs)) call turns
through different surfaces, so durability is a property of which product you
run, not a choice an embedder makes.

## Decision

### One seam: `TurnBackend`

[`TurnBackend`](../../crates/core/src/host/turn_backend.rs) in core's host is
the one place a host hands a turn to whatever runs it. It carries only what the
facade session actor and the worker's `AgentRunner` share: start a turn, cancel
it, and ask whether a session runs one and how many run. `Execution` stays the
inner seam; a backend never plans, it decides where planned steps run and how
durably.

- **Completion is the ticket.** Starting a turn returns a `TurnTicket`, a
  future of the turn's result. The caller selects on it beside its own mailbox
  (the session actor does, to accept steering and cancellation), and each
  backend decides how completion reaches it.
- **Continuations are inputs.** A new message, client-side tool results, and a
  turn a process exit cut off are variants of the turn input, because in
  process each one continues a turn exactly as a message starts one.
- **Steering travels with the request.** The caller keeps a clone of the
  steering handle. A backend that cannot deliver input mid-turn closes it, and
  the rejected input becomes the next turn, the same rule that holds once a
  turn commits to completion.
- **Unsealed and experimental.** Third-party backends may implement it. It
  stays experimental, outside [API Stability](api-stability.md)'s promises,
  until the backend conformance suite passes on the in-process and the durable
  backend alike.

Deliberately absent for now: a separate `resume_after_tool_results(resolution_id)`
and a crash `recover()`. The worker resumes from a persisted resolution id the
in-process path has no store for, and the in-process runtime keeps no queue to
recover. Both join with the durable backend, as an input variant and a method,
once a second implementation needs them.

### The default: in process

`InProcessBackend` wraps `InProcessRuntime` and is what every facade session
uses. The turn runs on the task that polls its ticket, as awaiting the runtime
directly always did: nothing advances an unpolled ticket, and dropping it drops
the turn mid-step. Routing the session actor through the seam therefore changed
no concurrency and no observable behavior. Choosing another backend is not yet
part of the facade builder.

### The planned durable backend

`everruns-durable-engine` becomes a published crate (see
[Crate Layout](../project/crate-layout.md)) whose `DurableBackend` implements
`TurnBackend` over `everruns-durable`'s memory or PostgreSQL store, behind an
`everruns` feature. It drives the facade's own runtime, so no second adapter is
needed, and the platform's worker becomes one more user of the same driver.

The crate already carries no transport: the worker owns the gRPC stores, the
gRPC runner constructors, `tonic`, and `everruns-internal-protocol`, and plugs
its store in through durable-engine's `DurableStoreBackend` and `TaskStore`
traits. The durable isolation guard keeps it that way.

The turn driver lives in the crate too:
[`TurnTaskDriver`](../../crates/durable-engine/src/turn_driver.rs) runs one
claimed turn task (cancellation check, heartbeat, the step, completion or
failure, then the next step's enqueue or the workflow's completion) against
any `TaskStore`. A `TurnTaskHost` supplies the runtime host each step runs on
and any activities that are not turn steps. The worker keeps only its poll
loop, registration and configuration, and supplies `WorkerRuntimeHost` plus
its cleanup, reaper and scheduled activities; a test drives a whole tool turn
through the driver on the memory store with the in-process runtime as host.

### Queue plus per-step checkpoint, not Workflow replay

The durable backend keeps today's model: each turn step is a queued task, and
the turn's state is checkpointed after every step. It does not express a turn
as an `everruns-durable` `Workflow` that rebuilds its state by replaying
history.

- `TurnExecution` is already a serializable state machine. A checkpoint is its
  value; replay would reconstruct what is already stored.
- Replay would need the turn's planning logic restated as deterministic
  workflow code, a second planner beside `everruns_core::engine` that must
  never diverge from it.
- Replay history grows with the session. A long-running conversation would
  replay ever more events to resume one step, while a checkpoint stays the
  size of one turn's state.

`Workflow`, `Activity` and `WorkflowExecutor` stay engine features for
workflows that fit them; turns do not use them. The option is recorded in
[Dismissed Options](../project/dismissed-options.md).

## Success Bars

- Every facade turn, including steering, cancellation, parked client-side
  tool calls and interrupted-turn resumption, runs through `TurnBackend`, with
  existing session tests unchanged.
- A conformance suite runs the same scenarios (single turn, tool loop,
  steering versus next turn, cancel, park and resume, kill mid-act then
  recover) on every backend and requires identical event sequences.
- Core's default build stays wasm-safe: the seam spawns nothing.

## Rejected Options

- **A second adapter for durable execution.** The facade's runtime already
  implements the adapter the activities need.
- **A `wait(ticket)` method on the trait.** A self-resolving ticket composes
  with the caller's own select loop and needs no registry lookup per poll.
- **Spawning in-process turns onto their own task.** It would let a turn run
  while the session actor applies overrides or runs hooks, which the actor's
  read-modify-write of the session record relies on not happening.
