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
./scripts/dev.sh durable-bench

# PostgreSQL benchmarks (auto-starts Docker)
./scripts/dev.sh durable-bench-db

# With checkpointing for historical comparison
./scripts/dev.sh durable-bench --save
./scripts/dev.sh durable-bench-db --save ci-4cpu-8gb
```

### Benchmark Framework

Custom framework in `crates/durable/src/bench/`:

- `runner.rs` - Scenario execution with warmup
- `metrics.rs` - Latency histograms, throughput tracking
- `checkpoint.rs` - JSON checkpoints for comparison
- `report.rs` - Markdown report generation
