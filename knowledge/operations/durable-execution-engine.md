---
type: Specification
title: "Durable Execution Engine Specification"
description: "PostgreSQL-backed durable workflow engine."
tags:
  - everruns
  - operations
---
# Durable Execution Engine Specification

## Abstract

Custom PostgreSQL-backed durable execution engine for workflow orchestration with automatic retries, circuit breakers, and distributed task execution.

`everruns-durable` is a generic engine: workflows, activities, tasks, signals,
and schedules, with no `everruns-*` dependency (enforced by
`scripts/lib/check-durable-isolation.sh`). Agent semantics live above it. Turns
use `everruns-durable-engine`: its `TurnTaskDriver` runs each claimed turn step
over `everruns_core::engine::TurnExecution`, restored from and checkpointed to
the task input between steps; its `durable_turn` module owns the
turn-level conventions (the `user_message` signal, idempotent waiting-turn
resolution tasks via `ActivityOptions::dedupe_by_activity_id`); and the server
turns a sealed task into `turn.sealed`. The durable crate owns persistence,
retries, and activity scheduling. The worker reaches these through
`everruns-durable-engine`, which is also published as the facade's experimental
durable turn backend ([Execution Backends](../framework/execution-backends.md)).
The worker owns no database driver; database connection construction
belongs only to server and durable, enforced by
[`check-database-driver-isolation.sh`](../../scripts/lib/check-database-driver-isolation.sh).

## Goals

1. **Self-contained** - `everruns-durable` crate with no Temporal dependencies
2. **PostgreSQL-only** - No additional infrastructure required
3. **Testable** - Unit tests, integration tests, load/stress tests
4. **Reliable** - Retries, circuit breakers, timeouts, dead letter queues
5. **Simple** - Event-sourced workflows with explicit state machines
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
│  │   Workflow   │  │   Activity   │  │   Worker     │  │   Scheduler   │   │
│  │   Engine     │  │   Executor   │  │   Pool       │  │               │   │
│  └──────┬───────┘  └──────┬───────┘  └──────┬───────┘  └───────┬───────┘   │
│         │                 │                 │                   │           │
│  ┌──────┴─────────────────┴─────────────────┴───────────────────┴────────┐ │
│  │                         WorkflowEventStore                             │ │
│  │  (PostgreSQL: durable_workflow_instances, durable_workflow_events,    │ │
│  │   durable_task_queue, durable_workers)                                │ │
│  └────────────────────────────────────────────────────────────────────────┘ │
│                                                                              │
│  ┌────────────────────────────────────────────────────────────────────────┐ │
│  │  Reliability: RetryPolicy, CircuitBreaker, TimeoutManager, DLQ        │ │
│  └────────────────────────────────────────────────────────────────────────┘ │
│                                                                              │
│  ┌────────────────────────────────────────────────────────────────────────┐ │
│  │  Observability: OTel Tracing, Metrics, Admin API                      │ │
│  └────────────────────────────────────────────────────────────────────────┘ │
└─────────────────────────────────────────────────────────────────────────────┘
```

`WorkflowEventStore` is an umbrella over focused traits (`EventLog`, `TaskQueue`, `SignalStore`, `WorkerRegistry`, `DeadLetters`, `CircuitBreakers`, `Schedules`, `DurableAdmin`), blanket-implemented, so a component can bound on only the slice it needs (a worker: `TaskQueue + SignalStore + WorkerRegistry`). Store methods have no silently-succeeding defaults; both stores implement every method, and only derived defaults (for example `count_events` via `load_events`) remain. See `crates/durable/src/persistence/store.rs`.

### Workflow engine feature

The crate has two halves. The core (store contract, task queue, signals,
reliability, `DurableScheduler`) is what Everruns runs in production: the
worker and server drive the queue directly, and turns use durable-engine's
checkpoints rather than replay. The generic workflow engine (`Workflow`,
`Activity`, `WorkflowExecutor` with timers, child workflows and system tasks,
`TimeoutManager`) has no production caller in this repository. `WorkerPool`
is core, not engine: the server's cluster-once maintenance jobs run on it.

Decision: keep the engine, so the crate stays usable and optimizable in
isolation, but put it behind the experimental `workflows` feature. The feature
is on by default for crates.io users, so `cargo add everruns-durable` gets
what the README documents; the workspace dependency sets
`default-features = false`, so server, durable-engine, the facade and the
serve hosts compile only the core. The record types the engine shares with the
queue (`WorkflowEvent`, `ActivityOptions`, `WorkflowError`, `ActivityError`,
`WorkflowSignal`) stay in the core. The engine is proven by the crate's own
suites, benches and
[`examples/order_pipeline.rs`](../../crates/durable/examples/order_pipeline.rs),
which has no Everruns dependency and runs on the in-memory store in the
`durable` CI shard. See `[features]` in
[`crates/durable/Cargo.toml`](../../crates/durable/Cargo.toml).

## Requirements

### Core Abstractions

Items 1-3 are the experimental `workflows` feature.

1. **Workflow** - Deterministic state machine driven by events
   - Unique type identifier
   - Input/Output types (serializable)
   - Event handlers: `on_start`, `on_activity_completed`, `on_activity_failed`, `on_timer_fired`, `on_child_workflow_completed`, `on_child_workflow_failed`, `on_signal`

2. **WorkflowAction** - Actions a workflow can request
   - `ScheduleActivity` - Queue activity with retry policy, timeouts, priority
   - `StartTimer` - Delayed execution (fires once, after its duration)
   - `CompleteWorkflow` / `FailWorkflow` - Terminal states
   - `ScheduleChildWorkflow` - Nested workflows; the parent hears the child's outcome
   - `CancelActivity` - Cancel pending work

3. **Activity** - Unit of work that may fail and be retried
   - Unique type identifier
   - Input/Output types
   - Access to `ActivityContext` (attempt info, heartbeat, cancellation)

4. **WorkflowSignal** - External signals to running workflows
   - Types: `cancel`, `shutdown`, custom

### Timers and Child Workflows

Timers and child workflows are tasks on the ordinary queue, claimed by a worker
registered for `SYSTEM_ACTIVITY_TYPES` that calls
`WorkflowExecutor::run_system_tasks`. A timer is a task held back by
`ActivityOptions::start_delay`; both stores honor that delay, and the store
conformance suite checks it. Starting a child and reporting its outcome to the
parent are tasks too. The queue already gives crash survival and retry, and a
task keeps the executor from re-entering a parent while that parent's own
actions are still being applied, which would race its optimistic sequence.

Delivery is at least once, so each step is idempotent: a timer fires once per
`TimerStarted`, a child's UUID is v5 of the parent UUID and the parent's id for
the child, and a parent records each child's outcome once. A child knows its
parent from the `parent` field of its `WorkflowStarted` event. A child whose
type is not registered fails the parent with code `child_not_started`.

Known gaps: a crash between recording a child's terminal status and enqueueing
its result leaves the parent waiting, and cancelling a child does not notify
the parent. Agent turns in Everruns do not use timers or child workflows yet:
workers talk to the store over gRPC, which exposes only the task operations, and
turns are driven by `TurnExecution` checkpoints in durable-engine rather than
replay.

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
  pool creation plus `migrate` for callers with no pool of their own (the
  facade's PostgreSQL durable backend). The script uses only built-in PostgreSQL 14+
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
module and bench binaries sit behind the off-by-default `bench` feature (which
implies `workflows`), and bench checkpoints are excluded from the package.

Workflow statuses: `pending`, `running`, `completed`, `failed`, `cancelled`, `continued_as_new`.

### Replay Safety

- **Pre-load count check (full path):** `count_events()` before `load_events()` rejects oversized histories without allocating.
- **Pre-load count check (snapshot path):** `count_events_after()` before `load_events_after()` rejects stale snapshots. Deletes the stale snapshot on rejection.
- **Continue-as-new:** When a workflow exceeds `max_events_per_workflow`, it can roll over via `continue_as_new()`. This snapshots current state, creates a new workflow from the snapshot, archives old events, and marks the old workflow `continued_as_new` with a reference to the new workflow ID (`continued_as_new_id` column).

- **Action application:** replay re-runs every handler, so the workflow re-issues every action it ever requested. `process_workflow` applies only those without a recording event in history yet (`ActivityScheduled`, `TimerStarted`, `WorkflowCompleted`, and so on), counted per occurrence. This is how the follow-up to a just-appended completion gets scheduled, and it makes re-processing after a crash between appending an event and applying its actions idempotent.

See `crates/durable/src/engine/executor.rs` for `load_workflow_state()`, `unrecorded_actions()`, and `continue_as_new()`.

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

Push-based via gRPC streaming (`SubscribeTaskNotifications`), backed by NATS when available and PostgreSQL `NOTIFY` otherwise. The standalone worker subscribes at startup (`crates/worker/src/task_wakeup.rs`) and every notification cuts its poll backoff short; a worker that finishes a task also wakes its own loop, because that task usually enqueued the next turn phase. Polling stays as the fallback, so a lost notification or a dropped stream costs at most one backoff interval (`WORKER_POLL_BACKOFF_MAX_MS`), and the worker resubscribes with a 1s to 30s backoff.

Why it matters: every turn phase (`process_input`, `reason`, `act`) is its own queued task. Before the worker consumed these notifications, an idle worker sat in a backoff of up to 5s, so a new message waited up to 5s to start and each tool call paid that wait twice (reason to act, act to reason). Measured locally on a two-tool turn: pickup fell from about 1.5s to about 70ms and each phase hand-off from up to 1.5s to under 200ms. `crates/server/tests/workflow_test/latency.rs` guards the pickup end to end.

Once a phase is claimed, its own setup is the next cost: before a reason phase calls the model, host setup reads the session, harness, and agent several times over (dependency check, capability loading, snapshot projection, tool augmentation), and each read from a gRPC worker is a control-plane round trip plus database queries. With the production database a few milliseconds away, that setup took about 600 ms of each ~700 ms hand-off, against about 130 ms locally. The worker memoizes those reads for the length of the setup (`crates/worker/src/phase_reads.rs`) and logs `phase setup` with `setup_ms` and the reads fetched and saved, once per phase.

Because a chained turn's phases run on one worker, the setup reads also carry across phases: what one phase's setup read is kept per turn (org, session, input message) and the turn's next phase starts from it (`crates/worker/src/turn_reads.rs`). A turn's configuration is therefore pinned for the turn on that worker, the way its agent definition already was: an edit made mid-turn applies from the next turn. Conversation history is not part of it and is read fresh by every reason. Session writes made through the worker drop the session's entries; a turn's entries end when it completes or pauses, and age out after ten minutes otherwise. `phase setup` reports `reads_from_turn`.

The first hand-off of a turn is gone: the `process_input` task runs the input step and then the turn's first reason in the same task, and is completed and scheduled as that `reason` (`crates/durable-engine/src/turn_start.rs`). The reason's setup reads start before the input runs, so they overlap. Locally this moved the first model call from 138-175 ms after `turn.started` to 32-46 ms; in production the saved hop was about 230 ms. The turn id comes from the task id, so a retried task reopens the same turn.

The input step runs on the first reason's host, so its reads share the reason's setup memo and its `session.activated` and `turn.started` join the reason's ordered write-behind queue: they are stored in the background, ahead of `reason.started`, instead of before the model call. The input message read runs alongside the session status write. Locally this took claim-to-`reason.started` from 76-90 ms to 44-66 ms.

The remaining hand-offs inside a turn (reason to act, act to reason) no longer go through the queue either. When a step completes, the worker enqueues the next step already claimed by itself (`TaskQueue::enqueue_claimed_task`, carried over gRPC as `claim_for_worker_id` on `EnqueueDurableTask`) and runs it at once, so a turn's steps run back to back on one worker without a notification, a poll wakeup or a claim between them (`crates/durable-engine/src/turn_driver.rs`). Each step is still its own task row: it heartbeats, retries, and is reclaimed when its worker dies, exactly like a claimed task, and the claimed insert records `ActivityStarted` the way a claim does. The store falls back to an ordinary pending task when the worker is draining or unregistered, and a control plane that predates the field ignores it, so mixed versions during a deploy keep working. `WORKER_CHAIN_STEPS=false` turns it off.

Two smaller costs sat on every turn. The server's database pools pinged each idle connection before handing it out, which doubled the round trips of every query outside a transaction. They now ping only connections idle for 30 s or more (`crates/server/src/storage/repositories/mod.rs`). And the first text delta waited a full 100 ms batch after the model's first token, because the batch clock started with the stream; the first token now goes out at once (`crates/core/src/engine/execution/reason.rs`). In the UI, the streaming typewriter reveals each chunk over about one batch (`apps/ui/src/components/streaming-message.tsx`) instead of at a fixed rate that left the text several hundred milliseconds behind the model.

Starting a turn is one store call, `EventLog::start_run_with_task` (`crates/durable/src/persistence/store.rs`): it creates the session's workflow or starts a new run of it and enqueues `process_input` in one transaction, or reports the run as active so the send becomes a steering signal. Before, `DurableRunner` made that decision in three or four calls under a process-wide mutex, so every send in a server process queued behind every other, and a failure between the claim and the enqueue left a session Running with no task. Now the runner holds no lock; the store's row lock elects one winner per session. The task claim also returns each task's workflow status from the same statement, so the control plane no longer reads it once per claimed task.

Operational contract:

- NATS is the preferred backend when configured
- PostgreSQL fallback must use a direct session-scoped listener connection, not an ordinary pooled query connection
- deployments that pool `DATABASE_URL` for regular queries should provide `DATABASE_UNPOOLED_URL` for PostgreSQL listener traffic

Reasoning:

- task notifications are latency-sensitive, but correctness matters more than transport choice
- PostgreSQL `LISTEN/NOTIFY` through a pooler/proxy can fail intermittently and surface as protocol errors on unrelated query traffic
- failing fast on an invalid listener URL is better than allowing a deployment that corrupts the control-plane's normal database interactions

### Worker Concurrency & Sizing

External/gRPC workers and the in-process worker both run `crates/worker/src/unified_worker.rs`, which exposes three independent knobs so concurrency, claim batch size, and idle polling can be tuned separately:

| Setting | Env var | Default | Purpose |
| --- | --- | --- | --- |
| Execution concurrency | `MAX_CONCURRENT_TASKS` | `50` | Tasks a worker runs at once / advertised capacity. |
| Claim batch size | `CLAIM_BATCH_SIZE` | `50` (clamped to concurrency) | Upper bound on `claim_task max_tasks`, regardless of free slots. |
| Fallback poll interval | `WORKER_POLL_INTERVAL_MS` | `100` | Base interval when push notifications are unavailable. |
| Fallback poll backoff cap | `WORKER_POLL_BACKOFF_MAX_MS` | `5000` | Idle polling backs off exponentially from the base up to this cap. |
| Step chaining | `WORKER_CHAIN_STEPS` | `true` | Run a turn's next step on the same worker, enqueued already claimed, instead of through the queue. |

Guidance:

- **The default concurrency is intentionally modest (50, matching the default DB pool).** Historically it was `1000`, so a few replicas advertised thousands of slots and a single idle poll issued `claim_task max_tasks=1000`; when the DB was already slow this amplified pool pressure into acquire timeouts (EVE-606). 1000-way concurrency is now opt-in via `MAX_CONCURRENT_TASKS`.
- **Raising concurrency does not raise claim cost:** the claim batch is bounded by `CLAIM_BATCH_SIZE` independently, so a high-concurrency worker still claims in modest batches.
- **Pool sizing, not pool inflation, is the lever.** Size against `pg_max_connections / replicas − margin`; do not raise the pool to mask worker over-claiming. Worker-side concurrency and claim batch should be tuned down first for small instances.
- **Size against the total, not `DATABASE_POOL_MAX` alone (EVE-1081).** A control-plane instance opens two pools: the request pool (`DATABASE_POOL_MAX`) and a smaller background pool for sweeps (`DATABASE_BACKGROUND_POOL_MAX`, default 8). The number to compare against `pg_max_connections / replicas` is their sum — `DatabasePoolConfig::total_max_connections()` in `crates/server/src/storage/repositories/mod.rs`, which is also what the startup sizing warning uses. An instance previously tuned so `DATABASE_POOL_MAX` exactly filled the budget will over-subscribe Postgres by the background pool's size per replica and start getting connection refusals, so reduce `DATABASE_POOL_MAX` by that much when upgrading. Setting `DATABASE_BACKGROUND_POOL_MAX=0` restores the single-pool budget exactly, at the cost of the isolation it buys.
- Effective worker settings (id, concurrency, claim batch, poll interval/backoff) are logged at worker init for troubleshooting.

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

### Failed-turn terminalization

Deterministic activity failures bypass the normal retry policy. When any
activity reaches the DLQ, the worker and stale-task reaper race an atomic
`running → failed` workflow transition; only the winner owns terminal effects.
That owner records `workflow.failed`, emits the canonical `turn.failed` and
`session.idled` lifecycle, and returns the session to `idle`. A later message
can then claim the terminal workflow for a new turn instead of remaining queued
behind dead work.

### Stale-task reaping

Reclaim and the settling of dead and sealed tasks are engine logic in
`everruns_durable::maintenance` (`reap_stale_tasks`, `StaleTaskReaper`), not in
the host. One pass reclaims stale claims, then for each dead task records
`ActivityFailed` and fails its workflow with the `try_fail_workflow`
compare-and-set; for each sealed task it records `ActivityFailed` and marks the
workflow failed (`task sealed: no_progress (N recoveries)`). The host plugs in
a `ReapHandler` for what that means in its domain: the server's
(`crates/server/src/durable_reaper.rs`) emits the failed or sealed turn
lifecycle, and only the reaper that won the workflow transition calls it, so
replicas racing the same reap notify once. The server reaps every 10 s with a
30 s stale threshold. `WorkerPool` runs the same pass with a no-op handler, and
`WorkerPoolConfig::without_stale_reclaim` turns its loop off for a host that
already runs a reaper with a real handler.

### Step hand-off and stranded runs

A turn step moves its workflow to the next step in one store write
(`TaskQueue::complete_task_and_hand_off`, carried over gRPC as `hand_off` on
`CompleteDurableTask`): the task completes, the steering wakes the next step
was planned with are consumed, and the next step is enqueued (claimed when
chaining) or the workflow completes, in one PostgreSQL transaction over the
queue rows only. History events stay outside it, appended around it as before.
Done as two writes, a worker exit or a lost reply between "complete and drain"
and "enqueue" left the workflow `running` with no task: the drained steering
was lost, and every later message became a wake no run drained.

The driver plans before it commits, so the step's lifecycle effects (turn
completed, session idle) run first and repeat if the hand-off never commits:
at least once, never lost. It counts pending wakes without consuming them and
the hand-off consumes exactly that many, so a wake that arrives meanwhile
stays pending. One race remains, as before: a wake arriving after the count
at a turn's final step is left pending on the completed workflow.

A run can still be stranded by a client that hands off in separate writes (a
control plane that predates `hand_off`, or a store without the atomic write).
Two things resume it by enqueueing its completed last step again, which reruns
as after a crash before its completion: the server's reaper pass sweeps
`turn_workflow` runs left `running` with no live task a minute after their last
step completed (`requeue_stranded_workflows`, `ReaperConfig::stranded_*`), and
a run start on such a run resumes it at once (`start_run_with_task`) before the
caller steers it. `crates/durable-engine/src/turn_recovery_matrix_tests.rs`
crashes a turn at every step boundary, on both write shapes, in memory and on
PostgreSQL.

### Forward-progress guard and Sealed terminal (EVE-534)

`RetryPolicy.max_attempts` bounds *how many times* a task may run, but a turn
that crashes and is reclaimed repeatedly without ever advancing can still loop,
re-running reason/act and burning tokens/billing, until it incidentally hits
max-iterations or max_attempts. The forward-progress guard adds a poison-turn
defense tied to *progress* and a deliberate **Sealed** terminal.

- **Progress token**: a per-turn, monotonically advancing marker derived from
  durably-recorded facts so it is stable under replay and cannot be advanced by
  a non-progressing retry. It is the highest `durable_workflow_events.sequence_num`
  for the turn's workflow (encoded so "no events yet" = 0). Each stale reclaim
  records the token observed for the task (`durable_task_queue.progress_token`).
- **No-progress detection**: on each reclaim, the store compares the current
  token to the previously recorded one. If it did not advance, the per-task
  `no_progress_count` is incremented; any advance resets it to 0.
- **Sealing**: when `no_progress_count` reaches `N` (default 3, configurable via
  `DURABLE_NO_PROGRESS_SEAL_THRESHOLD`), the reclaim path marks the task `dead`
  (→ DLQ) instead of returning it to `pending`. This stops scheduling, makes the
  turn **non-retryable**, and surfaces a distinct `turn.sealed { reason }` event
  plus `session.idled` (session returns to `idle`). The reclaim consumer also
  marks the workflow terminal so no further atoms are scheduled.
  See `crates/durable/src/persistence/store.rs` (`SealedTaskInfo`, `ReclaimResult`)
  and `reclaim_stale_tasks` in the Postgres/in-memory stores.

The turn-level outcome is `everruns_core::engine::TurnPlan::Terminal` with
`everruns_core::TurnStopReason::Sealed`,
distinct from `Success` and `Failed`. `SealReason` is `no_progress` (this guard)
or `budget`.

**Budget interplay**: work-budget-exceeded (`HardLimitStopRule` balance ≤ 0,
see `knowledge/security/budgeting.md`) resolves to `Sealed { reason: budget }` rather than
retrying: the worker classifies the budget-exhausted failure as a deliberate,
non-retryable seal, routes it straight to the DLQ (no re-billing retries), and
emits `turn.sealed { budget }`.

Operator follow-up (not yet implemented): a UI to inspect and replay sealed
turns from the DLQ. Sealed tasks already carry the seal reason and counters via
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

Real-time metrics on the durable overview page via SSE streaming. Shows last 15 minutes, zero-backfilled. Four chart panels: Workflow Status, Task Status, Throughput, System Load. See `crates/server/src/api/durable.rs` for `MetricsPoint` fields and the ring-buffer collector.

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

**Decision**: The task heartbeat tells two stop reasons apart (EVE-1134). A rejected heartbeat (`accepted = false`) is ownership loss: the task was reclaimed or finished elsewhere. An accepted heartbeat with `should_cancel` means this worker still owns the task but its workflow was cancelled, which is how an explicit turn cancel reaches a running activity. The worker fires its task cancellation for both, and a separate turn-cancel signal only for the second. Provider work that a new owner could resume (an OpenAI background response, see [LLM drivers](../foundations/llm-drivers.md#background-mode-openai-responses)) stops only on the turn cancel, never on ownership loss or a failed heartbeat.

### Worker Communication

**Decision**: Workers communicate via gRPC only, no direct database access. Clear separation between control-plane (owns state) and workers (stateless executors).

**Authentication**: Two layered mechanisms (see `knowledge/security/threat-model.md` TM-DURABLE-002):
1. Bearer token (`WORKER_GRPC_AUTH_TOKEN`) -- required in production
2. Mutual TLS (`WORKER_GRPC_TLS_*`) -- optional transport encryption

**Design decision**: Workers are intentionally cross-org. Org-scoping is enforced at the HTTP API layer, not the gRPC transport.
