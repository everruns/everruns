---
type: Decision
title: "Execution Backends"
description: "Engine session hosting, shared actor execution, and the remaining internal turn backend contracts."
tags:
  - everruns
  - framework
  - durable
  - experimental
---

# Execution Backends

**Status: experimental.** Framework Engine owns session creation, admission,
commands, lifecycle outcomes and queries through a coherent
[session hosting profile](engine-durable-sessions.md). Its default memory profile,
native SQLite profile and Durable Object profile use core's shared
`SessionActorDriver`. Memory has process-lifetime durability; choosing it does not
make accepted input survive process loss.

The platform server retains its internal `ActorTurnBackend`: it transactionally
marks sessions runnable and workers execute them through the same actor driver.
The per-step queue runner and facade queue feature remain retired.

## External session hosting profiles

Applications register ordinary Agents with Engine and select `SessionHosting`.
The profile owns the authority and activation domain; it supplies epoch-scoped
runtime ports rather than asking application code to assemble stores. Framework
start/completion hooks and outcome reconstruction stay inside Engine. serve
accepts that Engine, and the DO plugin supplies both Wasm object-local and native
executor placements. See [Shared Execution with Durable Object
Authority](durable-object-native.md) for the platform-specific guarantees.

`EngineBuilder::execution_backend` was removed. A turn factory cannot establish
durable creation, admission, callbacks or observation. Core's `TurnBackend` is
still an internal host integration contract, rather than the Framework plugin API.

Explicit legacy `LocalConfig` and custom host backends retain the older facade
actor lane and its documented weaker guarantees. They do not become coherent
hosting profiles merely because individual stores persist. That lane is separate
from default Engine and the registered hosting paths described above.

Both actor runners stop renewal attempts when the last confirmed lease expires,
including when a store call hangs. Canonical stale writes must still be rejected
by the storage authority.

## Problem

The framework and the platform already run the same turn. The facade's
`InProcessRuntime` implements `RuntimeHostAdapter`, and the worker drives the
same input, reason and act activities over that adapter
([`host.rs`](../../crates/core/src/host/host.rs)), both planned by the sans-IO
`Execution` state machine. What differs is only where the steps run and what
survives a crash: in process the turn lives and dies with the caller's task; on
the platform each step was then a task on `everruns-durable`'s queue,
checkpointed in PostgreSQL.

An application that outgrows in-process execution had no way across that line.
The server's entry (a server-only runner trait) and the facade's session actor
([`session.rs`](../../crates/everruns/src/session.rs)) called turns through
different surfaces, so durability was a property of which product you run, not
a choice an embedder makes.

The [Engine-owned session hosting boundary](engine-durable-sessions.md) extends
above the turn seam. The following sections describe the lower-level contracts
still used by platform hosts and the explicit legacy facade lane.

## Decision

### Internal turn delegation: `TurnBackend`

[`TurnBackend`](../../crates/core/src/host/turn_backend.rs) in core's host is
the one place a host hands a turn to whatever runs it, for the explicit legacy facade session
actor and the platform server: start a turn, cancel it, and ask whether a
session runs one and how many run. `Execution` stays the inner turn interface;
a backend never plans, it decides where planned steps run and how durably.

- **Completion is the ticket.** Starting a turn returns a `TurnTicket`, a
  future of the turn's result. The caller selects on it beside its own mailbox
  (the session actor does, to accept steering and cancellation), and each
  backend decides how completion reaches it.
- **Continuations are inputs.** A new message, client-side tool results, and a
  turn a process exit cut off are variants of the turn input, because in
  process each one continues a turn exactly as a message starts one.
- **Input is handed or stored.** A caller either hands the backend input to
  record (a message, tool results) or names input it already recorded through
  its own store (a stored message, recorded tool results keyed by a resolution
  id). Every backend serves the stored forms, so a host that persists input
  itself, as the platform server does, starts and continues turns the way a
  framework host can. A backend that cannot look the session up takes its
  organization, harness and agent from the request's scope.
- **Steering travels with the request.** The caller keeps a clone of the
  steering handle. A backend that cannot deliver input mid-turn closes it, and
  the rejected input becomes the next turn, the same rule that holds once a
  turn commits to completion.
- **Unsealed and experimental.** Third-party backends may implement it. It
  stays experimental, outside [API Stability](api-stability.md)'s promises;
  the backend conformance suite (below) is the bar a backend meets.

Deliberately absent: a separate `resume_after_tool_results(resolution_id)`
and a crash `recover()`. Resuming from a stored resolution is the recorded
tool-results input, which the in-process backend serves too (the caller records
results under the parked turn, then continues it); an earlier design kept it as
a doc-hidden, server-only input variant that no backend served together with
the rest, and that split is gone. A turn a process exit
cut off is resumed per session from the session log through
`ResumeInterrupted`, on any backend.

### One turn loop, resumed by folding the log

The loop that drives a whole turn (reason, act, reason, until it completes or
parks) is host-generic: [`drive_turn`](../../crates/core/src/host/turn_loop.rs)
runs over any `RuntimeHostAdapter` and takes the turn's input through one
port, `TurnInbox`, whose atomic take-or-close at a finishing reason is what
splits steering from the next turn. `InProcessRuntime` implements the inbox
over its steering queue, wake queue and in-memory parks; a worker running whole
turns on a session lease implements its own over the session log
([`actor_runtime/inbox.rs`](../../crates/core/src/host/session_actor/inbox.rs)).
A fresh turn opens through the same host-generic entry step,
[`turn_entry_plan`](../../crates/core/src/host/turn_loop.rs), on both.

Resume is a fold over the session log
([`turn_resume.rs`](../../crates/core/src/host/turn_resume.rs)) that needs
only the tail from the last `turn.started`. A turn parked on client-side or
approval calls whose results are now recorded is just an open turn with
nothing left to rerun, so resuming from the log covers it without a parked-turn
store. Rerun decisions look tools up in the surface a reason on that host
would hand its act (host-augmented and MCP tools included), not the plain
assembled context.

### Legacy facade lane: sessions as actors

`InProcessBackend` wraps `InProcessRuntime`: the turn runs on the task that
polls its ticket, as awaiting the runtime directly always did, and dropping
the ticket drops the turn mid-step. Routing the session actor through
`TurnBackend` changed no concurrency and no observable behavior.

Explicit legacy facade sessions run on [`ActorRunner`](../../crates/core/src/host/actor.rs)
(actor-based design, step 4): the same in-process backend, plus the
session's lease. A session is an actor whose state is its event log, so what
keeps two processes from running one session at once is a lease, not where
the session was opened.

- **A lease per turn, not per session.** The runner takes the lease before a
  turn starts, renews it at a third of its 10-second life while the turn runs
  (a turn waiting on an approval or a question keeps it), and releases it when
  the ticket resolves or drops. A session between turns, or parked on
  client-side tool results, holds nothing, so another process may run it.
- **One holder per store instance.** `SessionLeases` (acquire, renew,
  release, a fence number that grows each time the lease changes hands) is a
  host backend, `HostBackends::session_leases`: in memory by default, and in
  the local profile's SQLite database
  ([`session_leases.rs`](../../crates/everruns/src/local/session_leases.rs))
  so processes sharing a data directory share it. Engines in one process that
  share backends are one holder and never block each other.
- **A held lease is waited out once.** A process that died holding a lease
  (a turn was waiting on an approval when it was killed) cannot release it,
  and nothing tells a dead holder from a live one. So a turn that finds the
  lease held waits up to one lease life for it: after a crash the restarted
  process picks the session up once the dead holder's lease runs out (the
  AgentCore restart test relies on this), while a live holder keeps renewing
  and the turn fails with "runs in another process". The short life keeps
  that wait short.
- **A lost lease stops the turn.** If renewal finds another holder took the
  lease (this process stalled past its life), the ticket fails rather than
  letting two runners write. Fencing the log appends with the fence number
  comes with the bucket store (step 5), where a stalled writer is a real risk.

### The platform: sessions as actors on workers

The server's turn backend is
[`ActorTurnBackend`](../../crates/server/src/turns/actor_backend.rs), in
production and DEV_MODE alike. Starting a turn marks the session's actor
runnable in the transaction that stored the input; a worker's
[`actor_runtime`](../../crates/worker/src/actor_runtime/mod.rs) leases the
actor, folds the log's tail, and runs one whole turn with `drive_turn`
(fresh through `turn_entry_plan`, or resumed through `resume_open_turn`),
renewing the lease every third of its life and releasing it with the input
it consumed. Cancel has a single writer: the live holder ends and records the
turn, or, when nothing holds the actor, the server records it. A turn that
keeps killing its worker is sealed after a few leases. The runnable index,
the cursor, cancel and the deploy transition are specified in
[Durable Execution Engine](../operations/durable-execution-engine.md#session-actors-turn-execution).

Differences from the framework's `ActorRunner`: the lease is per runnable
session rather than per turn call, the inbox is the stored log rather than an
in-memory queue (the server persists every input before it marks), and a
crash costs a lease life, after which another worker resumes the turn from
the log.

### Retired: the per-step durable backend

`everruns-durable-engine` ran a turn as queued steps (`process_input`,
`reason`, `act`), each a task on `everruns-durable`'s queue with the turn's
state checkpointed after it, behind `DurableRunner`. It ran platform turns
until the actor cutover (step 6c) and was deleted in step 7 with everything
that existed only for it: the turn workflow and its `USER_MESSAGE` signals,
the step hand-off and stranded-run sweep, the deduplicated resolution enqueue,
the core stream heartbeater, and the old crash and recovery-matrix suites
(their scenarios run on the actor runtime now).

- The actor runtime gives the same recovery from the log alone: a crashed
  worker's lease expires and another worker folds the log and resumes the
  turn, without a second copy of the turn's state per step.
- Each step cost a queue round trip and a checkpoint write, and steering
  needed signals delivered under the workflow lock; a lease and an inbox
  cursor replace both.

The worker keeps a small task loop of its own
([`task_driver.rs`](../../crates/worker/src/task_driver.rs)) for the queued
activities that are not turns (leased-resource cleanup, the session task
reaper, scheduled channel and agent-trigger invocations).

### Retired: the facade's durable option

Until 2026-10-10 the facade's opt-in `durable` feature (`durable::Backend`)
ran a session's turn steps as tasks on durable-engine's queue
(`DurableBackend`, over memory or PostgreSQL), with in-process workers. It was
retired by the actor-based design (step 4), with no replacement flag:

- Durability comes from the session log, not from a queue. A turn a process
  exit cut off resumes from the log (`ResumeInterrupted`) on any backend, and
  the PostgreSQL option itself recovered that way: it ended what the dead
  process left in its queue and continued from the log.
- Its PostgreSQL queue could not share work: a step ran only in the engine
  that opened its session, so each backend instance claimed only its own
  queue.
- Each step added a queue write and a checkpoint (0.4 to 0.8 ms in memory,
  16 to 38 ms per turn on PostgreSQL) for no recovery the log did not
  already give.

Which process runs a session is now the lease's job (above), and durability
is the store choice. `DurableBackend` itself was deleted from durable-engine
once nothing used it, and the crate followed in step 7.

### Benchmark

[`benches/turn_backends.rs`](../../crates/everruns/benches/turn_backends.rs)
in the facade runs llmsim turns, with zero model latency, on an engine's
sessions: a text turn (one reason) and a tool turn (reason, one function tool
call, reason), at 1, 16 and 64 concurrent slots of short sessions, reporting
per-turn `send_and_wait` p50/p99 and turns per second. Its rows keep the
`in_process` name the baseline was recorded under (the actor runner is the
in-process backend plus a lease).

Baseline, a 4-core cloud container (one full run, before the lease,
[`turn_backends_baseline.jsonl`](../../crates/everruns/benches/turn_backends_baseline.jsonl)):

| Scenario | c1 p50 ms | c1 turns/s | c16 turns/s | c64 turns/s | c64 p99 ms |
|---|---:|---:|---:|---:|---:|
| text | 2.0 | 498 | 1238 | 1268 | 66 |
| tool | 5.0 | 198 | 338 | 351 | 258 |

The retired durable rows, for the record: durable memory added ~0.4 ms (text)
to ~0.8 ms (tool) per turn and ran at 70-90% of in-process throughput under
load; durable PostgreSQL took 16.4 ms (text) and 38.4 ms (tool) per turn for
one session and levelled off near 180 and 90-105 turns/s. Turn cost grows
with session history (over a 100-turn session an in-process tool turn went
from ~5 ms to ~85 ms p50), which is why the bench keeps sessions to five
turns.

Run it with `cargo bench -p everruns --bench turn_backends` (under half a
minute; `-- --summary <file>` appends JSONL in the shape of
`crates/durable/benches/baseline.jsonl`, which
`scripts/lib/durable-bench-compare.sh <file> crates/everruns/benches/turn_backends_baseline.jsonl`
compares). `-- --smoke` runs it in seconds. The bench target sets
`test = true`, so the facade CI job's existing
`cargo test -p everruns ... --all-features` runs the smoke at the cost of one
more link and no extra cargo invocation (see
[CI Build Time](../project/ci-build-time.md)). The full run runs weekly as
the `turn-backends` job of
[`durable-bench.yml`](../../.github/workflows/durable-bench.yml). It gates
throughput only against baseline rows with its own moniker
(`github-ubuntu-latest`).

### Historical: queue plus per-step checkpoint, not Workflow replay

The platform now runs turns as actors resumed by a log fold. When the retired
durable backend ran them, it kept this model: each turn step a queued
task, the turn's state checkpointed after every step. It does not express a turn
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

`everruns-durable` no longer ships a replayed workflow engine; it was removed
because nothing ran it. The option is recorded in
[Dismissed Options](../project/dismissed-options.md).

## Success Bars

- Default and registered Framework sessions run through the shared session actor
  with authority-owned commands and outcomes; their Engine conformance suite covers
  steering, cancellation, client-tool waits, lifecycle recovery and observation.
- Explicit legacy facade turns retain the internal `TurnBackend` delegation
  contract and its separate conformance suite.
- The conformance suite,
  [`tests/backend_conformance/`](../../crates/everruns/tests/backend_conformance/main.rs)
  in the facade, runs its scenarios (single turn, tool loop, steering versus
  next turn, cancel then next turn, park on a client-side call and resume
  with its result, a turn cut off mid-act then resumed from the log) on an
  engine's sessions. Park and resume also runs directly on `TurnBackend`,
  where the resumed turn's result is visible, on `InProcessBackend` and on
  `ActorRunner`, with handed and with stored input, and requires identical
  results and persisted event sequences. Each store the actor runner gains
  joins it.
- Two lease holders on one store never both hold a session's lease; a
  released or expired lease passes to the next holder with a higher fence
  ([`actor.rs`](../../crates/core/src/host/actor.rs) and
  [`session_leases.rs`](../../crates/everruns/src/local/session_leases.rs)
  tests), and a facade session whose lease another process holds refuses
  the turn
  ([`local_session_leases_test.rs`](../../crates/everruns/tests/local/local_session_leases_test.rs)).
- Core's default build stays wasm-safe: `TurnBackend` and the actor runner
  live in `host`.
- The facade compiles no durable engine, in any feature set.

## Rejected Options

- **A second adapter for durable execution.** The facade's runtime already
  implements the adapter the activities need.
- **A `wait(ticket)` method on the trait.** A self-resolving ticket composes
  with the caller's own select loop and needs no registry lookup per poll.
- **Spawning in-process turns onto their own task.** It would let a turn run
  while the session actor applies overrides or runs hooks, which the actor's
  read-modify-write of the session record relies on not happening.
