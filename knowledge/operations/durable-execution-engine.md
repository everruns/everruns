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

Agent turns use `everruns-durable::DurableExecution`, the checkpointed driver
for `everruns-engine::Execution`. The durable crate owns persistence, retries,
and activity scheduling; it does not own a second copy of atom or turn
semantics.

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

## Requirements

### Core Abstractions

1. **Workflow** - Deterministic state machine driven by events
   - Unique type identifier
   - Input/Output types (serializable)
   - Event handlers: `on_start`, `on_activity_completed`, `on_activity_failed`, `on_timer_fired`, `on_signal`

2. **WorkflowAction** - Actions a workflow can request
   - `ScheduleActivity` - Queue activity with retry policy, timeouts, priority
   - `StartTimer` - Delayed execution
   - `CompleteWorkflow` / `FailWorkflow` - Terminal states
   - `ScheduleChildWorkflow` - Nested workflows
   - `CancelActivity` - Cancel pending work

3. **Activity** - Unit of work that may fail and be retried
   - Unique type identifier
   - Input/Output types
   - Access to `ActivityContext` (attempt info, heartbeat, cancellation)

4. **WorkflowSignal** - External signals to running workflows
   - Types: `cancel`, `shutdown`, custom

### Persistence

All tables prefixed with `durable_` to avoid conflicts. See `crates/server/migrations/002_durable_execution.sql` for the full schema DDL.

Workflow statuses: `pending`, `running`, `completed`, `failed`, `cancelled`, `continued_as_new`.

### Replay Safety

- **Pre-load count check (full path):** `count_events()` before `load_events()` rejects oversized histories without allocating.
- **Pre-load count check (snapshot path):** `count_events_after()` before `load_events_after()` rejects stale snapshots. Deletes the stale snapshot on rejection.
- **Continue-as-new:** When a workflow exceeds `max_events_per_workflow`, it can roll over via `continue_as_new()`. This snapshots current state, creates a new workflow from the snapshot, archives old events, and marks the old workflow `continued_as_new` with a reference to the new workflow ID (`continued_as_new_id` column).

See `crates/durable/src/engine/executor.rs` for `load_workflow_state()` and `continue_as_new()`.

### Task Claiming

Workers claim tasks partitioned by `activity_type`. See `crates/durable/src/persistence/store.rs` for implementation.

### Task Notifications

Push-based via gRPC streaming (`SubscribeTaskNotifications`), backed by NATS when available and PostgreSQL `NOTIFY` otherwise. Falls back to polling (10s) on disconnect.

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

Guidance:

- **The default concurrency is intentionally modest (50, matching the default DB pool).** Historically it was `1000`, so a few replicas advertised thousands of slots and a single idle poll issued `claim_task max_tasks=1000`; when the DB was already slow this amplified pool pressure into acquire timeouts (EVE-606). 1000-way concurrency is now opt-in via `MAX_CONCURRENT_TASKS`.
- **Raising concurrency does not raise claim cost:** the claim batch is bounded by `CLAIM_BATCH_SIZE` independently, so a high-concurrency worker still claims in modest batches.
- **Pool sizing, not pool inflation, is the lever.** Size `DATABASE_POOL_MAX` to your managed Postgres `max_connections` (`pg_max_connections / replicas − margin`); do not raise it to mask worker over-claiming. Worker-side concurrency and claim batch should be tuned down first for small instances.
- Effective worker settings (id, concurrency, claim batch, poll interval/backoff) are logged at worker init for troubleshooting.

### Generic Queue (Standalone Tasks)

The task queue supports standalone tasks that run independently of any workflow. `TaskDefinition.workflow_id` is `Option<Uuid>` — when `None`, the task is a standalone queue entry.

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

### Forward-progress guard and Sealed terminal (EVE-534)

`RetryPolicy.max_attempts` bounds *how many times* a task may run, but a turn
that crashes and is reclaimed repeatedly without ever advancing can still loop —
re-running reason/act and burning tokens/billing — until it incidentally hits
max-iterations or max_attempts. The forward-progress guard adds a poison-turn
defense tied to *progress* and a deliberate **Sealed** terminal.

- **Progress token** — a per-turn, monotonically advancing marker derived from
  durably-recorded facts so it is stable under replay and cannot be advanced by
  a non-progressing retry. It is the highest `durable_workflow_events.sequence_num`
  for the turn's workflow (encoded so "no events yet" = 0). Each stale reclaim
  records the token observed for the task (`durable_task_queue.progress_token`).
- **No-progress detection** — on each reclaim, the store compares the current
  token to the previously recorded one. If it did not advance, the per-task
  `no_progress_count` is incremented; any advance resets it to 0.
- **Sealing** — when `no_progress_count` reaches `N` (default 3, configurable via
  `DURABLE_NO_PROGRESS_SEAL_THRESHOLD`), the reclaim path marks the task `dead`
  (→ DLQ) instead of returning it to `pending`. This stops scheduling, makes the
  turn **non-retryable**, and surfaces a distinct `turn.sealed { reason }` event
  plus `session.idled` (session returns to `idle`). The reclaim consumer also
  marks the workflow terminal so no further atoms are scheduled.
  See `crates/durable/src/persistence/store.rs` (`SealedTaskInfo`, `ReclaimResult`)
  and `reclaim_stale_tasks` in the Postgres/in-memory stores.

The turn-level outcome is `everruns_engine::TurnPlan::Terminal` with
`everruns_core::TurnStopReason::Sealed`,
distinct from `Success` and `Failed`. `SealReason` is `no_progress` (this guard)
or `budget`.

**Budget interplay** — work-budget-exceeded (`HardLimitStopRule` balance ≤ 0,
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
- Metrics time series: `/v1/durable/metrics/timeseries` — server-side ring buffer (360 points, 10s resolution)

### Metrics Dashboard

Real-time metrics on the durable overview page via SSE streaming. Shows last 15 minutes, zero-backfilled. Four chart panels: Workflow Status, Task Status, Throughput, System Load. See `crates/server/src/api/durable.rs` for `MetricsPoint` fields and the ring-buffer collector.

### System Health Counters

`get_system_health` (feeding `/v1/durable/health`, the ~10s metrics sampler, and the durable SSE stream) splits its fields by cost:

- **Live gauges** — pending/claimed tasks, running/pending workflows, workers, capacity/load, DLQ size — are queried directly. They are bounded by current work and backed by partial indexes, so they stay cheap.
- **Cumulative totals** — completed/failed/started for tasks and workflows (the Prometheus-style monotonic counters) — are read from `durable_stat_counters`, a tiny table maintained incrementally by AFTER triggers on `durable_task_queue` and `durable_workflow_instances` (migration `082`). The history tables are never pruned in production, so these grew without bound; counting them per call forced repeated full scans (~127MB heap at ~156k rows) and could starve the DB pool.

Each counter uses delta accounting (increment on entering a counted state, decrement on leaving or deletion), so it is always exactly equal to the `COUNT(*)` it replaces — there is no staleness window. Reads are O(1) point lookups independent of history size. The per-transition trigger writes touch one shared counter row per metric; this is acceptable at the engine's task throughput, and can be sharded later if a single counter row becomes a write hotspot.

### Worker Heartbeat & Stale Worker Handling

Workers heartbeat every 5s. `WORKER_HEARTBEAT_TIMEOUT_SECS` (60s, in `crates/durable/src/persistence/store.rs`) is the single source of truth for stale detection, used by `get_system_health`, `list_workers`, and `reclaim_stale_tasks`.

## Decisions

### Partitioning Strategy

**Decision**: No custom partitioning in v1. PostgreSQL with proper indexes and `SKIP LOCKED` handles the target load. Activity-type-based claiming naturally partitions work.

### Task Ownership Verification

**Decision**: Verify ownership on task completion. Prevents duplicate activity scheduling when a worker's heartbeat times out and task is reclaimed. Late-finishing worker gets `TaskNotOwned` error.

### Worker Communication

**Decision**: Workers communicate via gRPC only, no direct database access. Clear separation between control-plane (owns state) and workers (stateless executors).

**Authentication**: Two layered mechanisms (see `knowledge/security/threat-model.md` TM-DURABLE-002):
1. Bearer token (`WORKER_GRPC_AUTH_TOKEN`) -- required in production
2. Mutual TLS (`WORKER_GRPC_TLS_*`) -- optional transport encryption

**Design decision**: Workers are intentionally cross-org. Org-scoping is enforced at the HTTP API layer, not the gRPC transport.
