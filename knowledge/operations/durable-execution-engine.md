---
type: Specification
title: "Durable Execution Engine Specification"
description: "PostgreSQL-backed durable task queue, event log, signals and schedules."
tags:
  - everruns
  - operations
---
# Durable Execution Engine Specification

## Abstract

Custom PostgreSQL-backed durable execution engine: a distributed task queue with automatic retries, circuit breakers, signals, an event log, and schedules.

`everruns-durable` is a generic core: workflow records, activities, tasks,
signals, and schedules, with no `everruns-*` dependency (enforced by
`scripts/lib/check-durable-isolation.sh`). Agent semantics live above it.

Platform turns no longer run on it. Since the actor cutover (step 6c,
2026-10-11) a turn is a session actor: intake marks the session runnable, a
worker leases it and runs the whole turn, and the session's event log is the
only turn state ([Session actors](#session-actors-turn-execution)). The queue
still carries the activities that are not turns: leased-resource cleanup, the
session task reaper, and scheduled channel and agent-trigger invocations.
`everruns-durable-engine` (the `TurnTaskDriver` that ran a turn as one queued
task per step, checkpointing `everruns_core::engine::TurnExecution` between
them, and the `durable_turn` conventions) still runs those activities and is
still published as the facade's experimental durable turn backend
([Execution Backends](../framework/execution-backends.md)); its turn-step code
on the platform is dead and goes in step 7. The durable crate owns
persistence, retries, and activity scheduling.
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

`WorkflowEventStore` is an umbrella over focused traits (`EventLog`, `TaskQueue`, `SignalStore`, `WorkerRegistry`, `DeadLetters`, `CircuitBreakers`, `Schedules`, `DurableAdmin`), blanket-implemented, so a component can bound on only the slice it needs (a worker: `TaskQueue + SignalStore + WorkerRegistry`). Store methods have no silently-succeeding defaults; both stores implement every method, and only derived defaults (for example `count_events` via `load_events`) remain. See `crates/durable/src/persistence/store.rs`.

### No replayed workflow engine

Decision (2026-10-10): the crate has no generic replayed workflow engine. It
once carried one (`Workflow` and `Activity` traits, `WorkflowExecutor` with
timers, child workflows and system tasks, `TimeoutManager`) behind an
experimental `workflows` feature. Nothing in production ran it: the worker and
server drive the queue directly, and turns use durable-engine's checkpoints
rather than replay. The actor-based execution design retires it, so it was
removed instead of maintained. The record types it shared with the queue
(`WorkflowEvent`, `ActivityOptions`, `WorkflowError`, `ActivityError`,
`WorkflowSignal`) stay. `WorkerPool` is core: the server's cluster-once
maintenance jobs run on it.

## Requirements

### Core Abstractions

1. **WorkflowSignal** - External signals to running workflows
   - Types: `cancel`, `shutdown`, custom

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

Push-based via gRPC streaming (`SubscribeTaskNotifications`), backed by NATS when available and PostgreSQL `NOTIFY` otherwise. The standalone worker subscribes at startup (`crates/worker/src/task_wakeup.rs`) and every notification cuts its poll backoff short; a worker that finishes a task also wakes its own loop, because that task usually enqueued the next turn phase. Polling stays as the fallback, so a lost notification or a dropped stream costs at most one backoff interval (`WORKER_POLL_BACKOFF_MAX_MS`), and the worker resubscribes with a 1s to 30s backoff.

Session actors ride the same channel with their own payloads: `session_actor` when an actor becomes leasable and `session_actor_cancel` when a cancel lands on a leased one. The worker subscribes to each under its own subscriber id (the control plane keeps one subscription per id): the first wakes the actor lease loop, the second makes every running lease renew at once, which is how its holder learns of the cancel. Under PostgreSQL a trigger on `session_actors` notifies on commit; under NATS the server's turn backend publishes after the intake transaction commits.

The rest of this section is the history of the per-step turn path the actor cutover retired; its measurements explain choices the actor runtime kept (the push wake-up, per-phase setup reads, the per-turn read cache, write-behind events). Why it mattered then: every turn phase (`process_input`, `reason`, `act`) was its own queued task. Before the worker consumed these notifications, an idle worker sat in a backoff of up to 5s, so a new message waited up to 5s to start and each tool call paid that wait twice (reason to act, act to reason). Measured locally on a two-tool turn: pickup fell from about 1.5s to about 70ms and each phase hand-off from up to 1.5s to under 200ms. `crates/server/tests/workflow_test/latency.rs` guards the pickup end to end.

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

External/gRPC workers and the in-process worker both run `crates/worker/src/unified_worker.rs`, which exposes three independent knobs so concurrency, claim batch size, and idle polling can be tuned separately. The session actor loop and the queue loop share them: a leased actor takes an execution slot like a claimed task, `CLAIM_BATCH_SIZE` also bounds one lease call, and a turn waiting on the model hands its slot back the same way:

| Setting | Env var | Default | Purpose |
| --- | --- | --- | --- |
| Execution concurrency | `MAX_CONCURRENT_TASKS` | `50` | Tasks a worker runs at once / advertised capacity. |
| Model waits | `MAX_CONCURRENT_MODEL_WAITS` | `200` | Reason steps that hand their slot back while waiting on the model; `0` turns it off. |
| Claim batch size | `CLAIM_BATCH_SIZE` | `50` (clamped to concurrency) | Upper bound on `claim_task max_tasks`, regardless of free slots. |
| Fallback poll interval | `WORKER_POLL_INTERVAL_MS` | `100` | Base interval when push notifications are unavailable. |
| Fallback poll backoff cap | `WORKER_POLL_BACKOFF_MAX_MS` | `5000` | Idle polling backs off exponentially from the base up to this cap. |
| Step chaining | `WORKER_CHAIN_STEPS` | `true` | Retired path: chained a queued turn's next step on the same worker. Turns no longer queue steps. |
| Shutdown grace | `WORKER_SHUTDOWN_GRACE_SECS` | `25` | How long a stopping worker lets in-flight tasks finish before it aborts them. Keep it under the orchestrator's kill timeout. |

Guidance:

- **The default concurrency is intentionally modest (50, matching the default DB pool).** Historically it was `1000`, so a few replicas advertised thousands of slots and a single idle poll issued `claim_task max_tasks=1000`; when the DB was already slow this amplified pool pressure into acquire timeouts (EVE-606). 1000-way concurrency is now opt-in via `MAX_CONCURRENT_TASKS`.
- **A slot is for work, not for waiting on the model.** A reason step spends most of its time on the model's stream, doing little DB or CPU work, so it hands its slot back for that wait (`crates/core/src/engine/execution/model_wait.rs`). Without this, `MAX_CONCURRENT_TASKS` capped how many turns could talk to a model at once: on prod, 3 workers x 20 slots added about 2 s of queueing at 100 concurrent sessions (load test, 2026-10-08). Waits have their own cap because each open stream still costs memory, a connection and batched delta writes; past it a wait keeps its slot.
- **Raising concurrency does not raise claim cost:** the claim batch is bounded by `CLAIM_BATCH_SIZE` independently, so a high-concurrency worker still claims in modest batches.
- **Pool sizing, not pool inflation, is the lever.** Size against `pg_max_connections / replicas − margin`; do not raise the pool to mask worker over-claiming. Worker-side concurrency and claim batch should be tuned down first for small instances.
- **Size against the total, not `DATABASE_POOL_MAX` alone (EVE-1081).** A control-plane instance opens two pools: the request pool (`DATABASE_POOL_MAX`) and a smaller background pool for sweeps (`DATABASE_BACKGROUND_POOL_MAX`, default 8). The number to compare against `pg_max_connections / replicas` is their sum — `DatabasePoolConfig::total_max_connections()` in `crates/server/src/storage/repositories/mod.rs`, which is also what the startup sizing warning uses. An instance previously tuned so `DATABASE_POOL_MAX` exactly filled the budget will over-subscribe Postgres by the background pool's size per replica and start getting connection refusals, so reduce `DATABASE_POOL_MAX` by that much when upgrading. Setting `DATABASE_BACKGROUND_POOL_MAX=0` restores the single-pool budget exactly, at the cost of the isolation it buys.
- Effective worker settings (id, concurrency, claim batch, poll interval/backoff) are logged at worker init for troubleshooting.

### Worker Drain and Shutdown

A worker is drained two ways, and both end in the same registry state (`status = 'draining'`), so the queue hands it no new claims and no chained steps:

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
(`crates/server/src/background/durable_reaper.rs`) emits the failed or sealed turn
lifecycle, and only the reaper that won the workflow transition calls it, so
replicas racing the same reap notify once. The server reaps every 10 s with a
30 s stale threshold. `WorkerPool` runs the same pass with a no-op handler, and
`WorkerPoolConfig::without_stale_reclaim` turns its loop off for a host that
already runs a reaper with a real handler.

### Step hand-off and stranded runs

Retired by the actor cutover: this describes the per-step turn path, kept in
the code until step 7. The reaper no longer sweeps stranded `turn_workflow`
runs (`stranded_workflow_type: None`), because no live code creates them.

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
  starts nothing. `inbox_seq` (the newest input's event sequence) only moves
  forward. A stored message also records `next_input_seq`, the input the next
  fresh turn starts from, when the previous one was consumed, so queued
  messages start turns in arrival order and history written without a mark (a
  channel's backfill) never starts one. A resolution (tool results, an answer,
  an approval decision, a consent) marks the inbox at the log head instead: it
  continues the open turn.
- **One turn per lease.** The worker folds the log's tail from the latest
  `turn.started`. An open turn is resumed: unfinished calls rerun when their
  tool declares that safe (pure, idempotent, client-side, approval-gated,
  `ask_user`, or a synthetic person-facing call) and are otherwise settled as
  `interrupted`; an open turn whose answer is already in the log is completed
  without another model call. With no open turn, the first unconsumed input
  starts a fresh turn through core's `turn_entry_plan`, the entry step the
  in-process runtime uses.
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
  A retryable failure (a store or transport error) also lets the lease
  expire; the TTL is the backoff before another worker resumes from the log.
  Expiry is database time.
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
  exactly one terminal event. A cancel the holder records is answered only
  once the holder ended the turn (bounded by two lease lives); inside a
  command transaction that wait runs after the commit, when the holder can
  first see the request, still before the command answers. Past the bound the
  answer means "acknowledged": the request stays on the row for the holder,
  or the next one, to record.
- **Poison guard.** `attempts` counts leases since the last release. A lease
  that finds more than `DURABLE_NO_PROGRESS_SEAL_THRESHOLD` (default 3) seals
  the turn instead of running it: a user-facing processing error,
  `turn.sealed { no_progress }`, `session.idled`, and the pending input
  consumed. A turn that kills its worker cannot loop forever, and the session
  is usable afterwards: the release resets the count, and the next message
  starts a fresh turn.
- **Per-org fairness cap.** A lease call counts each org's held leases and
  takes no more than the cap (default 32). The count is read at the start of
  the call, so concurrent callers can overshoot it slightly: a fairness bound,
  not a quota. Admission stays with `ORG_MAX_ACTIVE_TURNS` when the input is
  accepted.
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
moves in-flight work across: every session that is active or has a pending or
running `turn_workflow` gets an actor row, runnable, with its cursor at the
log's last turn boundary, so a turn the old path left open is resumed from
the log by the first worker that leases it. Pending and claimed
`process_input`, `reason` and `act` tasks and pending and running
`turn_workflow` runs are cancelled. Old servers and workers still running
during a rolling deploy can create old-path work after the migration ran (an
old server starting a turn as `process_input`, an old worker enqueueing the
next step). A temporary sweep on every server
([`legacy_turn_sweep`](../../crates/server/src/background/legacy_turn_sweep.rs),
every 5 s) converts them the same way: pending tasks, and claimed ones whose
old worker went silent for 2 minutes, mark their session's actor runnable,
and the tasks and their `turn_workflow` are cancelled, in one idempotent
statement. The other direction needs nothing: old workers claim durable
activities by type and never see a turn of this release, which is a
`session_actors` row handed out only by the lease RPCs. During the overlap an
old worker may still be running an old-path step of a session a new worker
also leases, and its writes carry no fence; a recreate deploy (stop every
server and worker, migrate, start) avoids that window. Step 7 deletes the
sweep with the per-step workflow.

### Forward-progress guard and Sealed terminal (EVE-534)

Turns now have the actor poison guard ([Session actors](#session-actors-turn-execution));
this guard still covers queued tasks.

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
