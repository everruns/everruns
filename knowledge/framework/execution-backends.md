---
type: Decision
title: "Execution Backends"
description: "Why turns run through one experimental TurnBackend entry point, why framework sessions run as leased actors in process, and why the platform's durable backend keeps queue plus per-step checkpoint instead of Workflow replay."
tags:
  - everruns
  - framework
  - durable
  - experimental
---

# Execution Backends

**Status: experimental.** The entry point exists, and the facade and the
platform server both run on it. Framework sessions run on the actor runner
(the in-process backend plus a session lease); `DurableRunner` in the
published `everruns-durable-engine` crate implements it for stored input only
(the server's path). The facade's `durable` feature, which ran facade
sessions on durable-engine's queue, was retired on 2026-10-10 (actor-based
design, step 4); see [Retired: the facade's durable
option](#retired-the-facades-durable-option). The public [Framework
Architecture](../../docs/framework/architecture.md) page diagrams these
components for users.

## Problem

The framework and the platform already run the same turn. The facade's
`InProcessRuntime` implements `RuntimeHostAdapter`, and the worker drives the
same input, reason and act activities over that adapter
([`host.rs`](../../crates/core/src/host/host.rs)), both planned by the sans-IO
`Execution` state machine. What differs is only where the steps run and what
survives a crash: in process the turn lives and dies with the caller's task; on
the platform each step is a task on `everruns-durable`'s queue, checkpointed in
PostgreSQL.

An application that outgrows in-process execution had no way across that line.
The server's entry (a server-only runner trait) and the facade's session actor
([`session.rs`](../../crates/everruns/src/session.rs)) called turns through
different surfaces, so durability was a property of which product you run, not
a choice an embedder makes.

## Decision

### One entry point: `TurnBackend`

[`TurnBackend`](../../crates/core/src/host/turn_backend.rs) in core's host is
the one place a host hands a turn to whatever runs it, for the facade session
actor and the platform server alike: start a turn, cancel it, and ask whether a
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

### The default: sessions as actors

`InProcessBackend` wraps `InProcessRuntime`: the turn runs on the task that
polls its ticket, as awaiting the runtime directly always did, and dropping
the ticket drops the turn mid-step. Routing the session actor through
`TurnBackend` changed no concurrency and no observable behavior.

Every facade session runs on [`ActorRunner`](../../crates/core/src/host/actor.rs)
(actor-based design, step 4): the same in-process backend, plus the
session's lease. A session is an actor whose state is its event log, so what
keeps two processes from running one session at once is a lease, not where
the session was opened.

- **A lease per turn, not per session.** The runner takes the lease before a
  turn starts, renews it at a third of its 30-second life while the turn runs
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
- **A held lease refuses the turn.** Starting a turn on a session another
  holder runs fails at once with "runs in another process"; nothing queues
  behind it.
- **A lost lease stops the turn.** If renewal finds another holder took the
  lease (this process stalled past its life), the ticket fails rather than
  letting two runners write. Fencing the log appends with the fence number
  comes with the bucket store (step 5), where a stalled writer is a real risk.

### The durable backend

`everruns-durable-engine` is a published crate (see
[Crate Layout](../project/crate-layout.md)), experimental like the trait, whose
runner implements `TurnBackend` over `everruns-durable`'s memory or PostgreSQL
store for the platform. Its turns run on the same runtime host adapter the
framework uses, so no second adapter is needed.

The crate already carries no transport: the worker owns the gRPC stores, the
gRPC runner constructors, `tonic`, and `everruns-internal-protocol`, and plugs
its store in through durable-engine's `TurnStore` trait, the one store
contract its runner and turn driver share. The durable isolation guard keeps it that way.

The turn driver lives in the crate too:
[`TurnTaskDriver`](../../crates/durable-engine/src/turn_driver.rs) runs one
claimed turn task (cancellation check, heartbeat, the step, completion or
failure, then the next step's enqueue or the workflow's completion) against
any `TurnStore`. A `TurnTaskHost` supplies the runtime host each step runs on
and any activities that are not turn steps. The worker keeps only its poll
loop, registration and configuration, and supplies `WorkerRuntimeHost` plus
its cleanup, reaper and scheduled activities; a test drives a whole tool turn
through the driver on the memory store with the in-process runtime as host.

### The server's runner

[`DurableRunner`](../../crates/durable-engine/src/turn_backend.rs) implements
`TurnBackend`, and the server calls it directly
([`turns.rs`](../../crates/server/src/turns.rs) builds its requests). The
server persists every input first, then starts the turn from it: a stored
message with the session's scope (which steers the running workflow, as the
server always did, rather than failing as a second turn) and recorded tool
results keyed by the waiting-turn resolution id. The runner holds no session
runtime, so handed messages, handed tool results and interrupted-turn
resumption fail on it.

The workflow starts before `start_turn` returns, so a dropped ticket changes
nothing. The ticket waits until the workflow ends and maps the end to a turn
result: a parked turn reads as a completed one, as in process, and a cancelled
workflow resolves as cancelled. It waits on the store's workflow-end signal
when the store has one, re-reading the status on a long fallback poll in case
a wakeup is lost. The memory store has one at its status writes, the single
point every path that ends a workflow (driver completion, failed or dead task,
cancel) passes through, so turns on it report back as soon as they end. PostgreSQL has none, because another process can end the workflow,
so its tickets keep the short poll; the server drops its tickets today, and a
cross-process wakeup can come with the first caller that awaits one.

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
is the store choice. `DurableBackend` stays in durable-engine, unused by the
facade, until the platform moves to actors and the crate retires (steps 6
and 7).

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

### Queue plus per-step checkpoint, not Workflow replay

The platform's durable backend keeps its model: each turn step is a queued task, and
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
