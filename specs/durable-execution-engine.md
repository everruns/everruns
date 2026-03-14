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

All tables prefixed with `durable_` to avoid conflicts:

| Table | Purpose |
|-------|---------|
| `durable_workflow_instances` | Workflow state (status, input, result, error) |
| `durable_workflow_events` | Append-only event log for replay |
| `durable_task_queue` | Activity scheduling with claiming |
| `durable_dead_letter_queue` | Failed tasks after retry exhaustion |
| `durable_circuit_breaker_state` | Shared circuit breaker state |
| `durable_workers` | Worker registry for monitoring |
| `durable_signals` | Signal queue for workflows |
| `durable_workflow_snapshots` | Checkpoint snapshots for replay optimization |

Workflow statuses: `pending`, `running`, `completed`, `failed`, `cancelled`, `continued_as_new`.

### Replay Safety

- **Pre-load count check (full path):** `count_events()` before `load_events()` rejects oversized histories without allocating.
- **Pre-load count check (snapshot path):** `count_events_after()` before `load_events_after()` rejects stale snapshots. Deletes the stale snapshot on rejection.
- **Continue-as-new:** When a workflow exceeds `max_events_per_workflow`, it can roll over via `continue_as_new()`. This snapshots current state, creates a new workflow from the snapshot, archives old events, and marks the old workflow `continued_as_new` with a reference to the new workflow ID (`continued_as_new_id` column).

See `crates/durable/src/engine/executor.rs` for `load_workflow_state()` and `continue_as_new()`.

### Task Claiming

Critical for scalability at 1000+ workers:

- `SELECT FOR UPDATE SKIP LOCKED` - Workers don't block each other
- Partition by `activity_type` - Reduces row scanning
- Batch claiming - Fewer round trips
- Partial index on `status = 'pending'` - Smaller index

### Task Notifications (Push-Based)

Low-latency task distribution via PostgreSQL NOTIFY:

- **Trigger**: `notify_task_available()` fires on task INSERT with `status = 'pending'`
- **Channel**: `task_available` with activity_type as payload
- **Broadcaster**: Control-plane listens via `PgListener`, pushes to workers via gRPC streaming
- **Worker subscription**: `SubscribeTaskNotifications` gRPC stream
- **Fallback**: Workers poll with 10s interval if stream disconnects

Latency improvement:
- Polling (100ms): P50=~100ms, P99=~110ms
- Push notifications: P50=~4ms, P99=~10ms (~96% improvement)

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

Real-time metrics charts on the durable overview page, powered by SSE streaming and a server-side `MetricsCollector`. The UI always shows the **last 15 minutes**, zero-backfilled when data is missing (server just started or idle).

1. **Workflow Status** — stacked area (running + pending) with completed/failed rate lines
2. **Task Status** — stacked area (pending + claimed) with completed/failed rate lines
3. **Throughput** — line chart: completed/failed tasks per interval (delta rates)
4. **System Load** — dual-axis line chart: load % (0-100 left axis), workers + DLQ (right axis)

**Architecture**: Background `tokio::spawn` task samples `SystemHealth` + worker stats every 10 seconds into a `VecDeque<MetricsPoint>` ring buffer (max 360 = 1 hour). Data is:
- Included in global durable SSE `snapshot` events as `metrics_history` field
- Available via REST `GET /v1/durable/metrics/timeseries`
- Rendered with `recharts` (React charting library)
- Frontend slices to 15-minute window (90 points at 10s resolution) and zero-fills gaps

**MetricsPoint fields** (see `crates/server/src/api/durable.rs`):
- Gauges: `running_workflows`, `pending_workflows`, `pending_tasks`, `claimed_tasks`, `active_workers`, `load_percentage`, `dlq_size`
- Cumulative totals (for delta rates): `tasks_completed_total`, `tasks_failed_total`, `workflows_completed_total`, `workflows_failed_total`

### Worker Heartbeat & Stale Worker Handling

Workers send heartbeats every 5 seconds. The `WORKER_HEARTBEAT_TIMEOUT_SECS` constant (60s, defined in `crates/durable/src/persistence/store.rs`) is the single source of truth for stale detection:

- **`get_system_health`** — only counts workers with heartbeat within threshold as active
- **`list_workers`** — only returns workers with heartbeat within threshold
- **`reclaim_stale_tasks`** — marks workers with stale heartbeats as `stopped` (cleans up workers that crashed without calling `deregister_worker`)

Works in both dev mode (in-memory store) and full mode (PostgreSQL).

## Decisions

### Partitioning Strategy

**Decision**: No custom partitioning in v1.

**Rationale**: PostgreSQL with proper indexes handles 10,000+ tasks/second. `SKIP LOCKED` eliminates contention. Activity-type-based claiming naturally partitions work. Can add PostgreSQL native partitioning later if needed.

### Workflow Versioning

**Decision**: Not in v1.

**Rationale**: Running workflows complete with original code. New workflows use new code. Replay-based migration can be added when needed.

### Signals

**Decision**: Yes, implement signals.

**Use cases**: Cancel workflow, graceful shutdown, external events affecting workflow behavior.

### Task Ownership Verification

**Decision**: Verify ownership on task completion.

**Rationale**: Prevents duplicate activity scheduling when a worker's heartbeat times out and task is reclaimed by another worker. Late-finishing worker gets `TaskNotOwned` error.

### Worker Communication

**Decision**: Workers communicate via gRPC only, no direct database access.

**Rationale**: Clear separation between control-plane (owns state) and workers (stateless executors). Workers don't need database credentials.

**Task Distribution**: Push-based via gRPC streaming. Workers subscribe to `SubscribeTaskNotifications` and receive immediate notifications when tasks are enqueued. Falls back to polling (10s interval) if stream disconnects.

**Startup**: Workers retry connecting to control-plane for up to 5 seconds with exponential backoff (100ms → 1s). This handles startup race conditions when both services restart simultaneously.

**Authentication**: Two layered mechanisms (see `specs/threat-model.md` TM-DURABLE-002):
1. Bearer token (`WORKER_GRPC_AUTH_TOKEN`) — required in production, server panics if unset
2. Mutual TLS (`WORKER_GRPC_TLS_*`) — optional, provides transport encryption + mutual identity verification

**Design decision**: Workers are intentionally cross-org. They are stateless task executors that process work from any organization's queue. Org-scoping is enforced at the HTTP API layer, not the gRPC transport.

## Benchmarks

Load tests for validating performance and scalability. Located in `crates/durable/benches/`.

### In-Memory Benchmarks

Fast iteration without database overhead:

| Benchmark | Purpose |
|-----------|---------|
| `concurrent_workers` | Task claiming with SKIP LOCKED at various worker counts |
| `workflow_throughput` | Multi-step workflow execution throughput |
| `cold_start_latency` | Time from task enqueue to worker pickup |

### PostgreSQL Benchmarks

Real database performance with actual I/O:

| Benchmark | Purpose |
|-----------|---------|
| `db_concurrent_workers` | Task claiming with real PostgreSQL |
| `db_workflow_throughput` | Multi-step workflows with persistence |
| `db_cold_start_latency` | Cold-start latency: polling vs push notifications |

### Running Benchmarks

```bash
# In-memory benchmarks (fast)
just durable-bench

# PostgreSQL benchmarks (auto-starts Docker)
just durable-bench-db

# With checkpointing for historical comparison
just durable-bench --save
just durable-bench-db --save ci-4cpu-8gb
```

### Benchmark Framework

Custom framework in `crates/durable/src/bench/`:

- `runner.rs` - Scenario execution with warmup
- `metrics.rs` - Latency histograms, throughput tracking
- `checkpoint.rs` - JSON checkpoints for comparison
- `report.rs` - Markdown report generation
