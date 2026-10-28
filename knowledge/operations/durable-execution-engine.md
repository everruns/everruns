---
type: Specification
title: "Durable Execution Engine Specification"
description: "PostgreSQL-backed durable task queue, event log and schedules, and the session actors that run platform turns."
tags:
  - everruns
  - operations
---
# Durable Execution Engine Specification

## Abstract

Custom PostgreSQL-backed durable execution engine: a distributed task queue with automatic retries, circuit breakers, an event log, and schedules.

`everruns-durable` is a generic core: workflow records, activities, tasks and
schedules, with no `everruns-*` dependency (enforced by
`scripts/lib/check-durable-isolation.sh`). Agent semantics live above it.

Platform turns no longer run on it. Since the actor cutover (step 6c,
2026-10-11) a turn is a session actor: intake marks the session runnable, a
worker leases it and runs the whole turn, and the session's event log is the
only turn state ([Session actors](#session-actors-turn-execution)). The queue
still carries the activities that are not turns: leased-resource cleanup, the
session task reaper, and scheduled channel and agent-trigger invocations,
which the worker runs through its own task loop
(`crates/worker/src/task_driver.rs`). The per-step turn path
(`everruns-durable-engine`, its turn workflow and signals) was deleted in step
7 ([Execution Backends](../framework/execution-backends.md#retired-the-per-step-durable-backend)).
The durable crate owns persistence, retries, and activity scheduling.
The worker owns no database driver; database connection construction
belongs only to server, durable, and the `everruns` facade (embedded SQLite), enforced by
[`check-database-driver-isolation.sh`](../../scripts/lib/check-database-driver-isolation.sh).

## Goals

1. **Self-contained** - `everruns-durable` crate with no Temporal dependencies
2. **PostgreSQL-only** - No additional infrastructure required
3. **Testable** - Unit tests, integration tests, load/stress tests
4. **Reliable** - Retries, circuit breakers, timeouts, dead letter queues
5. **Simple** - Callers drive workflows step by step over the queue; no replayed state machines
6. **Scalable** - Support 1000+ concurrent workers
7. **Observable** - OpenTelemetry integration

## Non-Goals

1. Multi-region replication (use PostgreSQL replication)
2. Language-agnostic SDKs (Rust only)
3. Visual workflow designer
4. Multi-tenancy (deferred)

## Architecture

```
┌─────────────────────────────────────────────────────────────────────────────┐
│                            everruns-durable                                  │
├─────────────────────────────────────────────────────────────────────────────┤
│  ┌──────────────┐  ┌──────────────┐  ┌──────────────┐  ┌───────────────┐   │
│  │   Task       │  │   Stale-task │  │   Worker     │  │   Scheduler   │   │
│  │   Queue      │  │   Reaper     │  │   Pool       │  │               │   │
│  └──────┬───────┘  └──────┬───────┘  └──────┬───────┘  └───────┬───────┘   │
│         │                 │                 │                   │           │
│  ┌──────┴─────────────────┴─────────────────┴───────────────────┴────────┐ │
│  │                         WorkflowEventStore                             │ │
│  │  (PostgreSQL: durable_workflow_instances, durable_workflow_events,    │ │
│  │   durable_task_queue, durable_workers)                                │ │
│  └────────────────────────────────────────────────────────────────────────┘ │
│                                                                              │
│  ┌────────────────────────────────────────────────────────────────────────┐ │
│  │  Reliability: RetryPolicy, CircuitBreaker, DLQ                        │ │
│  └────────────────────────────────────────────────────────────────────────┘ │
│                                                                              │
│  ┌────────────────────────────────────────────────────────────────────────┐ │
│  │  Observability: OTel Tracing, Metrics, Admin API                      │ │
│  └────────────────────────────────────────────────────────────────────────┘ │
└─────────────────────────────────────────────────────────────────────────────┘
```

`WorkflowEventStore` is an umbrella over focused traits (`EventLog`, `TaskQueue`, `WorkerRegistry`, `DeadLetters`, `CircuitBreakers`, `Schedules`, `DurableAdmin`), blanket-implemented, so a component can bound on only the slice it needs (a worker: `TaskQueue + WorkerRegistry`). Store methods have no silently-succeeding defaults; both stores implement every method, and only derived defaults (for example `count_events` via `load_events`) remain. See `crates/durable/src/persistence/store.rs`.

### No replayed workflow engine

Decision (2026-10-10): the crate has no generic replayed workflow engine. It
once carried one (`Workflow` and `Activity` traits, `WorkflowExecutor` with
timers, child workflows and system tasks, `TimeoutManager`) behind an
experimental `workflows` feature. Nothing in production ran it: the worker and
server drive the queue directly, and turns used per-step checkpoints rather
than replay (and now run as session actors). The actor-based execution design
retires it, so it was removed instead of maintained. The record types it
shared with the queue (`WorkflowEvent`, `ActivityOptions`, `WorkflowError`,
`ActivityError`) stay; `WorkflowSignal` and the signal inbox went with the
turn workflow in step 7. `WorkerPool` is core: the server's cluster-once
maintenance jobs run on it.

## Requirements

### Persistence

All tables prefixed with `durable_` to avoid conflicts. The schema has two
owners that must stay identical:

- **Server migrations** (`crates/server/migrations/`, starting at
  `002_durable_execution.sql`) create and evolve the tables for the Everruns
  control plane, versioned and immutable like every other server table.
- **The crate's own schema** (`crates/durable/schema/postgres.sql`), applied by
  `PostgresWorkflowEventStore::migrate`, lets a crates.io user of
  `everruns-durable` create the tables without the server. It is one idempotent
  script (CREATE ... IF NOT EXISTS, CREATE OR REPLACE for trigger functions and
  triggers, counter seeding with ON CONFLICT DO NOTHING) run in a single
  transaction under an advisory lock, and stamps a hash of the schema on a
  table comment so a database that already took this exact schema is only
  read, not locked or re-applied. `PostgresWorkflowEventStore::connect` wraps
  pool creation plus `migrate` for callers with no pool of their own. The script uses only built-in PostgreSQL 14+
  functions (its `uuidv7()` fallback avoids pgcrypto), and creates objects in
  the first `search_path` schema. Against a server-migrated database it changes
  nothing. Schema changes append idempotent statements rather than versioned
  migrations, because the crate cannot own a migration history in a database
  whose durable tables the server already manages.

`crates/durable/tests/schema_drift_test.rs` (durable CI shard) applies the crate
schema to a scratch PostgreSQL schema and compares tables, columns, defaults,
constraints, indexes, triggers, trigger-function bodies, sequences and seeded
counter rows with the server-migrated database, and checks that `migrate` is a
no-op over server migrations. A server migration that touches a durable table
therefore updates `schema/postgres.sql` in the same change. The one deliberate
difference is `durable_tool_results`, which belongs to the server's tool-call
idempotency storage and is never touched by the crate.

`everruns-durable` is part of the crates.io publish set. Its benchmark support
module and bench binaries sit behind the off-by-default `bench` feature, and
bench checkpoints are excluded from the package.

Workflow statuses: `pending`, `running`, `completed`, `failed`, `cancelled`, `continued_as_new`.

### History Bounds

The store keeps per-workflow history bounded without replaying it:
`count_events()` and `count_events_after()` let a caller reject an oversized
history before loading it, and `continue_as_new()` rolls a workflow over into a
new one, archiving old events and marking the old workflow `continued_as_new`
with a reference to the new ID (`continued_as_new_id` column). See
`crates/durable/src/persistence/store.rs`.

### Task Claiming

Workers claim tasks partitioned by `activity_type`, from one task queue at a time (`ActivityOptions::queue`; the default queue unless a caller names one, as a PostgreSQL framework backend does). See `crates/durable/src/persistence/store.rs` for implementation.

The in-memory store is the test double for PostgreSQL and must behave the same
wherever a caller can tell: only a registered, non-draining worker claims;
claims go by priority, then visibility time; retries wait out their backoff;
reclaim honors the stale threshold; and a first claim records
`ActivityStarted`. `crates/durable/tests/store_conformance_test.rs` runs one
set of cases against both stores, so a difference fails CI instead of hiding
behind green unit tests.

### Task Notifications

Push-based via gRPC streaming (`SubscribeTaskNotifications`), backed by NATS when available and PostgreSQL `NOTIFY` otherwise. The standalone worker subscribes at startup (`crates/worker/src/task_wakeup.rs`) and every notification cuts its poll backoff short. Polling stays as the fallback, so a lost notification or a dropped stream costs at most one backoff interval (`WORKER_POLL_BACKOFF_MAX_MS`), and the worker resubscribes with a 1s to 30s backoff.

Session actors ride the same channel with their own payloads: `session_actor` when an actor becomes leasable and `session_actor_cancel` when a cancel lands on a leased one. The worker subscribes to each under its own subscriber id (the control plane keeps one subscription per id): the first wakes the actor lease loop, the second makes every running lease renew at once, which is how its holder learns of the cancel. Under PostgreSQL a trigger on `session_actors` notifies on commit; under NATS the server's turn backend publishes after the intake transaction commits. The in-process DEV_MODE worker has no channel: the turn backend wakes its lease loop and its running leases directly, also after the commit (`ActorWakeups`). A release that leaves the actor runnable wakes the releasing worker's own loop once the turn's slot is free. A lease that expires sends nothing, so the actor loop's poll backoff is capped at 1 s (`ACTOR_POLL_BACKOFF_MAX`), which bounds how long past its TTL a dead holder's turn waits.

History: before session actors, every turn phase (`process_input`, `reason`, `act`) was its own queued task, and the measurements of that path explain choices the actor runtime kept. Before the worker consumed these notifications, an idle worker sat in a backoff of up to 5 s, so a new message waited up to 5 s to start and each tool call paid that wait twice; pickup fell from about 1.5 s to about 70 ms. `crates/server/tests/workflow_test/latency.rs` guards the pickup end to end.

Per-phase host setup reads the session, harness, and agent several times over, and each read from a gRPC worker is a control-plane round trip. With the production database a few milliseconds away, that setup took about 600 ms of each ~700 ms phase hand-off, against about 130 ms locally. The worker memoizes those reads for the length of a phase's setup (`crates/worker/src/phase_reads.rs`, logged as `phase setup` with `setup_ms`) and keeps them per turn (`crates/worker/src/turn_reads.rs`), so a turn's configuration is pinned for the turn: an edit made mid-turn applies from the next turn. Conversation history is not cached and is read fresh by every reason. The input step's events join the first reason's ordered write-behind queue instead of being stored before the model call.

Two smaller costs sat on every turn. The server's database pools pinged each idle connection before handing it out, which doubled the round trips of every query outside a transaction. They now ping only connections idle for 30 s or more (`crates/server/src/storage/repositories/mod.rs`). And the first text delta waited a full 100 ms batch after the model's first token, because the batch clock started with the stream; the first token now goes out at once (`crates/core/src/engine/execution/reason.rs`). In the UI, the streaming typewriter reveals each chunk over about one batch (`apps/ui/src/components/streaming-message.tsx`) instead of at a fixed rate that left the text several hundred milliseconds behind the model.

Operational contract:

- NATS is the preferred backend when configured
- PostgreSQL fallback must use a direct session-scoped listener connection, not an ordinary pooled query connection
- deployments that pool `DATABASE_URL` for regular queries should provide `DATABASE_UNPOOLED_URL` for PostgreSQL listener traffic

Reasoning:

- task notifications are latency-sensitive, but correctness matters more than transport choice
- PostgreSQL `LISTEN/NOTIFY` through a pooler/proxy can fail intermittently and surface as protocol errors on unrelated query traffic
- failing fast on an invalid listener URL is better than allowing a deployment that corrupts the control-plane's normal database interactions

### Worker Concurrency & Sizing

External/gRPC workers and the in-process worker both run `crates/worker/src/unified_worker.rs`, which exposes three independent knobs so concurrency, claim batch size, and idle polling can be tuned separately. The session actor loop and the queue loop share them: a leased actor takes an execution slot like a claimed task, `CLAIM_BATCH_SIZE` also bounds one lease call, and a turn waiting on the model hands its slot back:

| Setting | Env var | Default | Purpose |
| --- | --- | --- | --- |
| Execution concurrency | `MAX_CONCURRENT_TASKS` | `50` | Tasks a worker runs at once / advertised capacity. |
| Model waits | `MAX_CONCURRENT_MODEL_WAITS` | `200` | Turns that hand their slot back while waiting on the model; `0` turns it off. |
| Claim batch size | `CLAIM_BATCH_SIZE` | `50` (clamped to concurrency) | Upper bound on `claim_task max_tasks`, regardless of free slots. |
| Fallback poll interval | `WORKER_POLL_INTERVAL_MS` | `100` | Base interval when push notifications are unavailable. |
| Fallback poll backoff cap | `WORKER_POLL_BACKOFF_MAX_MS` | `5000` | Idle polling backs off exponentially from the base up to this cap. |
| Shutdown grace | `WORKER_SHUTDOWN_GRACE_SECS` | `25` | How long a stopping worker lets in-flight tasks finish before it aborts them. Keep it under the orchestrator's kill timeout. |
| Actor stall bound | `WORKER_ACTOR_STALL_TIMEOUT_SECS` | `300` | How long a leased session may sit before or after its turn writing nothing before the worker lets its lease expire ([stall watchdog](#session-actors-turn-execution)). |
| Control-plane call deadline | `WORKER_GRPC_REQUEST_TIMEOUT_SECS` | `120` | Deadline of each worker call to the control plane; past it the call fails as a retryable error. |

Guidance:

- **The default concurrency is intentionally modest (50, matching the default DB pool).** Historically it was `1000`, so a few replicas advertised thousands of slots and a single idle poll issued `claim_task max_tasks=1000`; when the DB was already slow this amplified pool pressure into acquire timeouts (EVE-606). 1000-way concurrency is now opt-in via `MAX_CONCURRENT_TASKS`.
- **A slot is for work, not for waiting on the model.** A turn's reason spends most of its time on the model's stream, doing little DB or CPU work, so it hands its slot back for that wait (`crates/core/src/engine/execution/model_wait.rs`). Without this, `MAX_CONCURRENT_TASKS` capped how many turns could talk to a model at once: on prod, 3 workers x 20 slots added about 2 s of queueing at 100 concurrent sessions (load test, 2026-10-08). Waits have their own cap because each open stream still costs memory, a connection and batched delta writes; past it a wait keeps its slot.
- **Raising concurrency does not raise claim cost:** the claim batch is bounded by `CLAIM_BATCH_SIZE` independently, so a high-concurrency worker still claims in modest batches.
- **Pool sizing, not pool inflation, is the lever.** Size against `pg_max_connections / replicas − margin`; do not raise the pool to mask worker over-claiming. Worker-side concurrency and claim batch should be tuned down first for small instances.
- **Size against the total, not `DATABASE_POOL_MAX` alone (EVE-1081).** A control-plane instance opens two pools: the request pool (`DATABASE_POOL_MAX`) and a smaller background pool for sweeps (`DATABASE_BACKGROUND_POOL_MAX`, default 8). The number to compare against `pg_max_connections / replicas` is their sum — `DatabasePoolConfig::total_max_connections()` in `crates/server/src/storage/repositories/mod.rs`, which is also what the startup sizing warning uses. An instance previously tuned so `DATABASE_POOL_MAX` exactly filled the budget will over-subscribe Postgres by the background pool's size per replica and start getting connection refusals, so reduce `DATABASE_POOL_MAX` by that much when upgrading. Setting `DATABASE_BACKGROUND_POOL_MAX=0` restores the single-pool budget exactly, at the cost of the isolation it buys.
- Effective worker settings (id, concurrency, claim batch, poll interval/backoff) are logged at worker init for troubleshooting.

### Worker Drain and Shutdown

A worker is drained two ways, and both end in the same registry state (`status = 'draining'`), so the queue hands it no new claims:

- **Operator drain.** `POST /v1/durable/workers/{id}/drain` (and `/resume`) flips the registry row. The worker learns it from its next heartbeat, whose reply carries `draining`, and stops claiming; a heartbeat that reports the drain lifted wakes the poll loop at once. Running tasks are left alone.
- **Shutdown.** SIGTERM or Ctrl+C stops the poll loop and the actor lease loop, and the worker drains itself (`DrainDurableWorker`). It keeps heartbeating while in-flight tasks and running turns finish, waits up to `WORKER_SHUTDOWN_GRACE_SECS`, aborts what is left, then deregisters, which returns the aborted tasks to the queue. An aborted turn keeps its actor lease until it expires (10 s); another worker then leases the session and resumes the turn from the log. A second signal exits at once.

A drained worker leases no actors either; turns it already runs go on.

The grace period must fit inside the orchestrator's kill timeout (Kubernetes `terminationGracePeriodSeconds`, Compose `stop_grace_period`), or the worker is killed mid-drain and its tasks wait for claim expiry instead. See `crates/worker/src/unified_worker.rs` and `crates/worker/src/app_builder.rs`.

### Generic Queue (Standalone Tasks)

The task queue supports standalone tasks that run independently of any workflow. `TaskDefinition.workflow_id` is `Option<Uuid>`, when `None`, the task is a standalone queue entry.

- **Enqueue**: `POST /v1/durable/tasks` with `activity_type`, `input`, and optional retry/priority config
- **Processing**: Workers claim and execute standalone tasks identically to workflow tasks
- **Event recording**: Standalone tasks skip workflow event recording (ActivityStarted/Completed/Failed are no-ops)
- **Limits**: Global cap of 10,000 pending standalone tasks (vs 100 per-workflow)
- **DLQ**: Failed standalone tasks go to dead letter queue with `workflow_id = NULL`
- **Dashboard**: UI shows "standalone" badge for tasks not linked to a workflow

See `crates/durable/src/persistence/store.rs` for `TaskDefinition` and `crates/server/src/api/durable.rs` for the enqueue endpoint.

### Reliability

1. **RetryPolicy** - Exponential backoff with jitter
   - `max_attempts`, `initial_interval`, `max_interval`
   - `backoff_coefficient`, `jitter`
   - `non_retryable_errors` list

2. **CircuitBreaker** - Distributed state via database
   - States: Closed → Open → HalfOpen → Closed
   - `failure_threshold`, `success_threshold`, `reset_timeout`

3. **Timeouts**
   - `schedule_to_start_timeout` - Max wait in queue
   - `start_to_close_timeout` - Max execution time
   - `heartbeat_timeout` - Liveness detection

4. **Dead Letter Queue** - Failed tasks preserved for debugging/replay

### Failed-task terminalization

Deterministic activity failures bypass the normal retry policy. When a
workflow-bound activity reaches the DLQ, the worker and stale-task reaper race
an atomic `running → failed` workflow transition; only the winner records
`workflow.failed`. A failed turn is the actor runtime's to record
([Session actors](#session-actors-turn-execution)), not the queue's.

A turn whose session was deleted (before it was claimed or mid-step) is the
exception: there is nothing left to report to. The control plane answers its
reads and event writes with "session not found" naming the session (a session
FK violation on event insert included), on both the gRPC and in-process worker
transports, and the turn driver dead-letters the step without retry, skips the
`turn.failed` lifecycle, and logs at info rather than as a task failure. See
`fail_step` in `crates/durable-engine/src/turn_driver.rs`; session actors
handle the same case on their lease
([Session actors](#session-actors-turn-execution)).

### Stale-task reaping

Reclaim and the settling of dead and sealed tasks are engine logic in
`everruns_durable::maintenance` (`reap_stale_tasks`, `StaleTaskReaper`), not in
the host. One pass reclaims stale claims, then for each dead task records
`ActivityFailed` and fails its workflow with the `try_fail_workflow`
compare-and-set; for each sealed task it records `ActivityFailed` and marks the
workflow failed (`task sealed: no_progress (N recoveries)`). The host plugs in
a `ReapHandler` for what that means in its domain, and only the reaper that
won the workflow transition calls it, so replicas racing the same reap notify
once. The queue carries no turns, so the server's handler
(`crates/server/src/background/durable_reaper.rs`) only reports reap passes
that fail. The server reaps every 10 s with a
30 s stale threshold. `WorkerPool` runs the same pass with a no-op handler, and
`WorkerPoolConfig::without_stale_reclaim` turns its loop off for a host that
already runs a reaper with a real handler.

### Session actors (turn execution)

Turns run on the actor model the framework already runs
([Execution Backends](../framework/execution-backends.md#the-default-sessions-as-actors)):
each session is an actor, a worker leases a session and runs its whole turn,
and the session event log is the only turn state, so resume is a fold of the
log. The control plane keeps the `session_actors` runnable index
([migrations 215](../../crates/server/migrations/legacy/215_session_actors.sql) and
[20261012014120](../../crates/server/migrations/20261012014120_session_actor_inbox_cursor.sql),
[repository](../../crates/server/src/storage/repositories/session_actors/mod.rs))
and serves four worker-link calls, `LeaseActors`, `RenewActorLease`,
`ReleaseActor` and `ReadActorJournal`, over gRPC and, in DEV_MODE, through
`DirectWorkerAdapters` on the same repository. Workers still never touch the
database. The server's turn backend is
[`ActorTurnBackend`](../../crates/server/src/turns/actor_backend.rs) in
production and DEV_MODE alike; the worker side is
[`actor_runtime`](../../crates/worker/src/actor_runtime/mod.rs).

- **Intake marks the actor runnable in the input's transaction.** Starting a
  turn is an upsert the caller runs in the transaction that persists the
  input, so the input and the marker commit together and a rolled-back message
  starts nothing. Intake that is not a transactional command (a person's
  message, a scheduled message, a wake) opens that transaction itself
  ([`store_input_and_start`](../../crates/server/src/turns/mod.rs)): a send
  that fails stored nothing, and a retry with the same `client_message_id`
  that finds the message stored but not marked marks it. `inbox_seq` (the newest input's event sequence) only moves
  forward. A stored message also records `next_input_seq`, the input the next
  fresh turn starts from, when the previous one was consumed, so queued
  messages start turns in arrival order and history written without a mark (a
  channel's backfill) never starts one. A resolution (tool results, an answer,
  an approval decision, a consent) marks the inbox at the log head instead: it
  continues the open turn.
- **One turn per lease.** The worker folds the log's tail from the latest
  `turn.started`. An open turn is resumed: unfinished calls rerun when their
  tool declares that safe (pure, idempotent, client-side, approval-gated,
  `ask_user`, or a synthetic person-facing call) or never started, and are
  otherwise settled as `interrupted`. An at-most-once call does not run until
  its `tool.started` is stored (the worker's write-behind queue does not hold
  it), so a log without one proves the call never ran; an open turn whose answer is already in the log is completed
  without another model call. With no open turn, the first unconsumed input
  starts a fresh turn through core's `turn_entry_plan`, the entry step the
  in-process runtime uses. A closed latest turn whose terminal event has no
  `session.idled` after it lost its holder between the two: the next holder
  idles the session (the log keeps that idempotent) and treats the turn's
  input as consumed even when no renew committed it, so it never runs twice.
- **The inbox is the log; the cursor is `consumed_seq`.** A turn input is an
  `input.message` without a turn id; everything a turn or a resolution writes
  carries the turn's id. Before each reason (and when a reason would finish)
  the turn renews, which reports the inbox head, and input past the cursor
  becomes steering for that reason. Renew and release commit the cursor, so a
  holder that dies mid-turn leaves its successor only the input the model
  never saw. Input that reaches a turn parked on a person resolves the pause
  as intake does for a waiting session: the pending calls are settled as
  cancelled and the turn goes on to the input.
- **Lease, fence, expiry.** A lease call takes runnable actors whose lease is
  free or expired, oldest first, with `FOR UPDATE SKIP LOCKED`, so concurrent
  workers take disjoint sets without waiting. Every lease raises the fence.
  The holder renews every third of the 10-second TTL. A holder whose renew
  finds the fence moved, or that renewed nothing for a whole TTL, has lost the
  lease: it stops the turn, drops its unwritten events and releases nothing.
  A retryable failure (a store or transport error) runs the turn again from
  the log on the same lease, after a doubling backoff (default 5 retries from
  1 s, about 30 s), once the failed run's queued writes are dropped or stored
  so none lands behind the retry under the same fence. Only when the retries
  are spent does the lease expire, with the TTL as the backoff before another
  worker resumes from the log. That includes a control-plane read or write the reason step needs (a pool
  timeout loading the compaction checkpoint): the reason step returns it
  instead of reporting it as a model failure, so it never becomes a
  `turn.failed` with a provider-error message.
  Expiry is database time.
- **A stalled holder lets its lease go.** The renewer extends the lease
  whatever the turn does, so a holder stuck where it writes nothing (setup
  before the turn is driven, or the flush and checkpoint after it) once kept a
  session `active` with no events for two days: the lease never expired, so
  neither resume nor the poison guard ran, and the idle sandbox sweep skipped
  the session as active. A stall watchdog
  ([`lease.rs`](../../crates/core/src/host/session_actor/lease.rs)) bounds
  those phases (`WORKER_ACTOR_STALL_TIMEOUT_SECS`, default 5 minutes): past
  the bound the renewer stops, the lease is lost and the stuck phase is
  dropped, so the lease expires and the next one resumes from the log, each
  stalled lease counting toward the poison guard. A driven turn is not bounded
  this way; a long model stream or tool call is progress with its own
  timeouts. Every worker control-plane call also has a deadline
  (`WORKER_GRPC_REQUEST_TIMEOUT_SECS`, default 120 s,
  [`connect/mod.rs`](../../crates/worker/src/grpc_adapters/connect/mod.rs)), so a call
  nobody answers fails as a retryable store error instead of hanging the turn.
  The task subscription stream has its own channel and no deadline. Not yet
  covered: a sweep for an `active` session whose actor is neither leased nor
  runnable.
- **Every log append is fenced.** Stopping on a lost renew leaves a window: a
  write already in flight when the lease ran out could land after another
  holder took the session. So every event a leased turn appends carries its
  holder and fence (`actor_fence` on `EmitEvent`/`EmitEventStream`,
  `WorkerAdapters::emit_events_fenced` on gRPC and `DirectWorkerAdapters`),
  and the control plane stores a batch only while that is still the holder's
  unexpired lease ([`create_events_fenced`](../../crates/server/src/storage/repositories/events/mod.rs)).
  The check runs in the insert's transaction, after the session's
  `event_sequences` lock (the order message intake takes them in) and under
  `FOR SHARE` on the actor row, so a new lease cannot take the session until
  the append committed and its journal read then sees it. A refused append
  stores nothing and answers `FAILED_PRECONDITION` "actor lease lost"; the
  worker maps it to a distinct lease-lost error that flips the lease lost, and
  the turn stops exactly as on a lost renew. Ephemeral deltas that skip
  PostgreSQL under NATS are not log appends and go unfenced. Writes the server
  makes for a session (intake, resolutions, a withdrawn cancel) are not
  leased and carry no fence.
- **A deleted session stops its turn quietly.** The actor row goes with the
  session (FK cascade), so the holder's next renew finds no row at all, which
  the control plane answers as "session not found" naming the session rather
  than a lost lease (`NotFound` over gRPC, `SessionNotFound` in process). An
  append that gets there first fails the session FK and is answered the same
  way, fenced or not. A turn that instead fails on a read (its input message
  or conversation already gone) asks the renew before it counts the failure
  as its own. The holder stops the turn as on a lost lease, at info level: no
  `turn.failed` (there is nowhere to store it) and nothing to release.
- **Release never loses input.** Release clears the runnable marker only when
  the holder consumed input up to `inbox_seq`. A message that lands as a turn
  finishes moves `inbox_seq` past it, so the actor stays runnable and the next
  lease runs the next turn.
- **Cancel has one writer.** One statement under the row lock decides: on an
  actor a live holder leases, the request is recorded on the row and the
  holder ends the turn and writes `turn.cancelled`, the person's request and
  `session.idled` (the server writes nothing); on an actor that only waits to
  run, or whose holder's lease expired, the actor is withdrawn and the server
  records the cancel as before. Lease and renew report a pending cancel, and
  release clears it only once the holder was told, so a cancel that lands
  after the last renew reaches the next holder. Either way the turn ends with
  exactly one terminal event. A cancel targets the turn it was requested for:
  the request records the inbox position at that moment, a holder applies it
  only to a turn whose input is at or before it, and recording it consumes
  input only up to it. A holder that recorded `turn.cancelled` and died before
  releasing leaves the cancel on the row, and the next holder runs the
  follow-up message's turn instead of cancelling it (chaos harness). A cancel
  the holder records is answered only once the holder ended the turn (bounded
  by two lease lives); inside a command transaction that wait runs after the
  commit, when the holder can first see the request, still before the command
  answers. Past the bound the answer means "acknowledged": the request stays
  on the row for the holder, or the next one, to record. A backend error does
  not transfer event ownership to the API: session and channel cancellation
  return the error without synthesizing terminal events or idling the session.
  Otherwise a worker completing while the API waits for a connection can leave
  both completion and cancellation in the same turn's log.
- **Poison guard.** `attempts` counts leases since the last release. A lease
  that finds more than `DURABLE_NO_PROGRESS_SEAL_THRESHOLD` (default 3) seals
  the turn instead of running it: a user-facing processing error,
  `turn.sealed { no_progress }`, `session.idled`, and the pending input
  consumed. A turn that kills its worker cannot loop forever, and the session
  is usable afterwards: the release resets the count, and the next message
  starts a fresh turn. It counts holder deaths, not failures to reach the
  control plane: those are retried on the lease (above), so a database blip
  no longer spends leases. Before EVE-1301 every lease that ended on a
  retryable error counted, and a pool exhaustion followed by a server kill
  sealed healthy turns after three of them (chaos harness). A turn whose
  failures outlast a lease's retries still lets the lease expire and counts
  an attempt, so the seal stays the bound. Releasing instead of expiring was
  rejected: release resets the count, so such a turn would never be sealed,
  and it leaves an open turn with no pending input unrunnable. The reasoning
  is next to the code in
  [`turn.rs`](../../crates/core/src/host/session_actor/turn.rs).
- **Per-org fairness order.** A lease call counts each org's held leases and
  takes actors of orgs under the cap (default 32) first, oldest first; an
  org's actors past it fill only the room left. It orders, it never holds
  work back: a lease runs a whole turn, model and tool waits included, so a
  hard cap of 32 was a fleet-wide cap on one org's concurrent turns. The
  chaos harness (one org, 50 live sessions) sat at exactly 32 leases with
  worker slots free, and pickup rose from 38 ms to 1.3 s at p50 (p95 3.2 s);
  ordering instead of capping brought it to 102 ms (p95 604 ms), with 50
  leases held at once. Admission
  stays with `ORG_MAX_ACTIVE_TURNS` when the input is accepted.
- **Resolutions carry the open turn's id.** Tool results, answers, approval
  decisions, MCP URL consents, the tool-result timeout, a message's
  cancellation of pending client calls and the server-recorded cancel all
  write under the session's open turn (`turns::open_turn_context`, read from
  its `turn.started`). The worker's fold counts a `tool.completed` only under
  the open turn's id, and a turn input is one without a turn id, so a
  resolution written under another id would be lost.
- **The worker host is the one queued phases used.** `WorkerRuntimeHost` with
  its per-phase setup reads, the per-turn read cache, write-behind events and
  the model-wait slot release, built once per turn and re-armed at each phase
  boundary. Stream heartbeats are not needed: the lease renewer keeps a turn
  alive.

**Deploy transition.** [Migration 20261012014121](../../crates/server/migrations/20261012014121_session_actor_cutover.sql)
moved in-flight work across: every session that was active or had a pending or
running `turn_workflow` got an actor row, runnable, with its cursor at the
log's last turn boundary, so a turn the old path left open was resumed from
the log by the first worker that leased it, and the old path's pending and
claimed `process_input`, `reason` and `act` tasks and pending and running
`turn_workflow` runs were cancelled. A temporary sweep converted old-path work
that servers and workers of the previous release created during the rolling
deploy; step 7 deleted it with the per-step path, and
[migration 20261012061054](../../crates/server/migrations/20261012061054_drop_turn_workflow_signals.sql)
dropped the turn workflow's signal table and its deduplicated-enqueue index.
Historic `turn_workflow` instance rows stay as ordinary terminal workflows.

### Forward-progress guard and Sealed terminal (EVE-534)

Turns have the actor poison guard ([Session actors](#session-actors-turn-execution)),
which emits `turn.sealed { no_progress }`. This store-level guard covers the
queued tasks that are not turns.

`RetryPolicy.max_attempts` bounds *how many times* a task may run, but a task
that crashes and is reclaimed repeatedly without ever advancing can still loop
until it incidentally hits max_attempts. The forward-progress guard adds a
poison-task defense tied to *progress* and a deliberate **Sealed** terminal.

- **Progress token**: a per-turn, monotonically advancing marker derived from
  durably-recorded facts so it is stable under replay and cannot be advanced by
  a non-progressing retry. It is the highest `durable_workflow_events.sequence_num`
  for the task's workflow (encoded so "no events yet" = 0). Each stale reclaim
  records the token observed for the task (`durable_task_queue.progress_token`).
- **No-progress detection**: on each reclaim, the store compares the current
  token to the previously recorded one. If it did not advance, the per-task
  `no_progress_count` is incremented; any advance resets it to 0.
- **Sealing**: when `no_progress_count` reaches `N` (default 3, configurable via
  `DURABLE_NO_PROGRESS_SEAL_THRESHOLD`), the reclaim path marks the task `dead`
  (→ DLQ) instead of returning it to `pending`. This stops scheduling and makes
  the task **non-retryable**; the reaper marks its workflow failed and calls
  the host's `ReapHandler::task_sealed`.
  See `crates/durable/src/persistence/store.rs` (`SealedTaskInfo`, `ReclaimResult`)
  and `reclaim_stale_tasks` in the Postgres/in-memory stores.

The turn-level outcome of a seal is `everruns_core::engine::TurnPlan::Terminal`
with `everruns_core::TurnStopReason::Sealed`, distinct from `Success` and
`Failed`. The `turn.sealed` reason string is `no_progress`; the event contract
also reserves `budget`, which no live path emits.

Operator follow-up (not yet implemented): a UI to inspect and replay sealed
tasks from the DLQ. Sealed tasks already carry the seal reason and counters via
`SealedTaskInfo` and persist in `durable_dead_letter_queue`.

### Backpressure

- **Worker-side**: High/low watermarks based on load ratio
- **System-wide**: Queue depth relative to total capacity
- Workers report `accepting_tasks` status in heartbeats

### Observability

- OpenTelemetry spans for workflows, activities, task operations
- Semantic conventions: `durable.workflow.*`, `durable.activity.*`, `durable.worker.*`
- Metrics: workflows started/completed/failed, activity durations, queue depth
- Admin API: `/api/durable/workers`, `/api/durable/workflows`, `/api/durable/dlq`
- Metrics time series: `/v1/durable/metrics/timeseries`, server-side ring buffer (360 points, 10s resolution)

### Metrics Dashboard

The durable landing page is [Monitoring](monitoring.md), showing session-actor
execution, worker ownership, and lease health. Generic queue/workflow metrics
remain available through the durable metrics API and Prometheus. They describe
background activities, not platform turn throughput. Source contracts and the
ring-buffer collector remain in `crates/server/src/api/durable.rs`.

### System Health Counters

`get_system_health` (feeding `/v1/durable/health`, the ~10s metrics sampler, and the durable SSE stream) splits its fields by cost:

- **Live gauges**: pending/claimed tasks, running/pending workflows, workers, capacity/load, DLQ size, are queried directly. They are bounded by current work and backed by partial indexes, so they stay cheap.
- **Cumulative totals**: completed/failed/started for tasks and workflows (the Prometheus-style monotonic counters), are read from `durable_stat_counters`, a tiny table maintained incrementally by AFTER triggers on `durable_task_queue` and `durable_workflow_instances` (migration `082`). The history tables are never pruned in production, so these grew without bound; counting them per call forced repeated full scans (~127MB heap at ~156k rows) and could starve the DB pool.

Each counter uses delta accounting (increment on entering a counted state, decrement on leaving or deletion), so it is always exactly equal to the `COUNT(*)` it replaces, there is no staleness window. Reads are O(1) point lookups independent of history size. The per-transition trigger writes touch one shared counter row per metric; this is acceptable at the engine's task throughput, and can be sharded later if a single counter row becomes a write hotspot.

### Worker Heartbeat & Stale Worker Handling

Workers heartbeat every 5s. `WORKER_HEARTBEAT_TIMEOUT_SECS` (60s, in `crates/durable/src/persistence/store.rs`) is the single source of truth for stale detection, used by `get_system_health`, `list_workers`, and `reclaim_stale_tasks`.

## Decisions

### Partitioning Strategy

**Decision**: No custom partitioning in v1. PostgreSQL with proper indexes and `SKIP LOCKED` handles the target load. Activity-type-based claiming naturally partitions work.

### Task Ownership Verification

**Decision**: Verify ownership on task completion. Prevents duplicate activity scheduling when a worker's heartbeat times out and task is reclaimed. Late-finishing worker gets `TaskNotOwned` error.

### Task Heartbeat Cancellation

**Decision**: The task heartbeat reports two stop reasons: a rejected heartbeat (`accepted = false`) is ownership loss (the task was reclaimed or finished elsewhere), and an accepted one with `should_cancel` means its workflow was cancelled. The worker stops renewing the claim on either; its non-turn activities are short control-plane calls, so a running one finishes and its completion is refused (`TaskNotOwned`) rather than being cut off mid-write (`crates/worker/src/task_driver.rs`). Turn cancellation travels on the session actor's lease instead ([Session actors](#session-actors-turn-execution)).

### Worker Communication

**Decision**: Workers communicate via gRPC only, no direct database access. Clear separation between control-plane (owns state) and workers (stateless executors).

**Authentication**: Two layered mechanisms (see `knowledge/security/threat-model.md` TM-DURABLE-002):
1. Bearer token (`WORKER_GRPC_AUTH_TOKEN`) -- required in production
2. Mutual TLS (`WORKER_GRPC_TLS_*`) -- optional transport encryption

Both the durable/task client and the normal turn client, including its raw blob
fetcher, use the shared TLS configuration. TLS retains the endpoint hostname for
certificate verification and bypasses the plaintext DNS replica balancer. The
turn client's deadline bounds connection establishment and TLS handshake as
well as requests; TLS failures never select a plaintext fallback. Unset or empty
CA configuration preserves plaintext compatibility. See the
[turn connection implementation](../../crates/worker/src/grpc_adapters/connect/mod.rs)
and [environment variables](../../docs/sre/environment-variables.md#worker_grpc_tls_ca_cert).

This verifies transport peers, not an assignment or tenant. A shared worker's
bearer still grants cross-org authority; a per-executor application identity and
assignment admission must be enforced separately before scoped issuance.

The unactivated [journal capability verifier](../../crates/server/src/worker_link/grpc_service/journal_capability/mod.rs)
is the next component: it binds an exact TLS-verified client leaf certificate to
a registered executor and requires an opaque proof whose stored digest selects
that executor's journal grant. It creates a distinct verified context, strips the
proof header, and rechecks identity, capability, grant and lease in one database
statement at each read admission. Caller identity headers and legacy bearers
cannot substitute for either proof. Middleware accepts only the exact journal
RPC wire route; the RPC and raw blob gates also refuse other operations under
the verified context.

No production listener calls the verifier and no production registration or
capability issuer exists. The migration creates empty records. Identity and
capability bindings cannot be retargeted, extended, un-revoked or recycled;
parent purge leaves unusable digest tombstones. Trusted issuance still needs
certificate enrollment/validity policy, secure proof delivery, retention and
resource budgets, and complete underlying user/custom-policy provenance before
activation. A qualified listener must supply actual verified mTLS connection
evidence; a TLS-terminating proxy's headers are insufficient. Transport peers
sharing one process's identities or proofs still share its authority.

The source-only [lifecycle admission](../../crates/server/src/storage/repositories/session_actors/capability_admission.rs)
also requires explicit supporting grants. These are revocation prerequisites,
not permission to read a journal or use a resource. This first supported class
is session-scoped service grants to organization-owned service resources; personal, agent-consumer and wider
scopes cannot be enrolled without verified invocation provenance. No issuer
selects or registers dependencies in production. A future trusted issuer must
prove journal authority independently and register the complete dependency set;
one arbitrary live resource grant is insufficient evidence of that authority.

[Database lifecycle triggers](../../crates/server/migrations/20261028115847_worker_grant_lifecycle_revocation.sql)
pin database-owned versions at assignment creation and dependency registration.
Resource/grant edits or deletion, session identity or archive changes, configured
agent identity or status changes, cancellation, lease release/reassignment, and
renewing an expired lease revoke old grants atomically. Forward renewal of a live
lease and session title changes preserve it. Versions remain monotonic when old
values are restored; deleted source references become immutable tombstones, so
recreating identifiers cannot revive a proof. An elapsed supporting-grant expiry
is refused online; extending it requires a fresh assignment. Existing grants
without captured origins or dependencies fail closed rather than being backfilled.

Registration locks source records before the assignment. Creation serializes
against lifecycle changes. Online admission checks current versions as well as
revocation, so a racing uncommitted registration cannot escape by missing an
update notification. Scoped reads use one
[read-only snapshot](../../crates/server/src/storage/repositories/session_actors/journal_snapshot.rs)
for the protection floor, authority, journal page and head metadata. A revocation
committed before that snapshot is denied. An earlier snapshot may finish afterward,
but cannot fetch later-committed journal events or head metadata. Retained contexts,
subsequent pages and new connections all recheck. This does not retract delivered
data or synchronously stop in-flight reads. Snapshot lifetime and transaction round
trips still need production-scale budget qualification before activation.

The source-only [worker ingress floor](../../crates/server/migrations/20261028140332_worker_ingress_protection_floor.sql)
closes canonical legacy wire access before protected capability enrollment.
It is empty on upgrade and has no production writer or activation API. A
capability cannot be enrolled without the supported floor, and verified reads
without it are refused. This is a database-wide protocol requirement: legacy
global work claims, org-wide credential lookups and signed object URLs cannot
be safely filtered by an assignment selector. Mixed legacy/scoped execution in
one database is unsupported. Unprotected deployments retain legacy behavior;
a missing schema or unavailable database never implies an absent floor.

Every generated RPC and raw byte route checks the floor without caching a
positive legacy decision. Legacy, authentication-disabled development and
selector-only journal assertions are all refused once protection is required.
A qualified verified journal reader retains only its existing journal operation;
an unknown future floor version refuses it too. The floor cannot be removed or
downgraded, including after grants expire or their parent data is deleted.
[Wire admission](../../crates/server/src/worker_link/grpc_service/authority/mod.rs)
does not derive scope from caller org/session identifiers or identity headers.

Event streams admitted earlier recheck before publishing/storing their collected
batch; notification streams recheck before yielding each item; uploads recheck
after body receipt before decoding/storage. New uploads are refused before body extraction. Read links authenticate their
signature before any database access, then check the floor before object lookup;
journal-only contexts remain refused before parsing either byte operation. This is admission enforcement,
not a transaction coupling every operation to floor activation. Previously
admitted finite operations or byte downloads may finish; prior disclosures
cannot be retracted. The legacy event-stream collector still needs bounded
memory qualification before any scoped emit operation is supported.

This floor protects the canonical RPC/HTTP wire boundary, not arbitrary server
code with database access. Default development storage initialization refuses
protected in-process hosting. Already captured direct stores, custom embedders,
alternate listeners and old replicas are not made safe by that startup check.
No production listener installs the scoped verifier and no issuer exists;
protected execution is not a runnable production mode at this stage.

Before activation: migrate empty metadata first, upgrade and independently verify
every serving replica, drain and stop all legacy workers/streams/transfers, remove
direct/in-process and unqualified alternate worker hosting, implement and qualify
the required scoped operations and complete authority provenance, and compose the
verified listener. A floor record cannot enforce itself on an old binary that
ignores it; deployment version enforcement is still required. Do not roll back
to pre-floor binaries or reopen legacy access after protection is required.
Production enrollment, trust, network, credential or security-setting actions
remain separate approved rollout work. No such action is performed here.

**Design decision**: Workers are intentionally cross-org. Org-scoping is enforced at the HTTP API layer, not the gRPC transport.
