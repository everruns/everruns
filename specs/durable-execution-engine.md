# Durable Execution Engine Specification

## Abstract

Custom PostgreSQL-backed durable execution engine for workflow orchestration with automatic retries, circuit breakers, and distributed task execution.

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

Push-based via gRPC streaming (`SubscribeTaskNotifications`), backed by PostgreSQL NOTIFY or NATS. Falls back to polling (10s) on disconnect.

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

### Worker Heartbeat & Stale Worker Handling

Workers heartbeat every 5s. `WORKER_HEARTBEAT_TIMEOUT_SECS` (60s, in `crates/durable/src/persistence/store.rs`) is the single source of truth for stale detection, used by `get_system_health`, `list_workers`, and `reclaim_stale_tasks`.

## Decisions

### Partitioning Strategy

**Decision**: No custom partitioning in v1. PostgreSQL with proper indexes and `SKIP LOCKED` handles the target load. Activity-type-based claiming naturally partitions work.

### Task Ownership Verification

**Decision**: Verify ownership on task completion. Prevents duplicate activity scheduling when a worker's heartbeat times out and task is reclaimed. Late-finishing worker gets `TaskNotOwned` error.

### Worker Communication

**Decision**: Workers communicate via gRPC only, no direct database access. Clear separation between control-plane (owns state) and workers (stateless executors).

**Authentication**: Two layered mechanisms (see `specs/threat-model.md` TM-DURABLE-002):
1. Bearer token (`WORKER_GRPC_AUTH_TOKEN`) -- required in production
2. Mutual TLS (`WORKER_GRPC_TLS_*`) -- optional transport encryption

**Design decision**: Workers are intentionally cross-org. Org-scoping is enforced at the HTTP API layer, not the gRPC transport.

