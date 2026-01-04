# Durable Execution Engine Specification

## Abstract

This specification describes a custom durable execution engine to replace Temporal. The engine provides workflow orchestration with persistence, automatic retries, circuit breakers, and distributed task execution - all backed by PostgreSQL.

## Goals

1. **Self-contained crate** - `everruns-durable` with no Temporal dependencies
2. **PostgreSQL-only persistence** - No additional infrastructure required
3. **Testable in isolation** - Unit tests, integration tests, load/stress tests
4. **Production-ready reliability** - Retries, circuit breakers, timeouts, dead letter queues
5. **Simple mental model** - Event-sourced workflows with explicit state machines
6. **Scalable** - Support 1000+ concurrent workers
7. **Observable** - Full OpenTelemetry integration with monitoring UI

## Non-Goals

1. Multi-region replication (use PostgreSQL replication)
2. Language-agnostic SDKs (Rust only)
3. Visual workflow designer
4. Multi-tenancy (deferred to future system-wide multi-tenancy)

---

## Architecture Overview

```
┌─────────────────────────────────────────────────────────────────────────────┐
│                            everruns-durable                                  │
├─────────────────────────────────────────────────────────────────────────────┤
│                                                                              │
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
│  │                      Reliability Layer                                  │ │
│  │  ┌────────────┐  ┌────────────┐  ┌────────────┐  ┌────────────┐       │ │
│  │  │   Retry    │  │  Circuit   │  │  Timeout   │  │    DLQ     │       │ │
│  │  │   Policy   │  │  Breaker   │  │  Manager   │  │  Handler   │       │ │
│  │  └────────────┘  └────────────┘  └────────────┘  └────────────┘       │ │
│  └────────────────────────────────────────────────────────────────────────┘ │
│                                                                              │
│  ┌────────────────────────────────────────────────────────────────────────┐ │
│  │                      Observability Layer                                │ │
│  │  ┌────────────┐  ┌────────────┐  ┌────────────┐  ┌────────────┐       │ │
│  │  │   OTel     │  │  Metrics   │  │  Admin     │  │  Worker    │       │ │
│  │  │  Tracing   │  │  Export    │  │  API       │  │  Monitor   │       │ │
│  │  └────────────┘  └────────────┘  └────────────┘  └────────────┘       │ │
│  └────────────────────────────────────────────────────────────────────────┘ │
└─────────────────────────────────────────────────────────────────────────────┘
```

---

## Crate Structure

```
crates/durable/
├── Cargo.toml
├── src/
│   ├── lib.rs                 # Public API exports
│   │
│   ├── workflow/              # Workflow abstractions
│   │   ├── mod.rs
│   │   ├── definition.rs      # Workflow trait and types
│   │   ├── state.rs           # WorkflowState, serialization
│   │   ├── action.rs          # WorkflowAction enum
│   │   ├── signal.rs          # WorkflowSignal for external communication
│   │   └── registry.rs        # WorkflowRegistry
│   │
│   ├── activity/              # Activity execution
│   │   ├── mod.rs
│   │   ├── definition.rs      # Activity trait and types
│   │   ├── context.rs         # ActivityContext for retries, heartbeats
│   │   └── registry.rs        # ActivityRegistry
│   │
│   ├── engine/                # Core execution engine
│   │   ├── mod.rs
│   │   ├── executor.rs        # WorkflowExecutor - drives state machines
│   │   ├── scheduler.rs       # Task scheduling and claiming
│   │   └── replay.rs          # WorkflowEvent replay for recovery
│   │
│   ├── persistence/           # Database layer
│   │   ├── mod.rs
│   │   ├── store.rs           # WorkflowEventStore trait
│   │   ├── postgres.rs        # PostgreSQL implementation
│   │   ├── memory.rs          # In-memory for testing
│   │   └── migrations/        # SQL migrations
│   │       ├── V001__durable_workflow_instances.sql
│   │       ├── V002__durable_workflow_events.sql
│   │       ├── V003__durable_task_queue.sql
│   │       ├── V004__durable_dead_letter_queue.sql
│   │       ├── V005__durable_circuit_breaker_state.sql
│   │       └── V006__durable_workers.sql
│   │
│   ├── reliability/           # Resilience patterns
│   │   ├── mod.rs
│   │   ├── retry.rs           # RetryPolicy, exponential backoff
│   │   ├── circuit_breaker.rs # CircuitBreaker state machine
│   │   ├── timeout.rs         # Timeout handling
│   │   ├── rate_limiter.rs    # Rate limiting for activities
│   │   ├── backpressure.rs    # Backpressure signaling
│   │   └── dlq.rs             # Dead letter queue
│   │
│   ├── worker/                # Worker process
│   │   ├── mod.rs
│   │   ├── pool.rs            # WorkerPool - manages worker threads
│   │   ├── poller.rs          # Task polling with backoff
│   │   ├── heartbeat.rs       # Worker liveness
│   │   └── backpressure.rs    # Worker-side backpressure handling
│   │
│   ├── observability/         # OpenTelemetry & monitoring
│   │   ├── mod.rs
│   │   ├── tracing.rs         # OTel span creation and propagation
│   │   ├── metrics.rs         # Prometheus/OTel metrics
│   │   └── semantic.rs        # Semantic conventions for durable execution
│   │
│   └── admin/                 # Admin API and monitoring
│       ├── mod.rs
│       ├── api.rs             # REST API for admin operations
│       ├── workers.rs         # Worker monitoring
│       ├── workflows.rs       # Workflow inspection
│       └── dlq.rs             # DLQ management
│
├── tests/
│   ├── integration/           # Integration tests
│   │   ├── mod.rs
│   │   ├── workflow_execution.rs
│   │   ├── activity_retry.rs
│   │   ├── circuit_breaker.rs
│   │   ├── recovery.rs
│   │   ├── backpressure.rs
│   │   └── scale_1000_workers.rs
│   │
│   └── fixtures/              # Test utilities
│       ├── mod.rs
│       ├── test_workflows.rs
│       └── test_activities.rs
│
├── benches/                   # Performance benchmarks
│   ├── workflow_throughput.rs
│   ├── activity_latency.rs
│   ├── concurrent_workers.rs
│   └── task_claiming.rs       # Critical path benchmark
│
└── examples/
    ├── simple_workflow.rs
    ├── retry_example.rs
    ├── circuit_breaker_example.rs
    └── signal_cancellation.rs
```

---

## Core Abstractions

### 1. Workflow Trait

```rust
/// A workflow is a deterministic state machine driven by events
pub trait Workflow: Send + Sync + 'static {
    /// Unique type identifier for this workflow
    const TYPE: &'static str;

    /// Input type for starting the workflow
    type Input: Serialize + DeserializeOwned + Send;

    /// Output type when workflow completes
    type Output: Serialize + DeserializeOwned + Send;

    /// Create a new workflow instance from input
    fn new(input: Self::Input) -> Self;

    /// Called when workflow starts (or replays from beginning)
    fn on_start(&mut self) -> Vec<WorkflowAction>;

    /// Called when an activity completes successfully
    fn on_activity_completed(
        &mut self,
        activity_id: &str,
        result: serde_json::Value,
    ) -> Vec<WorkflowAction>;

    /// Called when an activity fails (after all retries exhausted)
    fn on_activity_failed(
        &mut self,
        activity_id: &str,
        error: &ActivityError,
    ) -> Vec<WorkflowAction>;

    /// Called when a timer fires
    fn on_timer_fired(&mut self, timer_id: &str) -> Vec<WorkflowAction>;

    /// Called when an external signal is received
    fn on_signal(&mut self, signal: &WorkflowSignal) -> Vec<WorkflowAction>;

    /// Check if workflow has reached a terminal state
    fn is_completed(&self) -> bool;

    /// Get the workflow result (if completed)
    fn result(&self) -> Option<Result<Self::Output, WorkflowError>>;
}
```

### 2. WorkflowAction Enum

```rust
/// Actions a workflow can request
#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum WorkflowAction {
    /// Schedule an activity for execution
    ScheduleActivity {
        activity_id: String,
        activity_type: String,
        input: serde_json::Value,
        options: ActivityOptions,
    },

    /// Start a timer
    StartTimer {
        timer_id: String,
        duration: Duration,
    },

    /// Complete the workflow successfully
    CompleteWorkflow {
        result: serde_json::Value,
    },

    /// Fail the workflow
    FailWorkflow {
        error: WorkflowError,
    },

    /// Schedule a child workflow
    ScheduleChildWorkflow {
        workflow_id: String,
        workflow_type: String,
        input: serde_json::Value,
    },

    /// Request cancellation of a pending activity
    CancelActivity {
        activity_id: String,
    },

    /// No action (used for event handling that doesn't trigger new work)
    None,
}

/// Options for activity execution
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ActivityOptions {
    /// Retry policy for this activity
    pub retry_policy: RetryPolicy,

    /// Maximum time to wait for activity to start
    pub schedule_to_start_timeout: Duration,

    /// Maximum time for activity execution
    pub start_to_close_timeout: Duration,

    /// Heartbeat interval (for long-running activities)
    pub heartbeat_timeout: Option<Duration>,

    /// Circuit breaker configuration
    pub circuit_breaker: Option<CircuitBreakerConfig>,

    /// Priority (higher = more important, claimed first)
    pub priority: i32,
}
```

### 3. WorkflowSignal

```rust
/// External signals that can be sent to running workflows
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct WorkflowSignal {
    /// Signal type identifier
    pub signal_type: String,

    /// Signal payload
    pub payload: serde_json::Value,

    /// When the signal was sent
    pub sent_at: DateTime<Utc>,
}

/// Common signal types
pub mod signal_types {
    /// Request workflow cancellation
    pub const CANCEL: &str = "cancel";

    /// Request graceful shutdown (complete current activity, then stop)
    pub const SHUTDOWN: &str = "shutdown";

    /// Custom application signal
    pub const CUSTOM: &str = "custom";
}
```

### 4. Activity Trait

```rust
/// An activity is a unit of work that may fail and be retried
#[async_trait]
pub trait Activity: Send + Sync + 'static {
    /// Unique type identifier for this activity
    const TYPE: &'static str;

    /// Input type
    type Input: Serialize + DeserializeOwned + Send;

    /// Output type
    type Output: Serialize + DeserializeOwned + Send;

    /// Execute the activity
    async fn execute(
        &self,
        ctx: &ActivityContext,
        input: Self::Input,
    ) -> Result<Self::Output, ActivityError>;
}

/// Context provided to activities during execution
pub struct ActivityContext {
    /// Unique execution attempt ID
    pub attempt_id: Uuid,

    /// Current attempt number (1-based)
    pub attempt: u32,

    /// Maximum attempts allowed
    pub max_attempts: u32,

    /// Workflow instance ID
    pub workflow_id: Uuid,

    /// Activity ID within the workflow
    pub activity_id: String,

    /// OpenTelemetry span context for distributed tracing
    pub span_context: opentelemetry::Context,

    /// Heartbeat sender for long-running activities
    heartbeat_tx: mpsc::Sender<HeartbeatPayload>,

    /// Cancellation token
    cancellation_token: CancellationToken,
}

impl ActivityContext {
    /// Record a heartbeat (prevents timeout for long activities)
    pub async fn heartbeat(&self, details: Option<serde_json::Value>) -> Result<(), HeartbeatError>;

    /// Check if cancellation was requested
    pub fn is_cancelled(&self) -> bool;

    /// Get a future that resolves when cancellation is requested
    pub fn cancelled(&self) -> impl Future<Output = ()>;
}
```

---

## Persistence Layer

### Database Schema

All tables are prefixed with `durable_` to avoid conflicts with application tables.

```sql
-- V001: Workflow instances
CREATE TABLE durable_workflow_instances (
    id UUID PRIMARY KEY,
    workflow_type TEXT NOT NULL,
    status TEXT NOT NULL DEFAULT 'pending',  -- pending, running, completed, failed, cancelled
    input JSONB NOT NULL,
    result JSONB,
    error JSONB,
    created_at TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    updated_at TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    started_at TIMESTAMPTZ,
    completed_at TIMESTAMPTZ,

    -- Partition key for future sharding
    partition_key INT NOT NULL DEFAULT 0,

    -- Tracing context
    trace_id TEXT,
    span_id TEXT
);

CREATE INDEX idx_durable_workflow_instances_status ON durable_workflow_instances(status);
CREATE INDEX idx_durable_workflow_instances_type ON durable_workflow_instances(workflow_type);
CREATE INDEX idx_durable_workflow_instances_created ON durable_workflow_instances(created_at);

-- V002: Workflow events (append-only log)
CREATE TABLE durable_workflow_events (
    id BIGSERIAL PRIMARY KEY,
    workflow_id UUID NOT NULL REFERENCES durable_workflow_instances(id),
    sequence_num INT NOT NULL,  -- Per-workflow sequence number
    event_type TEXT NOT NULL,
    event_data JSONB NOT NULL,
    created_at TIMESTAMPTZ NOT NULL DEFAULT NOW(),

    -- Tracing context for this event
    trace_id TEXT,
    span_id TEXT,

    UNIQUE(workflow_id, sequence_num)
);

CREATE INDEX idx_durable_workflow_events_workflow ON durable_workflow_events(workflow_id, sequence_num);

-- V003: Task queue (for activity scheduling)
CREATE TABLE durable_task_queue (
    id UUID PRIMARY KEY DEFAULT gen_random_uuid(),
    workflow_id UUID NOT NULL REFERENCES durable_workflow_instances(id),
    activity_id TEXT NOT NULL,
    activity_type TEXT NOT NULL,
    input JSONB NOT NULL,
    options JSONB NOT NULL,

    -- Scheduling
    status TEXT NOT NULL DEFAULT 'pending',  -- pending, claimed, completed, failed, dead, cancelled
    priority INT NOT NULL DEFAULT 0,
    scheduled_at TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    visible_at TIMESTAMPTZ NOT NULL DEFAULT NOW(),  -- For delayed retry

    -- Claiming (partitioned by activity_type for better distribution)
    claimed_by TEXT,  -- Worker ID
    claimed_at TIMESTAMPTZ,
    heartbeat_at TIMESTAMPTZ,

    -- Execution tracking
    attempt INT NOT NULL DEFAULT 0,
    max_attempts INT NOT NULL,
    last_error TEXT,

    -- Timeouts
    schedule_to_start_timeout INTERVAL NOT NULL,
    start_to_close_timeout INTERVAL NOT NULL,
    heartbeat_timeout INTERVAL,

    -- Tracing
    trace_id TEXT,
    span_id TEXT,

    created_at TIMESTAMPTZ NOT NULL DEFAULT NOW()
);

-- Efficient polling query - CRITICAL for 1000 workers
-- Uses partial index + BRIN for time-based queries
CREATE INDEX idx_durable_task_queue_pending
    ON durable_task_queue(activity_type, priority DESC, visible_at)
    WHERE status = 'pending';

CREATE INDEX idx_durable_task_queue_claimed
    ON durable_task_queue(claimed_by, heartbeat_at)
    WHERE status = 'claimed';

CREATE INDEX idx_durable_task_queue_workflow ON durable_task_queue(workflow_id);

-- V004: Dead letter queue
CREATE TABLE durable_dead_letter_queue (
    id UUID PRIMARY KEY DEFAULT gen_random_uuid(),
    original_task_id UUID NOT NULL,
    workflow_id UUID NOT NULL REFERENCES durable_workflow_instances(id),
    activity_id TEXT NOT NULL,
    activity_type TEXT NOT NULL,
    input JSONB NOT NULL,

    -- Failure info
    attempts INT NOT NULL,
    last_error TEXT NOT NULL,
    error_history JSONB NOT NULL,  -- Array of all errors

    -- Metadata
    dead_at TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    requeued_at TIMESTAMPTZ,
    requeue_count INT NOT NULL DEFAULT 0
);

CREATE INDEX idx_durable_dlq_workflow ON durable_dead_letter_queue(workflow_id);
CREATE INDEX idx_durable_dlq_activity_type ON durable_dead_letter_queue(activity_type);
CREATE INDEX idx_durable_dlq_dead_at ON durable_dead_letter_queue(dead_at);

-- V005: Circuit breaker state (shared across workers)
CREATE TABLE durable_circuit_breaker_state (
    key TEXT PRIMARY KEY,  -- e.g., "activity:llm_call" or "external:openai"
    state TEXT NOT NULL DEFAULT 'closed',  -- closed, open, half_open
    failure_count INT NOT NULL DEFAULT 0,
    success_count INT NOT NULL DEFAULT 0,
    last_failure_at TIMESTAMPTZ,
    opened_at TIMESTAMPTZ,
    half_open_at TIMESTAMPTZ,
    updated_at TIMESTAMPTZ NOT NULL DEFAULT NOW()
);

-- V006: Worker registry (for monitoring and coordination)
CREATE TABLE durable_workers (
    id TEXT PRIMARY KEY,  -- Worker ID (e.g., hostname-pid-uuid)
    worker_group TEXT NOT NULL,  -- Logical grouping (e.g., "default", "high-priority")
    activity_types TEXT[] NOT NULL,  -- Types this worker handles

    -- Capacity and load
    max_concurrency INT NOT NULL,
    current_load INT NOT NULL DEFAULT 0,

    -- Status
    status TEXT NOT NULL DEFAULT 'active',  -- active, draining, stopped
    started_at TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    last_heartbeat_at TIMESTAMPTZ NOT NULL DEFAULT NOW(),

    -- Backpressure signaling
    accepting_tasks BOOLEAN NOT NULL DEFAULT true,
    backpressure_reason TEXT,

    -- Metadata
    hostname TEXT,
    version TEXT,
    metadata JSONB
);

CREATE INDEX idx_durable_workers_status ON durable_workers(status) WHERE status = 'active';
CREATE INDEX idx_durable_workers_heartbeat ON durable_workers(last_heartbeat_at);
CREATE INDEX idx_durable_workers_group ON durable_workers(worker_group);

-- V007: Signals queue
CREATE TABLE durable_signals (
    id UUID PRIMARY KEY DEFAULT gen_random_uuid(),
    workflow_id UUID NOT NULL REFERENCES durable_workflow_instances(id),
    signal_type TEXT NOT NULL,
    payload JSONB NOT NULL,
    sent_at TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    processed_at TIMESTAMPTZ,

    -- Ordering
    sequence_num SERIAL
);

CREATE INDEX idx_durable_signals_pending
    ON durable_signals(workflow_id, sequence_num)
    WHERE processed_at IS NULL;
```

### WorkflowEvent Types

```rust
/// Events stored in the durable_workflow_events table
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "type", content = "data")]
pub enum WorkflowEvent {
    // Workflow lifecycle
    WorkflowStarted { input: serde_json::Value },
    WorkflowCompleted { result: serde_json::Value },
    WorkflowFailed { error: WorkflowError },
    WorkflowCancelled { reason: String },

    // Activity lifecycle
    ActivityScheduled {
        activity_id: String,
        activity_type: String,
        input: serde_json::Value,
        options: ActivityOptions,
    },
    ActivityStarted {
        activity_id: String,
        attempt: u32,
        worker_id: String,
    },
    ActivityCompleted {
        activity_id: String,
        result: serde_json::Value,
    },
    ActivityFailed {
        activity_id: String,
        error: ActivityError,
        will_retry: bool,
    },
    ActivityTimedOut {
        activity_id: String,
        timeout_type: TimeoutType,
    },
    ActivityCancelled {
        activity_id: String,
        reason: String,
    },

    // Timers
    TimerStarted { timer_id: String, duration_ms: u64 },
    TimerFired { timer_id: String },
    TimerCancelled { timer_id: String },

    // Signals
    SignalReceived { signal: WorkflowSignal },

    // Child workflows
    ChildWorkflowStarted { workflow_id: Uuid, workflow_type: String },
    ChildWorkflowCompleted { workflow_id: Uuid, result: serde_json::Value },
    ChildWorkflowFailed { workflow_id: Uuid, error: WorkflowError },
}
```

### WorkflowEventStore Trait

```rust
#[async_trait]
pub trait WorkflowEventStore: Send + Sync + 'static {
    /// Create a new workflow instance
    async fn create_workflow(
        &self,
        workflow_id: Uuid,
        workflow_type: &str,
        input: serde_json::Value,
        trace_context: Option<&TraceContext>,
    ) -> Result<(), StoreError>;

    /// Append events to a workflow (with optimistic concurrency)
    async fn append_events(
        &self,
        workflow_id: Uuid,
        expected_sequence: i32,
        events: Vec<WorkflowEvent>,
    ) -> Result<i32, StoreError>;

    /// Load all events for a workflow (for replay)
    async fn load_events(
        &self,
        workflow_id: Uuid,
    ) -> Result<Vec<(i32, WorkflowEvent)>, StoreError>;

    /// Update workflow status
    async fn update_workflow_status(
        &self,
        workflow_id: Uuid,
        status: WorkflowStatus,
        result: Option<serde_json::Value>,
        error: Option<WorkflowError>,
    ) -> Result<(), StoreError>;

    // Task queue operations

    /// Enqueue an activity task
    async fn enqueue_task(&self, task: TaskDefinition) -> Result<Uuid, StoreError>;

    /// Claim a task for execution (SELECT FOR UPDATE SKIP LOCKED)
    /// Returns None if no tasks available or backpressure active
    async fn claim_task(
        &self,
        worker_id: &str,
        activity_types: &[String],
        max_tasks: usize,  // Batch claiming for efficiency
    ) -> Result<Vec<ClaimedTask>, StoreError>;

    /// Record task heartbeat
    async fn heartbeat_task(
        &self,
        task_id: Uuid,
        worker_id: &str,
        details: Option<serde_json::Value>,
    ) -> Result<HeartbeatResponse, StoreError>;

    /// Complete a task
    async fn complete_task(
        &self,
        task_id: Uuid,
        result: serde_json::Value,
    ) -> Result<(), StoreError>;

    /// Fail a task (may requeue or send to DLQ)
    async fn fail_task(
        &self,
        task_id: Uuid,
        error: &str,
    ) -> Result<TaskFailureOutcome, StoreError>;

    /// Find and reclaim stale tasks (no heartbeat)
    async fn reclaim_stale_tasks(
        &self,
        stale_threshold: Duration,
    ) -> Result<Vec<Uuid>, StoreError>;

    // Signal operations

    /// Send a signal to a workflow
    async fn send_signal(
        &self,
        workflow_id: Uuid,
        signal: WorkflowSignal,
    ) -> Result<(), StoreError>;

    /// Get pending signals for a workflow
    async fn get_pending_signals(
        &self,
        workflow_id: Uuid,
    ) -> Result<Vec<WorkflowSignal>, StoreError>;

    // Worker registry operations

    /// Register a worker
    async fn register_worker(&self, worker: WorkerInfo) -> Result<(), StoreError>;

    /// Update worker heartbeat and load
    async fn worker_heartbeat(
        &self,
        worker_id: &str,
        current_load: usize,
        accepting_tasks: bool,
    ) -> Result<(), StoreError>;

    /// Get all active workers
    async fn list_workers(&self, filter: WorkerFilter) -> Result<Vec<WorkerInfo>, StoreError>;

    /// Deregister a worker
    async fn deregister_worker(&self, worker_id: &str) -> Result<(), StoreError>;

    // Dead letter queue operations

    /// Move task to DLQ
    async fn move_to_dlq(
        &self,
        task_id: Uuid,
        error_history: Vec<String>,
    ) -> Result<(), StoreError>;

    /// Requeue task from DLQ
    async fn requeue_from_dlq(&self, dlq_id: Uuid) -> Result<Uuid, StoreError>;

    /// List DLQ entries (for admin UI)
    async fn list_dlq(
        &self,
        filter: DlqFilter,
        pagination: Pagination,
    ) -> Result<Vec<DlqEntry>, StoreError>;
}
```

---

## Scalability: Supporting 1000+ Workers

### Bottleneck Analysis

| Component | Potential Bottleneck | Mitigation |
|-----------|---------------------|------------|
| **Task claiming** | Lock contention on `SELECT FOR UPDATE` | Partition by `activity_type`, batch claims, SKIP LOCKED |
| **Heartbeat updates** | Write amplification | Batch heartbeats, optimistic updates |
| **Event appending** | Sequence number conflicts | Per-workflow optimistic locking (no global lock) |
| **Worker registry** | Frequent heartbeat writes | Batch updates, longer intervals with jitter |
| **Connection pool** | Pool exhaustion | Separate pools per concern, connection limits |

### Critical Path: Task Claiming

The task claiming query is the most critical for scalability:

```sql
-- Optimized for 1000 workers claiming tasks concurrently
-- Each worker claims tasks for specific activity_types, reducing contention

WITH claimed AS (
    SELECT id
    FROM durable_task_queue
    WHERE status = 'pending'
      AND activity_type = ANY($1)  -- Worker's activity types
      AND visible_at <= NOW()
    ORDER BY priority DESC, visible_at ASC
    LIMIT $2  -- Batch size (e.g., 5)
    FOR UPDATE SKIP LOCKED
)
UPDATE durable_task_queue
SET status = 'claimed',
    claimed_by = $3,
    claimed_at = NOW(),
    heartbeat_at = NOW(),
    attempt = attempt + 1
WHERE id IN (SELECT id FROM claimed)
RETURNING *;
```

**Key optimizations:**
1. `SKIP LOCKED` - Workers don't block each other
2. `activity_type` partitioning - Reduces row scanning
3. Batch claiming - Fewer round trips
4. Partial index on `status = 'pending'` - Smaller index

### Worker Heartbeat Batching

```rust
/// Batched heartbeat sender to reduce DB writes
pub struct HeartbeatBatcher {
    pending: DashMap<Uuid, HeartbeatData>,
    flush_interval: Duration,
}

impl HeartbeatBatcher {
    /// Queue a heartbeat (non-blocking)
    pub fn queue(&self, task_id: Uuid, details: Option<serde_json::Value>) {
        self.pending.insert(task_id, HeartbeatData {
            details,
            queued_at: Instant::now(),
        });
    }

    /// Flush all pending heartbeats in a single transaction
    async fn flush(&self, store: &dyn WorkflowEventStore) -> Result<(), StoreError> {
        let batch: Vec<_> = self.pending.iter()
            .map(|e| (*e.key(), e.value().clone()))
            .collect();

        self.pending.clear();

        // Single batch UPDATE
        store.batch_heartbeat(&batch).await
    }
}
```

### Connection Pool Configuration

```rust
/// Database pool configuration for high-concurrency
pub struct PoolConfig {
    /// Pool for task claiming (high contention)
    pub task_pool_size: u32,  // e.g., 50

    /// Pool for event operations (low contention)
    pub event_pool_size: u32,  // e.g., 20

    /// Pool for worker registry (very low contention)
    pub registry_pool_size: u32,  // e.g., 10

    /// Connection timeout
    pub connect_timeout: Duration,

    /// Idle connection timeout
    pub idle_timeout: Duration,
}

impl Default for PoolConfig {
    fn default() -> Self {
        Self {
            task_pool_size: 50,
            event_pool_size: 20,
            registry_pool_size: 10,
            connect_timeout: Duration::from_secs(5),
            idle_timeout: Duration::from_secs(300),
        }
    }
}
```

---

## Backpressure

### Worker-Side Backpressure

```rust
/// Backpressure configuration
#[derive(Debug, Clone)]
pub struct BackpressureConfig {
    /// High watermark - stop accepting tasks
    pub high_watermark: f64,  // e.g., 0.9 (90% of max_concurrency)

    /// Low watermark - resume accepting tasks
    pub low_watermark: f64,   // e.g., 0.7 (70% of max_concurrency)

    /// Memory pressure threshold (bytes)
    pub memory_threshold: Option<usize>,

    /// CPU pressure threshold (percentage)
    pub cpu_threshold: Option<f64>,
}

/// Backpressure state for a worker
pub struct BackpressureState {
    config: BackpressureConfig,
    current_load: AtomicUsize,
    max_concurrency: usize,
    accepting_tasks: AtomicBool,
}

impl BackpressureState {
    /// Check if worker should accept new tasks
    pub fn should_accept(&self) -> bool {
        if !self.accepting_tasks.load(Ordering::Relaxed) {
            // Check if we've dropped below low watermark
            let load_ratio = self.current_load.load(Ordering::Relaxed) as f64
                / self.max_concurrency as f64;

            if load_ratio < self.config.low_watermark {
                self.accepting_tasks.store(true, Ordering::Relaxed);
                return true;
            }
            return false;
        }

        // Check if we've exceeded high watermark
        let load_ratio = self.current_load.load(Ordering::Relaxed) as f64
            / self.max_concurrency as f64;

        if load_ratio >= self.config.high_watermark {
            self.accepting_tasks.store(false, Ordering::Relaxed);
            return false;
        }

        true
    }

    /// Record task start
    pub fn task_started(&self) {
        self.current_load.fetch_add(1, Ordering::Relaxed);
    }

    /// Record task completion
    pub fn task_completed(&self) {
        self.current_load.fetch_sub(1, Ordering::Relaxed);
    }
}
```

### System-Wide Backpressure

```rust
/// Global backpressure coordinator
pub struct BackpressureCoordinator {
    store: Arc<dyn WorkflowEventStore>,
}

impl BackpressureCoordinator {
    /// Check global system health
    pub async fn check_system_health(&self) -> SystemHealth {
        let workers = self.store.list_workers(WorkerFilter::active()).await?;

        let total_capacity: usize = workers.iter()
            .filter(|w| w.accepting_tasks)
            .map(|w| w.max_concurrency - w.current_load)
            .sum();

        let pending_tasks = self.store.count_pending_tasks().await?;

        SystemHealth {
            available_capacity: total_capacity,
            pending_tasks,
            accepting_workers: workers.iter().filter(|w| w.accepting_tasks).count(),
            total_workers: workers.len(),
        }
    }

    /// Should new workflows be accepted?
    pub async fn should_accept_workflow(&self) -> bool {
        let health = self.check_system_health().await?;

        // Don't accept if queue is too deep relative to capacity
        let queue_depth_ratio = health.pending_tasks as f64
            / health.available_capacity.max(1) as f64;

        queue_depth_ratio < 10.0  // Max 10x capacity in queue
    }
}
```

### Backpressure Response in Poller

```rust
impl WorkerPool {
    async fn claim_task(&self) -> Result<Option<ClaimedTask>, WorkerError> {
        // Check local backpressure first
        if !self.backpressure.should_accept() {
            tracing::debug!("Worker under backpressure, skipping poll");
            return Ok(None);
        }

        // Update worker status in registry
        self.store.worker_heartbeat(
            &self.config.worker_id,
            self.backpressure.current_load(),
            self.backpressure.accepting_tasks(),
        ).await?;

        // Claim task
        let tasks = self.store.claim_task(
            &self.config.worker_id,
            &self.config.activity_types,
            1,  // Claim one at a time when under pressure
        ).await?;

        Ok(tasks.into_iter().next())
    }
}
```

---

## Observability: OpenTelemetry Integration

### Semantic Conventions

```rust
/// Durable execution semantic conventions for OpenTelemetry
pub mod semantic {
    // Workflow attributes
    pub const WORKFLOW_ID: &str = "durable.workflow.id";
    pub const WORKFLOW_TYPE: &str = "durable.workflow.type";
    pub const WORKFLOW_STATUS: &str = "durable.workflow.status";
    pub const WORKFLOW_RUN_ID: &str = "durable.workflow.run_id";

    // Activity attributes
    pub const ACTIVITY_ID: &str = "durable.activity.id";
    pub const ACTIVITY_TYPE: &str = "durable.activity.type";
    pub const ACTIVITY_ATTEMPT: &str = "durable.activity.attempt";
    pub const ACTIVITY_MAX_ATTEMPTS: &str = "durable.activity.max_attempts";

    // Worker attributes
    pub const WORKER_ID: &str = "durable.worker.id";
    pub const WORKER_GROUP: &str = "durable.worker.group";
    pub const WORKER_CONCURRENCY: &str = "durable.worker.concurrency";
    pub const WORKER_LOAD: &str = "durable.worker.load";

    // Task queue attributes
    pub const TASK_QUEUE_DEPTH: &str = "durable.task_queue.depth";
    pub const TASK_PRIORITY: &str = "durable.task.priority";
    pub const TASK_SCHEDULE_TO_START_LATENCY: &str = "durable.task.schedule_to_start_latency_ms";

    // Circuit breaker attributes
    pub const CIRCUIT_BREAKER_KEY: &str = "durable.circuit_breaker.key";
    pub const CIRCUIT_BREAKER_STATE: &str = "durable.circuit_breaker.state";

    /// Span names following OTel conventions
    pub mod span {
        pub const WORKFLOW_RUN: &str = "durable.workflow.run";
        pub const ACTIVITY_EXECUTE: &str = "durable.activity.execute";
        pub const TASK_CLAIM: &str = "durable.task.claim";
        pub const TASK_COMPLETE: &str = "durable.task.complete";
        pub const EVENT_APPEND: &str = "durable.event.append";
        pub const WORKFLOW_REPLAY: &str = "durable.workflow.replay";
    }
}
```

### Tracing Implementation

```rust
use opentelemetry::{trace::{Tracer, SpanKind, Status}, Context, KeyValue};
use tracing_opentelemetry::OpenTelemetrySpanExt;

/// Create a span for workflow execution
pub fn workflow_span(
    tracer: &impl Tracer,
    workflow_id: Uuid,
    workflow_type: &str,
) -> opentelemetry::trace::Span {
    tracer
        .span_builder(semantic::span::WORKFLOW_RUN)
        .with_kind(SpanKind::Internal)
        .with_attributes([
            KeyValue::new(semantic::WORKFLOW_ID, workflow_id.to_string()),
            KeyValue::new(semantic::WORKFLOW_TYPE, workflow_type.to_string()),
        ])
        .start(tracer)
}

/// Create a span for activity execution (child of workflow span)
pub fn activity_span(
    tracer: &impl Tracer,
    parent_context: &Context,
    activity_id: &str,
    activity_type: &str,
    attempt: u32,
) -> opentelemetry::trace::Span {
    tracer
        .span_builder(semantic::span::ACTIVITY_EXECUTE)
        .with_kind(SpanKind::Internal)
        .with_attributes([
            KeyValue::new(semantic::ACTIVITY_ID, activity_id.to_string()),
            KeyValue::new(semantic::ACTIVITY_TYPE, activity_type.to_string()),
            KeyValue::new(semantic::ACTIVITY_ATTEMPT, attempt as i64),
        ])
        .start_with_context(tracer, parent_context)
}

/// Trace context propagation for database storage
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TraceContext {
    pub trace_id: String,
    pub span_id: String,
    pub trace_flags: u8,
}

impl TraceContext {
    pub fn from_current() -> Option<Self> {
        use opentelemetry::trace::TraceContextExt;
        let span = tracing::Span::current();
        let context = span.context();
        let span_context = context.span().span_context();

        if span_context.is_valid() {
            Some(Self {
                trace_id: span_context.trace_id().to_string(),
                span_id: span_context.span_id().to_string(),
                trace_flags: span_context.trace_flags().to_u8(),
            })
        } else {
            None
        }
    }

    pub fn to_context(&self) -> Context {
        // Reconstruct OTel context from stored trace IDs
        // Used when replaying workflows or resuming activities
    }
}
```

### Metrics

```rust
use opentelemetry::metrics::{Counter, Histogram, UpDownCounter};

/// Metrics for the durable execution engine
pub struct DurableMetrics {
    // Workflow metrics
    pub workflows_started: Counter<u64>,
    pub workflows_completed: Counter<u64>,
    pub workflows_failed: Counter<u64>,
    pub workflow_duration: Histogram<f64>,

    // Activity metrics
    pub activities_scheduled: Counter<u64>,
    pub activities_completed: Counter<u64>,
    pub activities_failed: Counter<u64>,
    pub activities_retried: Counter<u64>,
    pub activity_duration: Histogram<f64>,
    pub activity_schedule_to_start_latency: Histogram<f64>,

    // Task queue metrics
    pub task_queue_depth: UpDownCounter<i64>,
    pub tasks_claimed: Counter<u64>,
    pub tasks_claim_latency: Histogram<f64>,

    // Worker metrics
    pub active_workers: UpDownCounter<i64>,
    pub worker_load: Histogram<f64>,
    pub workers_under_backpressure: UpDownCounter<i64>,

    // Circuit breaker metrics
    pub circuit_breaker_opens: Counter<u64>,
    pub circuit_breaker_state: UpDownCounter<i64>,  // By key

    // DLQ metrics
    pub dlq_entries: UpDownCounter<i64>,
    pub dlq_requeued: Counter<u64>,
}

impl DurableMetrics {
    pub fn new(meter: &opentelemetry::metrics::Meter) -> Self {
        Self {
            workflows_started: meter
                .u64_counter("durable.workflows.started")
                .with_description("Number of workflows started")
                .build(),
            // ... other metrics
        }
    }
}
```

---

## Monitoring UI and Admin API

### Admin API Endpoints

```rust
/// Admin API router
pub fn admin_router(state: AdminState) -> Router {
    Router::new()
        // Worker management
        .route("/api/durable/workers", get(list_workers))
        .route("/api/durable/workers/:id", get(get_worker))
        .route("/api/durable/workers/:id/drain", post(drain_worker))

        // Workflow inspection
        .route("/api/durable/workflows", get(list_workflows))
        .route("/api/durable/workflows/:id", get(get_workflow))
        .route("/api/durable/workflows/:id/events", get(get_workflow_events))
        .route("/api/durable/workflows/:id/signal", post(send_signal))
        .route("/api/durable/workflows/:id/cancel", post(cancel_workflow))

        // Task queue
        .route("/api/durable/tasks", get(list_tasks))
        .route("/api/durable/tasks/stats", get(get_task_stats))

        // Dead letter queue
        .route("/api/durable/dlq", get(list_dlq))
        .route("/api/durable/dlq/:id/requeue", post(requeue_dlq))
        .route("/api/durable/dlq/:id", delete(delete_dlq))
        .route("/api/durable/dlq/purge", post(purge_dlq))

        // Circuit breakers
        .route("/api/durable/circuit-breakers", get(list_circuit_breakers))
        .route("/api/durable/circuit-breakers/:key/reset", post(reset_circuit_breaker))

        // System health
        .route("/api/durable/health", get(system_health))
        .route("/api/durable/metrics", get(prometheus_metrics))

        .with_state(state)
}
```

### Admin API Types

```rust
/// Worker information returned by admin API
#[derive(Debug, Serialize, Deserialize, ToSchema)]
pub struct WorkerInfo {
    pub id: String,
    pub worker_group: String,
    pub activity_types: Vec<String>,
    pub max_concurrency: u32,
    pub current_load: u32,
    pub status: WorkerStatus,
    pub accepting_tasks: bool,
    pub backpressure_reason: Option<String>,
    pub started_at: DateTime<Utc>,
    pub last_heartbeat_at: DateTime<Utc>,
    pub hostname: Option<String>,
    pub version: Option<String>,

    // Live stats
    pub tasks_completed: u64,
    pub tasks_failed: u64,
    pub avg_task_duration_ms: f64,
}

#[derive(Debug, Serialize, Deserialize, ToSchema)]
pub enum WorkerStatus {
    Active,
    Draining,
    Stopped,
    Stale,  // No heartbeat
}

/// System health summary
#[derive(Debug, Serialize, Deserialize, ToSchema)]
pub struct SystemHealth {
    pub status: HealthStatus,

    // Workers
    pub total_workers: usize,
    pub active_workers: usize,
    pub workers_accepting: usize,
    pub total_capacity: usize,
    pub current_load: usize,
    pub load_percentage: f64,

    // Task queue
    pub pending_tasks: usize,
    pub claimed_tasks: usize,
    pub queue_depth_by_type: HashMap<String, usize>,

    // Workflows
    pub running_workflows: usize,
    pub pending_workflows: usize,

    // DLQ
    pub dlq_size: usize,

    // Circuit breakers
    pub open_circuit_breakers: Vec<String>,
}

/// Task queue statistics
#[derive(Debug, Serialize, Deserialize, ToSchema)]
pub struct TaskQueueStats {
    pub by_activity_type: HashMap<String, ActivityTypeStats>,
    pub by_priority: HashMap<i32, usize>,
    pub oldest_pending_task_age_ms: u64,
    pub avg_schedule_to_start_ms: f64,
    pub avg_execution_time_ms: f64,
}

#[derive(Debug, Serialize, Deserialize, ToSchema)]
pub struct ActivityTypeStats {
    pub pending: usize,
    pub claimed: usize,
    pub completed_last_hour: usize,
    pub failed_last_hour: usize,
    pub avg_duration_ms: f64,
    pub p99_duration_ms: f64,
}
```

### Example API Responses

```json
// GET /api/durable/workers
{
  "workers": [
    {
      "id": "worker-abc123",
      "worker_group": "default",
      "activity_types": ["input", "reason", "act"],
      "max_concurrency": 10,
      "current_load": 7,
      "status": "active",
      "accepting_tasks": true,
      "started_at": "2024-01-15T10:30:00Z",
      "last_heartbeat_at": "2024-01-15T14:25:30Z",
      "hostname": "worker-node-1",
      "version": "0.1.0",
      "tasks_completed": 1523,
      "tasks_failed": 12,
      "avg_task_duration_ms": 2340.5
    }
  ],
  "total": 1,
  "summary": {
    "active": 1,
    "draining": 0,
    "stopped": 0,
    "total_capacity": 10,
    "total_load": 7
  }
}

// GET /api/durable/health
{
  "status": "healthy",
  "total_workers": 5,
  "active_workers": 5,
  "workers_accepting": 4,
  "total_capacity": 50,
  "current_load": 35,
  "load_percentage": 70.0,
  "pending_tasks": 120,
  "claimed_tasks": 35,
  "queue_depth_by_type": {
    "reason": 80,
    "act": 30,
    "input": 10
  },
  "running_workflows": 150,
  "pending_workflows": 20,
  "dlq_size": 3,
  "open_circuit_breakers": []
}
```

---

## Reliability Layer

### 1. Retry Policy

```rust
/// Configuration for activity retries
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RetryPolicy {
    /// Maximum number of attempts (including initial)
    pub max_attempts: u32,

    /// Initial delay before first retry
    pub initial_interval: Duration,

    /// Maximum delay between retries
    pub max_interval: Duration,

    /// Backoff multiplier (e.g., 2.0 for exponential)
    pub backoff_coefficient: f64,

    /// Jitter factor (0.0-1.0) to add randomness
    pub jitter: f64,

    /// Errors that should NOT be retried
    pub non_retryable_errors: Vec<String>,
}

impl RetryPolicy {
    pub fn exponential() -> Self {
        Self {
            max_attempts: 5,
            initial_interval: Duration::from_secs(1),
            max_interval: Duration::from_secs(60),
            backoff_coefficient: 2.0,
            jitter: 0.1,
            non_retryable_errors: vec![],
        }
    }

    /// Calculate delay for a given attempt number
    pub fn delay_for_attempt(&self, attempt: u32) -> Duration {
        let base = self.initial_interval.as_secs_f64()
            * self.backoff_coefficient.powi(attempt as i32 - 1);
        let capped = base.min(self.max_interval.as_secs_f64());
        let jittered = capped * (1.0 + (rand::random::<f64>() - 0.5) * 2.0 * self.jitter);
        Duration::from_secs_f64(jittered)
    }
}
```

### 2. Circuit Breaker

```rust
/// Circuit breaker configuration
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CircuitBreakerConfig {
    /// Failure threshold to open circuit
    pub failure_threshold: u32,

    /// Success threshold to close circuit (in half-open state)
    pub success_threshold: u32,

    /// Time to wait before transitioning to half-open
    pub reset_timeout: Duration,

    /// Sliding window size for failure counting
    pub window_size: Duration,
}

/// Circuit breaker states
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum CircuitState {
    Closed,
    Open,
    HalfOpen,
}

/// Distributed circuit breaker (state shared via database)
pub struct DistributedCircuitBreaker {
    key: String,
    config: CircuitBreakerConfig,
    store: Arc<dyn WorkflowEventStore>,
    local_cache: RwLock<Option<CachedState>>,
}

impl DistributedCircuitBreaker {
    /// Check if call should be allowed
    pub async fn allow(&self) -> Result<CircuitBreakerPermit, CircuitBreakerError> {
        let state = self.get_state().await?;

        match state {
            CircuitState::Closed => Ok(CircuitBreakerPermit::new(self)),
            CircuitState::Open => {
                // Check if reset_timeout has passed
                if self.should_try_half_open().await? {
                    self.transition_to_half_open().await?;
                    Ok(CircuitBreakerPermit::new(self))
                } else {
                    Err(CircuitBreakerError::Open)
                }
            }
            CircuitState::HalfOpen => {
                // Allow limited calls in half-open
                Ok(CircuitBreakerPermit::new(self))
            }
        }
    }

    /// Record a successful call
    pub async fn record_success(&self) -> Result<(), StoreError>;

    /// Record a failed call
    pub async fn record_failure(&self) -> Result<(), StoreError>;
}
```

### 3. Timeout Manager

```rust
/// Manages activity timeouts
pub struct TimeoutManager {
    store: Arc<dyn WorkflowEventStore>,
    metrics: Arc<DurableMetrics>,
}

impl TimeoutManager {
    /// Spawn background task to check for timeouts
    pub fn spawn(self) -> JoinHandle<()> {
        tokio::spawn(async move {
            let mut interval = tokio::time::interval(Duration::from_secs(1));
            loop {
                interval.tick().await;
                if let Err(e) = self.check_timeouts().await {
                    tracing::error!(error = %e, "Timeout check failed");
                }
            }
        })
    }

    async fn check_timeouts(&self) -> Result<(), StoreError> {
        // 1. Check schedule_to_start timeouts (pending too long)
        let stale_pending = self.store
            .find_stale_pending_tasks(Duration::from_secs(60))
            .await?;

        for task in stale_pending {
            self.handle_schedule_to_start_timeout(task).await?;
        }

        // 2. Check start_to_close timeouts (running too long)
        let overtime_tasks = self.store
            .find_overtime_tasks()
            .await?;

        for task in overtime_tasks {
            self.handle_start_to_close_timeout(task).await?;
        }

        // 3. Check heartbeat timeouts (no heartbeat)
        let stale_tasks = self.store
            .reclaim_stale_tasks(Duration::from_secs(30))
            .await?;

        for task_id in stale_tasks {
            tracing::warn!(task_id = %task_id, "Reclaimed stale task");
        }

        Ok(())
    }
}
```

---

## Implementation Phases

### Phase 1: Core Abstractions
- [ ] Workflow and Activity traits
- [ ] WorkflowAction enum
- [ ] WorkflowSignal for cancellation
- [ ] In-memory WorkflowEventStore for testing
- [ ] Basic WorkflowExecutor (no persistence)
- [ ] Unit tests for state machines

### Phase 2: Persistence Layer
- [ ] PostgreSQL migrations (with `durable_` prefix)
- [ ] PostgresWorkflowEventStore implementation
- [ ] Task queue with claim/complete (optimized for scale)
- [ ] WorkflowEvent replay for recovery
- [ ] Integration tests with PostgreSQL

### Phase 3: Reliability Features
- [ ] RetryPolicy implementation
- [ ] CircuitBreaker implementation (distributed)
- [ ] Timeout manager
- [ ] Dead letter queue
- [ ] Integration tests for reliability

### Phase 4: Worker Pool & Backpressure
- [ ] WorkerPool with concurrency control
- [ ] Worker registry and heartbeat
- [ ] Backpressure implementation
- [ ] Heartbeat batching
- [ ] Graceful shutdown and draining
- [ ] Stale task reclamation

### Phase 5: Observability
- [ ] OpenTelemetry tracing integration
- [ ] Metrics (Prometheus/OTel)
- [ ] Admin API endpoints
- [ ] Trace context propagation

### Phase 6: Scale Testing
- [ ] 1000-worker scale test
- [ ] Task claiming benchmark
- [ ] Connection pool tuning
- [ ] Performance regression tests

### Phase 7: Integration with Everruns
- [ ] Migrate TurnWorkflow to new engine
- [ ] Update worker crate to use durable engine
- [ ] Remove Temporal dependencies
- [ ] End-to-end smoke tests

---

## Decisions

### Partitioning Strategy

**Decision**: No custom partitioning in v1. PostgreSQL handles it.

**Rationale**:
- PostgreSQL with proper indexes can handle 10,000+ tasks/second
- `SKIP LOCKED` eliminates contention
- Activity-type-based claiming naturally partitions work
- If we hit limits, PostgreSQL native partitioning can be added without code changes
- Sharding adds complexity that isn't needed at our scale

### Workflow Versioning

**Decision**: Not in v1. To be added when needed.

**Rationale**:
- Running workflows complete with their original code
- New workflows use new code
- In-place migration is complex and error-prone
- When needed, will implement replay-based migration

### Signals

**Decision**: Yes, implement signals for workflow communication.

**Use cases**:
- Cancel a running workflow
- Request graceful shutdown
- External events that affect workflow behavior

---

## Dependencies

```toml
[package]
name = "everruns-durable"
version = "0.1.0"
edition = "2024"

[dependencies]
# Async runtime
tokio = { version = "1", features = ["full"] }
async-trait = "0.1"
tokio-util = { version = "0.7", features = ["rt"] }

# Database
sqlx = { version = "0.8", features = ["runtime-tokio", "postgres", "uuid", "chrono", "json"] }

# Serialization
serde = { version = "1", features = ["derive"] }
serde_json = "1"

# Utilities
uuid = { version = "1", features = ["v7", "serde"] }
chrono = { version = "0.4", features = ["serde"] }
thiserror = "2"
anyhow = "1"
rand = "0.8"
dashmap = "6"

# Observability (aligned with everruns-core)
tracing.workspace = true
opentelemetry.workspace = true
opentelemetry_sdk.workspace = true
opentelemetry-otlp.workspace = true
tracing-opentelemetry.workspace = true

# HTTP for admin API
axum = { version = "0.8", features = ["macros"] }
tower = "0.5"

[dev-dependencies]
criterion = { version = "0.5", features = ["async_tokio"] }
tokio-test = "0.4"
testcontainers = "0.23"
```
