---
type: Decision
title: "Execution Backends"
description: "Why turns run through one experimental TurnBackend entry point, why the in-process runtime is its default, and why the durable backend keeps queue plus per-step checkpoint instead of Workflow replay."
tags:
  - everruns
  - framework
  - durable
  - experimental
---

# Execution Backends

**Status: experimental.** The entry point exists, and the facade and the
platform server both run on it. The in-process backend implements it in full;
`DurableRunner` in the published `everruns-durable-engine` crate implements it
for stored input only (the server's path),
and `DurableBackend` beside it runs facade sessions behind the `everruns`
`durable` feature, over an in-memory or a PostgreSQL store, for every
framework input: new messages, steering, cancellation, parked client-side
tool calls and interrupted turns. The cross-backend conformance suite passes
on all three. The public [Framework Architecture](../../docs/framework/architecture.md)
page diagrams these components for users.

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
`ResumeInterrupted`, on any backend. The durable memory backend needs no
`recover()` for it: its queue dies with the process, so the log is all that
survives. The PostgreSQL one does not either: what a dead process leaves in
the shared store is ended per session when the session is attached again
(see [PostgreSQL](#postgresql) below), and the turn continues from the log the
same way.

### The default: in process

`InProcessBackend` wraps `InProcessRuntime` and is what every facade session
uses. The turn runs on the task that polls its ticket, as awaiting the runtime
directly always did: nothing advances an unpolled ticket, and dropping it drops
the turn mid-step. Routing the session actor through `TurnBackend` therefore changed
no concurrency and no observable behavior. An engine keeps it unless its
builder selects the durable backend.

### The durable backend

`everruns-durable-engine` is a published crate (see
[Crate Layout](../project/crate-layout.md)), experimental like the trait, whose
runner implements `TurnBackend` over `everruns-durable`'s memory or PostgreSQL
store. It drives the facade's own runtime, so no second adapter is needed,
and the platform's worker is one more user of the same driver.

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
resumption fail on it; a facade session's input goes through the facade option
below, which records it itself.

The workflow starts before `start_turn` returns, so a dropped ticket changes
nothing. The ticket waits until the workflow ends and maps the end to a turn
result: a parked turn reads as a completed one, as in process, and a cancelled
workflow resolves as cancelled. It waits on the store's workflow-end signal
when the store has one, re-reading the status on a long fallback poll in case
a wakeup is lost. The memory store has one at its status writes, the single
point every path that ends a workflow (driver completion, failed or dead task,
cancel) passes through, so the facade's durable turns report back as soon as
they end. PostgreSQL has none, because another process can end the workflow,
so its tickets keep the short poll; the server drops its tickets today, and a
cross-process wakeup can come with the first caller that awaits one.

### The facade option

The `everruns` facade's opt-in `durable` feature adds
[`durable::Backend`](../../crates/everruns/src/durable.rs), selected on the
engine builder; without it an engine runs in process, and the default build
never compiles the durable engine. The generic part is
[`DurableBackend`](../../crates/durable-engine/src/durable_backend.rs) in
durable-engine: it owns a durable store (in memory, or PostgreSQL; see
below) and a pool of in-process workers, each handing claimed turn tasks to a `TurnTaskDriver` whose host is
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

Stored input takes the same paths: a stored message starts the same workflow
without writing it again, and recorded tool results continue the parked turn
under the caller's resolution id. The request's scope is ignored, since the
attached runtime resolves the session.

### PostgreSQL

`durable::Backend::postgres(store)` runs the same backend over
`everruns-durable`'s PostgreSQL store
(`PostgresWorkflowEventStore::connect` connects and applies its schema, so a
framework user never touches the driver). The queue is then shared, by other
engines and processes, which raises two questions the memory store never did.

- **Routing: a step runs where its session is attached.** A step needs the
  session's `InProcessRuntime`, which lives only in the engine that opened
  the session, so a worker of another process cannot run it; claiming it
  would fail it. Each PostgreSQL backend instance therefore gets a task queue
  of its own (`ActivityOptions::queue`, stored in the task row's `queue`
  column), enqueues every task to it, and claims from it only; a claim never
  takes another queue's tasks, and the server's workers claim from the
  default queue
  ([`backend_store.rs`](../../crates/durable-engine/src/backend_store.rs)).
  Tasks keep their plain activity type. Earlier the routing key was a tag in
  the activity type (`reason@<key>`), which worked without a schema change
  but leaked into every type a listener or an admin view saw.
  The queue is one per backend instance, not configurable: a queue shared by
  processes would hand one process a step only another one can run. Sharing
  it becomes useful only once any process can build a session's runtime from
  configuration (a runtime factory), which the framework does not have.
- **Recovery: per session, from the log.** A process that dies leaves its
  sessions' workflows running in the store, with a step claimed by a worker
  that is gone; nothing ever claims it again, and the workflow would refuse
  the session's next turn. So the first turn a freshly attached session starts
  (after `Engine::resume`, in a new process or a new engine) first ends any
  workflow left running for it: it fails the claimed steps without retry and
  cancels the workflow. The turn then starts as it would in process; a turn
  cut off in its tool calls continues through `ResumeInterrupted` from the
  session log, which therefore has to outlive the process too. Nothing
  replays the dead process's queue: its checkpoint holds no more than the
  log does, and the session actor that would own a replayed turn is gone.
  Two live engines attaching the same session is a misuse either way; the
  second ends the first's turn.
- **Tickets still wake on the workflow's end.** Every workflow a routed
  backend starts ends in its own process (driver completion, task failure,
  cancellation, recovery), so the backend's store wrappers fire a local
  end signal at those status writes, as the memory store does, instead of
  the 50 ms poll.
- Idle workers poll every 500 ms instead of 50 ms: every task routed to a
  backend is enqueued in its process, which wakes a worker, so the poll only
  catches a retry's backoff.

### Benchmark

[`benches/turn_backends.rs`](../../crates/everruns/benches/turn_backends.rs)
in the facade runs the same llmsim turns, with zero model latency, on each
backend: a text turn (one reason) and a tool turn (reason, one function tool
call, reason), at 1, 16 and 64 concurrent slots of short sessions, reporting
per-turn `send_and_wait` p50/p99 and turns per second. It lives in the facade
because it needs both backends and `Engine`; durable-engine depending back on
the facade would build a second copy of itself. The durable PostgreSQL store
is measured only in a full run with `DATABASE_URL` set; the smoke never
needs a database.

Baseline, a 4-core cloud container (one full run,
[`turn_backends_baseline.jsonl`](../../crates/everruns/benches/turn_backends_baseline.jsonl);
durable rows at 4 workers, 16 workers within noise except where shown;
the PostgreSQL rows from a later run against PostgreSQL 16 on the same
host):

| Scenario | Backend | c1 p50 ms | c1 turns/s | c16 turns/s | c64 turns/s | c64 p99 ms |
|---|---|---:|---:|---:|---:|---:|
| text | in process | 2.0 | 498 | 1238 | 1268 | 66 |
| text | durable memory | 2.3 | 436 | 864 (1019 at w16) | 894 (952 at w16) | 81 |
| tool | in process | 5.0 | 198 | 338 | 351 | 258 |
| tool | durable memory | 5.7 | 172 | 280 (300 at w16) | 282 (255 at w16) | 302 |
| text | durable PostgreSQL | 16.4 | 59 | 183 | 181 | 407 |
| tool | durable PostgreSQL | 38.4 | 26 | 89 (105 at w16) | 90 | 906 |

- **A durable turn costs ~0.4 ms (text) to ~0.8 ms (tool) over in process
  for one session**: the queue, checkpoints and wakeups. Before tickets woke
  on the workflow's end they polled every 50 ms, and that poll was almost all
  of the durable latency: c1 p50 was 51.6 ms text and 51.5 ms tool (19
  turns/s), and c16 throughput 302 and 263 turns/s.
- **On PostgreSQL a turn costs a database round trip per store write**
  (local PostgreSQL 16 on the same host): ~16 ms text and ~38 ms tool for
  one session, and throughput levels off near 180 text and 90-105 tool
  turns/s whatever the worker count, because the runner serializes its store
  calls behind one lock and the pool holds ten connections. Real model
  latency dwarfs both; neither is tuned yet.
- **Under load the in-process and memory backends are CPU-bound** and close: at c16 and c64
  durable throughput is ~70-90% of in process.
- **Turn cost grows with session history on both backends**, which is why
  the bench keeps sessions to five turns: over a 100-turn session an
  in-process tool turn went from ~5 ms to ~85 ms p50.

Run it with
`cargo bench -p everruns --features durable --bench turn_backends`
(under half a minute; `-- --summary <file>` appends JSONL in the shape of
`crates/durable/benches/baseline.jsonl`, which
`scripts/lib/durable-bench-compare.sh <file> crates/everruns/benches/turn_backends_baseline.jsonl`
compares). `-- --smoke` runs it in seconds. The bench target sets
`test = true`, so the facade CI job's existing
`cargo test -p everruns ... --all-features` runs the smoke at the cost of one
more link and no extra cargo invocation (see
[CI Build Time](../project/ci-build-time.md)). The full run, PostgreSQL rows
included, runs weekly as the `turn-backends` job of
[`durable-bench.yml`](../../.github/workflows/durable-bench.yml) against a
PostgreSQL service container. It fails when the PostgreSQL rows are missing
and gates throughput only against baseline rows with its own moniker
(`github-ubuntu-latest`); the committed baseline is from a container
(`local`), so the job reports without gating until a CI run's
`turn-backends-bench` artifact replaces it.

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
  AG-UI runs and directly on `TurnBackend`, where the resumed turn's result
  is visible, and directly once more with input the caller stored itself,
  which must match handed input on every backend. It passes on the in-process, the durable memory and, with
  `DATABASE_URL` set, the durable PostgreSQL backend. CI's durable PostgreSQL
  shard runs the PostgreSQL half with `EVERRUNS_REQUIRE_POSTGRES_TESTS` set,
  so a job without a database fails instead of passing on two backends.
- Core's default build stays wasm-safe: `TurnBackend` spawns nothing.
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
