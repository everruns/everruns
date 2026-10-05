---
type: Decision
title: "Execution Backends"
description: "Why turns run through one experimental TurnBackend seam, why the in-process runtime is its default, and why the durable backend keeps queue plus per-step checkpoint instead of Workflow replay."
tags:
  - everruns
  - framework
  - durable
  - experimental
---

# Execution Backends

**Status: experimental.** The seam exists and the facade runs on it. The
in-process backend implements it in full; `DurableRunner` in the published
`everruns-durable-engine` crate implements it for server-persisted input only,
and `DurableBackend` beside it runs facade sessions behind the `everruns`
`durable` feature for every framework input: new messages, steering,
cancellation, parked client-side tool calls and interrupted turns. The
cross-backend conformance suite passes on both.

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
  stays experimental, outside [API Stability](api-stability.md)'s promises;
  the backend conformance suite (below) is the bar a backend meets.

Deliberately absent: a separate `resume_after_tool_results(resolution_id)`
and a crash `recover()`. The worker resumes from a persisted resolution id the
in-process path has no store for, so it is a doc-hidden `Persisted` input
variant instead, which the in-process backend rejects. A turn a process exit
cut off is resumed per session from the session log through
`ResumeInterrupted`, on any backend. The durable memory backend needed no
`recover()` for it: its queue dies with the process, so the log is all that
survives. `recover()` joins only with a shared store, for workflows another
process left behind.

### The default: in process

`InProcessBackend` wraps `InProcessRuntime` and is what every facade session
uses. The turn runs on the task that polls its ticket, as awaiting the runtime
directly always did: nothing advances an unpolled ticket, and dropping it drops
the turn mid-step. Routing the session actor through the seam therefore changed
no concurrency and no observable behavior. An engine keeps it unless its
builder selects the durable backend.

### The durable backend

`everruns-durable-engine` is a published crate (see
[Crate Layout](../project/crate-layout.md)), experimental like the seam, whose
runner implements `TurnBackend` over `everruns-durable`'s memory or PostgreSQL
store. It drives the facade's own runtime, so no second adapter is needed,
and the platform's worker is one more user of the same driver.

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

### The server's runner on the seam

[`DurableRunner`](../../crates/durable-engine/src/turn_backend.rs) implements
`TurnBackend`, and the server's `AgentRunner` is a shim over it until the
server calls the seam directly. It serves only server-persisted input: a stored
message (which steers the running workflow, as the server always did, rather
than failing as a second turn) and a stored tool resolution. Unpersisted
messages, client-side tool results and interrupted-turn resumption fail on the
runner; a facade session's messages go through the facade option below, which
persists them itself.

The workflow starts before `start_turn` returns, so a dropped ticket changes
nothing. The ticket polls the workflow status until it ends (polling first; a
notifier once several processes wait on tickets) and maps the end to a turn
result: a parked turn reads as a completed one, as in process, and a cancelled
workflow resolves as cancelled.

### The facade option

The `everruns` facade's opt-in `durable` feature adds
[`durable::Backend`](../../crates/everruns/src/durable.rs), selected on the
engine builder; without it an engine runs in process, and the default build
never compiles the durable engine. The generic part is
[`DurableBackend`](../../crates/durable-engine/src/durable_backend.rs) in
durable-engine: it owns an in-memory durable store and a pool of in-process
workers, each handing claimed turn tasks to a `TurnTaskDriver` whose host is
the runtime the session attached. The facade only wires the builder and
attaches each session's runtime; the workflow id is the session id, as on the
platform.

- **Input is persisted as in process.** A new message is written as the same
  canonical input event the in-process runtime writes, then the workflow
  starts with the requested turn id, so answers, turn ids and the session's
  event sequence match the in-process backend.
- **Steering matches in process exactly.** The session's steering handle stays
  open; the driver's steering hooks on `TurnTaskHost` drain it before each
  reason and, when a reason would finish, drain or close it in one atomic
  step, the boundaries the in-process loop uses. Steered messages cross the
  `user_prompt_submit` boundary at the reason that delivers them. The
  platform's steering stays persisted messages plus wake signals, through the
  same hooks' defaults.
- **Cancellation stops the next step.** Cancel marks the workflow cancelled,
  drops the session's in-flight step and waits until it has, failing its task
  so it never runs again.
- **Wakeup is a local notification with polling as the fallback**; a worker
  that finished a step claims again at once.
- **Workers stop with the engine**: when the last backend handle drops, or on
  an explicit shutdown. A step in flight is dropped, as an in-process turn is.

- **Continuations reuse both sides' resume logic.** The in-process runtime
  exposes, doc-hidden, the pieces its own resumes are made of, and the
  durable engine resumes through paths it already had:
  - A pause is recorded on the runtime when the driver plans it, so the
    session sees the parked calls as in process. Client-side tool results
    are recorded under the parked turn by the runtime, then the workflow
    continues from the checkpoint it parked with through the runner's
    tool-resolution resume, the path the server's stored resolutions take.
  - An interrupted turn is read from the session log by the runtime, which
    plans the act that reruns its unfinished calls; the backend starts a
    workflow whose first task is that act, checkpointed with the turn's
    state, and the driver plans on from there.
  - A continued turn's result counts only the steps that run took, as in
    process: the ticket subtracts what the turn had counted before.

Not served on the durable backend: server-persisted input, a configuration
error. Still to come: a PostgreSQL store. PostgreSQL needs more than a store constructor: workers sharing a queue across processes
would claim tasks for sessions another process attached, so it waits on a
route from a task's session to the process (or runtime factory) that can run
it, and on `recover()` for workflows a crashed process left behind.

### Benchmark

[`benches/turn_backends.rs`](../../crates/everruns/benches/turn_backends.rs)
in the facade runs the same llmsim turns, with zero model latency, on each
backend: a text turn (one reason) and a tool turn (reason, one function tool
call, reason), at 1, 16 and 64 concurrent slots of short sessions, reporting
per-turn `send_and_wait` p50/p99 and turns per second. It lives in the facade
because it needs both backends and `Engine`; durable-engine depending back on
the facade would build a second copy of itself. The durable PostgreSQL store
is not a framework backend yet, so it is not measured.

Baseline, a 4-core cloud container (one full run,
[`turn_backends_baseline.jsonl`](../../crates/everruns/benches/turn_backends_baseline.jsonl);
durable rows at 4 workers, 16 workers within noise except where shown):

| Scenario | Backend | c1 p50 ms | c1 turns/s | c16 turns/s | c64 turns/s | c64 p99 ms |
|---|---|---:|---:|---:|---:|---:|
| text | in process | 2.0 | 496 | 1113 | 1101 | 74 |
| text | durable memory | 51.6 | 19 | 302 | 780 (970 at w16) | 113 |
| tool | in process | 4.8 | 200 | 319 | 316 | 290 |
| tool | durable memory | 51.5 | 19 | 263 | 294 | 310 |

- **The ticket poll is the durable backend's latency.** A durable turn waits
  for the next 50 ms `TICKET_POLL_INTERVAL` tick after its workflow ends, so
  one session gets at most ~19 turns/s. With the interval at 1 ms (measured,
  not shipped) c1 is 3.7 ms text and 7.0 ms tool p50 (268 and 141 turns/s): the
  queue, checkpoints and wakeups cost ~1.7 ms per text turn and ~2.3 ms per
  tool turn over in process. A notifier on the workflow's terminal transition,
  with the poll kept as the fallback, would recover that; it needs a hook on
  every path that ends a workflow (driver, runner, cancel, dead task), so it is
  follow-up work rather than a constant change.
- **Under load both backends are CPU-bound** and close: at c64 throughput is
  within ~10-30% and the poll no longer dominates.
- **Turn cost grows with session history on both backends**, which is why
  the bench keeps sessions to five turns: over a 100-turn session an
  in-process tool turn went from ~5 ms to ~85 ms p50.

Run it with
`cargo bench -p everruns --features durable --bench turn_backends`
(about a minute; `-- --summary <file>` appends JSONL in the shape of
`crates/durable/benches/baseline.jsonl`, which
`scripts/lib/durable-bench-compare.sh <file> crates/everruns/benches/turn_backends_baseline.jsonl`
compares). `-- --smoke` runs it in seconds. The bench target sets
`test = true`, so the facade CI job's existing
`cargo test -p everruns ... --all-features` runs the smoke at the cost of one
more link and no extra cargo invocation (see
[CI Build Time](../project/ci-build-time.md)); there is no scheduled full run.

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
- The conformance suite,
  [`tests/backend_conformance/`](../../crates/everruns/tests/backend_conformance/main.rs)
  in the facade, runs the same scenarios (single turn, tool loop, steering
  versus next turn, cancel then next turn, park on a client-side call and
  resume with its result, a turn cut off mid-act then resumed from the log)
  on every backend and requires identical answers, turn shapes, notes and
  persisted event sequences. Park and resume runs both through a session's
  AG-UI runs and directly on the seam, where the resumed turn's result is
  visible. It passes on the in-process and the durable memory backend.
- Core's default build stays wasm-safe: the seam spawns nothing.
- The facade's default build compiles no durable engine; durable execution
  is the opt-in `durable` feature.

## Rejected Options

- **A second adapter for durable execution.** The facade's runtime already
  implements the adapter the activities need.
- **A `wait(ticket)` method on the trait.** A self-resolving ticket composes
  with the caller's own select loop and needs no registry lookup per poll.
- **Spawning in-process turns onto their own task.** It would let a turn run
  while the session actor applies overrides or runs hooks, which the actor's
  read-modify-write of the session record relies on not happening.
