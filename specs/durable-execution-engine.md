# Durable Execution Engine Specification

## Abstract

This specification describes a custom durable execution engine to replace Temporal. The engine provides workflow orchestration with persistence, automatic retries, circuit breakers, and distributed task execution - all backed by PostgreSQL.

## Goals

1. **Self-contained crate** - `everruns-durable` with no Temporal dependencies
2. **PostgreSQL-only persistence** - No additional infrastructure required
3. **Testable in isolation** - Unit tests, integration tests, load/stress tests
4. **Production-ready reliability** - Retries, circuit breakers, timeouts, dead letter queues
5. **Simple mental model** - Event-sourced workflows with explicit state machines

## Non-Goals

1. Multi-region replication (use PostgreSQL replication)
2. Language-agnostic SDKs (Rust only)
3. Visual workflow designer
4. Versioning/migration of running workflows (v1 scope)

---

## Architecture Overview

```
┌─────────────────────────────────────────────────────────────────────────┐
│                          everruns-durable                                │
├─────────────────────────────────────────────────────────────────────────┤
│                                                                          │
│  ┌──────────────┐  ┌──────────────┐  ┌──────────────┐  ┌─────────────┐ │
│  │   Workflow   │  │   Activity   │  │   Worker     │  │  Scheduler  │ │
│  │   Engine     │  │   Executor   │  │   Pool       │  │             │ │
│  └──────┬───────┘  └──────┬───────┘  └──────┬───────┘  └──────┬──────┘ │
│         │                 │                 │                 │         │
│  ┌──────┴─────────────────┴─────────────────┴─────────────────┴──────┐ │
│  │                         Event Store                                │ │
│  │  (PostgreSQL: workflow_instances, workflow_events, task_queue)    │ │
│  └────────────────────────────────────────────────────────────────────┘ │
│                                                                          │
│  ┌──────────────────────────────────────────────────────────────────┐   │
│  │                      Reliability Layer                            │   │
│  │  ┌────────────┐  ┌────────────┐  ┌────────────┐  ┌────────────┐  │   │
│  │  │   Retry    │  │  Circuit   │  │  Timeout   │  │    DLQ     │  │   │
│  │  │   Policy   │  │  Breaker   │  │  Manager   │  │  Handler   │  │   │
│  │  └────────────┘  └────────────┘  └────────────┘  └────────────┘  │   │
│  └──────────────────────────────────────────────────────────────────┘   │
└─────────────────────────────────────────────────────────────────────────┘
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
│   │   └── replay.rs          # Event replay for recovery
│   │
│   ├── persistence/           # Database layer
│   │   ├── mod.rs
│   │   ├── store.rs           # EventStore trait
│   │   ├── postgres.rs        # PostgreSQL implementation
│   │   ├── memory.rs          # In-memory for testing
│   │   └── migrations/        # SQL migrations
│   │       ├── V001__workflow_instances.sql
│   │       ├── V002__workflow_events.sql
│   │       ├── V003__task_queue.sql
│   │       └── V004__dead_letter_queue.sql
│   │
│   ├── reliability/           # Resilience patterns
│   │   ├── mod.rs
│   │   ├── retry.rs           # RetryPolicy, exponential backoff
│   │   ├── circuit_breaker.rs # CircuitBreaker state machine
│   │   ├── timeout.rs         # Timeout handling
│   │   ├── rate_limiter.rs    # Rate limiting for activities
│   │   └── dlq.rs             # Dead letter queue
│   │
│   ├── worker/                # Worker process
│   │   ├── mod.rs
│   │   ├── pool.rs            # WorkerPool - manages worker threads
│   │   ├── poller.rs          # Task polling with backoff
│   │   └── heartbeat.rs       # Worker liveness
│   │
│   └── metrics/               # Observability
│       ├── mod.rs
│       └── prometheus.rs      # Metrics export
│
├── tests/
│   ├── integration/           # Integration tests
│   │   ├── mod.rs
│   │   ├── workflow_execution.rs
│   │   ├── activity_retry.rs
│   │   ├── circuit_breaker.rs
│   │   └── recovery.rs
│   │
│   └── fixtures/              # Test utilities
│       ├── mod.rs
│       ├── test_workflows.rs
│       └── test_activities.rs
│
├── benches/                   # Performance benchmarks
│   ├── workflow_throughput.rs
│   ├── activity_latency.rs
│   └── concurrent_workers.rs
│
└── examples/
    ├── simple_workflow.rs
    ├── retry_example.rs
    └── circuit_breaker_example.rs
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
}
```

### 3. Activity Trait

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

    /// Heartbeat sender for long-running activities
    heartbeat_tx: mpsc::Sender<HeartbeatPayload>,
}

impl ActivityContext {
    /// Record a heartbeat (prevents timeout for long activities)
    pub async fn heartbeat(&self, details: Option<serde_json::Value>) -> Result<(), HeartbeatError>;

    /// Check if cancellation was requested
    pub fn is_cancelled(&self) -> bool;
}
```

---

## Persistence Layer

### Database Schema

```sql
-- V001: Workflow instances
CREATE TABLE workflow_instances (
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

    -- Partition key for scaling (optional)
    partition_key INT NOT NULL DEFAULT 0
);

CREATE INDEX idx_workflow_instances_status ON workflow_instances(status);
CREATE INDEX idx_workflow_instances_type ON workflow_instances(workflow_type);

-- V002: Workflow events (append-only log)
CREATE TABLE workflow_events (
    id BIGSERIAL PRIMARY KEY,
    workflow_id UUID NOT NULL REFERENCES workflow_instances(id),
    sequence_num INT NOT NULL,  -- Per-workflow sequence number
    event_type TEXT NOT NULL,
    event_data JSONB NOT NULL,
    created_at TIMESTAMPTZ NOT NULL DEFAULT NOW(),

    UNIQUE(workflow_id, sequence_num)
);

CREATE INDEX idx_workflow_events_workflow ON workflow_events(workflow_id, sequence_num);

-- V003: Task queue (for activity scheduling)
CREATE TABLE task_queue (
    id UUID PRIMARY KEY DEFAULT gen_random_uuid(),
    workflow_id UUID NOT NULL REFERENCES workflow_instances(id),
    activity_id TEXT NOT NULL,
    activity_type TEXT NOT NULL,
    input JSONB NOT NULL,
    options JSONB NOT NULL,

    -- Scheduling
    status TEXT NOT NULL DEFAULT 'pending',  -- pending, claimed, completed, failed, dead
    priority INT NOT NULL DEFAULT 0,
    scheduled_at TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    visible_at TIMESTAMPTZ NOT NULL DEFAULT NOW(),  -- For delayed retry

    -- Claiming
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

    created_at TIMESTAMPTZ NOT NULL DEFAULT NOW()
);

-- Efficient polling query
CREATE INDEX idx_task_queue_pending ON task_queue(visible_at, priority DESC)
    WHERE status = 'pending';
CREATE INDEX idx_task_queue_claimed ON task_queue(claimed_by, heartbeat_at)
    WHERE status = 'claimed';
CREATE INDEX idx_task_queue_workflow ON task_queue(workflow_id);

-- V004: Dead letter queue
CREATE TABLE dead_letter_queue (
    id UUID PRIMARY KEY DEFAULT gen_random_uuid(),
    original_task_id UUID NOT NULL,
    workflow_id UUID NOT NULL REFERENCES workflow_instances(id),
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

CREATE INDEX idx_dlq_workflow ON dead_letter_queue(workflow_id);
CREATE INDEX idx_dlq_activity_type ON dead_letter_queue(activity_type);

-- V005: Circuit breaker state
CREATE TABLE circuit_breaker_state (
    key TEXT PRIMARY KEY,  -- e.g., "activity:llm_call" or "external:openai"
    state TEXT NOT NULL DEFAULT 'closed',  -- closed, open, half_open
    failure_count INT NOT NULL DEFAULT 0,
    success_count INT NOT NULL DEFAULT 0,
    last_failure_at TIMESTAMPTZ,
    opened_at TIMESTAMPTZ,
    half_open_at TIMESTAMPTZ,
    updated_at TIMESTAMPTZ NOT NULL DEFAULT NOW()
);
```

### Event Types

```rust
/// Events stored in the workflow_events table
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

    // Timers
    TimerStarted { timer_id: String, duration_ms: u64 },
    TimerFired { timer_id: String },
    TimerCancelled { timer_id: String },

    // Child workflows
    ChildWorkflowStarted { workflow_id: Uuid, workflow_type: String },
    ChildWorkflowCompleted { workflow_id: Uuid, result: serde_json::Value },
    ChildWorkflowFailed { workflow_id: Uuid, error: WorkflowError },
}
```

### EventStore Trait

```rust
#[async_trait]
pub trait EventStore: Send + Sync + 'static {
    /// Create a new workflow instance
    async fn create_workflow(
        &self,
        workflow_id: Uuid,
        workflow_type: &str,
        input: serde_json::Value,
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
    async fn claim_task(
        &self,
        worker_id: &str,
        activity_types: &[String],
    ) -> Result<Option<ClaimedTask>, StoreError>;

    /// Record task heartbeat
    async fn heartbeat_task(
        &self,
        task_id: Uuid,
        worker_id: &str,
        details: Option<serde_json::Value>,
    ) -> Result<(), StoreError>;

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
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CircuitState {
    /// Normal operation - all calls allowed
    Closed,

    /// Failure threshold exceeded - all calls rejected
    Open,

    /// Testing if service recovered - limited calls allowed
    HalfOpen,
}

/// Circuit breaker implementation
pub struct CircuitBreaker {
    config: CircuitBreakerConfig,
    state: AtomicU8,
    failure_count: AtomicU32,
    success_count: AtomicU32,
    last_failure: AtomicI64,
    opened_at: AtomicI64,
}

impl CircuitBreaker {
    /// Check if call should be allowed
    pub fn allow(&self) -> Result<CircuitBreakerPermit, CircuitBreakerError>;

    /// Record a successful call
    pub fn record_success(&self);

    /// Record a failed call
    pub fn record_failure(&self);
}
```

### 3. Timeout Manager

```rust
/// Manages activity timeouts
pub struct TimeoutManager {
    store: Arc<dyn EventStore>,
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
        // 2. Check start_to_close timeouts (running too long)
        // 3. Check heartbeat timeouts (no heartbeat)
        // 4. Fail or requeue affected tasks
    }
}
```

### 4. Dead Letter Queue

```rust
/// Dead letter queue handler
pub struct DlqHandler {
    store: Arc<dyn EventStore>,
}

impl DlqHandler {
    /// Process a task that has exhausted all retries
    pub async fn handle_dead_task(
        &self,
        task: &FailedTask,
        error_history: Vec<String>,
    ) -> Result<(), StoreError> {
        // 1. Move to DLQ table
        // 2. Emit WorkflowEvent::ActivityFailed with will_retry=false
        // 3. Let workflow decide how to handle (fail workflow or continue)
    }

    /// Requeue a DLQ entry for retry (admin action)
    pub async fn requeue(
        &self,
        dlq_id: Uuid,
        new_options: Option<ActivityOptions>,
    ) -> Result<Uuid, StoreError>;

    /// Purge old DLQ entries
    pub async fn purge_older_than(&self, age: Duration) -> Result<u64, StoreError>;
}
```

---

## Worker Architecture

### Worker Pool

```rust
/// Configuration for the worker pool
#[derive(Debug, Clone)]
pub struct WorkerConfig {
    /// Worker ID (unique across cluster)
    pub worker_id: String,

    /// Activity types this worker can handle
    pub activity_types: Vec<String>,

    /// Number of concurrent activity executions
    pub concurrency: usize,

    /// Polling interval when no tasks available
    pub poll_interval: Duration,

    /// Backoff configuration for polling
    pub poll_backoff: BackoffConfig,
}

/// Worker pool that manages concurrent activity execution
pub struct WorkerPool {
    config: WorkerConfig,
    store: Arc<dyn EventStore>,
    activity_registry: Arc<ActivityRegistry>,
    circuit_breakers: Arc<CircuitBreakerRegistry>,
    shutdown: watch::Receiver<bool>,
}

impl WorkerPool {
    /// Run the worker pool until shutdown
    pub async fn run(&self) -> Result<(), WorkerError> {
        let semaphore = Arc::new(Semaphore::new(self.config.concurrency));

        loop {
            tokio::select! {
                _ = self.shutdown.changed() => break,
                permit = semaphore.clone().acquire_owned() => {
                    let permit = permit?;

                    // Try to claim a task
                    match self.claim_task().await? {
                        Some(task) => {
                            let this = self.clone();
                            tokio::spawn(async move {
                                let _permit = permit;  // Hold permit until done
                                this.execute_task(task).await;
                            });
                        }
                        None => {
                            drop(permit);
                            // Backoff when no tasks
                            tokio::time::sleep(self.poll_interval()).await;
                        }
                    }
                }
            }
        }

        Ok(())
    }

    async fn execute_task(&self, task: ClaimedTask) -> Result<(), WorkerError> {
        // 1. Check circuit breaker
        // 2. Start heartbeat task
        // 3. Execute activity
        // 4. Complete or fail task
        // 5. Cancel heartbeat
    }
}
```

### Workflow Executor

```rust
/// Executes workflows by replaying events and processing new ones
pub struct WorkflowExecutor {
    store: Arc<dyn EventStore>,
    workflow_registry: Arc<WorkflowRegistry>,
}

impl WorkflowExecutor {
    /// Start a new workflow
    pub async fn start_workflow<W: Workflow>(
        &self,
        workflow_id: Uuid,
        input: W::Input,
    ) -> Result<Uuid, ExecutorError> {
        // 1. Create workflow instance in DB
        // 2. Append WorkflowStarted event
        // 3. Create workflow state machine
        // 4. Call on_start() and process actions
        // 5. Return workflow ID
    }

    /// Process a workflow activation (after activity completes)
    pub async fn activate_workflow(
        &self,
        workflow_id: Uuid,
        event: WorkflowEvent,
    ) -> Result<(), ExecutorError> {
        // 1. Load events from store
        // 2. Replay events to rebuild state
        // 3. Apply new event
        // 4. Call appropriate workflow method
        // 5. Process resulting actions
        // 6. Persist new events
    }

    /// Replay a workflow from events (for recovery)
    async fn replay(&self, workflow_id: Uuid) -> Result<Box<dyn Workflow>, ExecutorError> {
        let events = self.store.load_events(workflow_id).await?;

        // Get workflow type from first event
        let workflow_type = match events.first() {
            Some((_, WorkflowEvent::WorkflowStarted { .. })) => {
                // Extract type from workflow_instances table
            }
            _ => return Err(ExecutorError::InvalidEventHistory),
        };

        // Create workflow and replay
        let mut workflow = self.workflow_registry.create(workflow_type, input)?;

        for (seq, event) in events {
            match event {
                WorkflowEvent::ActivityCompleted { activity_id, result } => {
                    workflow.on_activity_completed(&activity_id, result);
                }
                // ... handle other events
            }
        }

        Ok(workflow)
    }
}
```

---

## Testing Strategy

### Unit Tests

Located in each module's `tests` submodule:

1. **Workflow state machine tests** - Test state transitions without persistence
2. **RetryPolicy calculation tests** - Verify backoff timing
3. **CircuitBreaker state transitions** - Test state machine logic
4. **Event serialization tests** - Roundtrip serialize/deserialize

### Integration Tests

Located in `tests/integration/`:

```rust
// tests/integration/workflow_execution.rs

#[tokio::test]
async fn test_simple_workflow_execution() {
    let store = InMemoryEventStore::new();
    let executor = WorkflowExecutor::new(store.clone());

    // Start workflow
    let workflow_id = executor.start_workflow::<TestWorkflow>(
        Uuid::now_v7(),
        TestInput { value: 42 },
    ).await.unwrap();

    // Simulate activity completion
    executor.activate_workflow(
        workflow_id,
        WorkflowEvent::ActivityCompleted {
            activity_id: "step-1".to_string(),
            result: json!({"computed": 84}),
        },
    ).await.unwrap();

    // Verify workflow completed
    let status = store.get_workflow_status(workflow_id).await.unwrap();
    assert_eq!(status, WorkflowStatus::Completed);
}

#[tokio::test]
async fn test_activity_retry_on_failure() {
    // Test that activities are retried according to policy
}

#[tokio::test]
async fn test_circuit_breaker_opens_on_failures() {
    // Test that circuit breaker opens after threshold
}

#[tokio::test]
async fn test_workflow_recovery_after_crash() {
    // Test that workflow can be replayed from events
}

#[tokio::test]
async fn test_dlq_after_max_retries() {
    // Test that tasks go to DLQ after exhausting retries
}
```

### Load/Stress Tests

Located in `benches/`:

```rust
// benches/workflow_throughput.rs

use criterion::{criterion_group, criterion_main, Criterion, Throughput};

fn workflow_throughput(c: &mut Criterion) {
    let rt = tokio::runtime::Runtime::new().unwrap();
    let store = rt.block_on(PostgresEventStore::connect("..."));

    let mut group = c.benchmark_group("workflow_throughput");
    group.throughput(Throughput::Elements(1));

    group.bench_function("start_workflow", |b| {
        b.iter(|| {
            rt.block_on(async {
                executor.start_workflow::<SimpleWorkflow>(
                    Uuid::now_v7(),
                    SimpleInput {},
                ).await
            })
        })
    });

    group.finish();
}

criterion_group!(benches, workflow_throughput);
criterion_main!(benches);
```

### Stress Test Suite

Separate binary for load testing:

```rust
// tests/stress/main.rs

/// Stress test configuration
struct StressConfig {
    /// Number of concurrent workflows
    concurrent_workflows: usize,

    /// Total workflows to run
    total_workflows: usize,

    /// Activity failure rate (0.0-1.0)
    failure_rate: f64,

    /// Activity latency simulation
    activity_latency: Duration,
}

#[tokio::main]
async fn main() {
    let config = StressConfig::from_env();

    // Spawn workers
    let workers = spawn_workers(config.worker_count).await;

    // Generate load
    let metrics = generate_load(config).await;

    // Report results
    println!("Throughput: {} workflows/sec", metrics.throughput);
    println!("P50 latency: {:?}", metrics.p50);
    println!("P99 latency: {:?}", metrics.p99);
    println!("Failure rate: {:.2}%", metrics.failure_rate * 100.0);
}
```

---

## Implementation Phases

### Phase 1: Core Abstractions (Week 1)
- [ ] Workflow and Activity traits
- [ ] WorkflowAction enum
- [ ] In-memory EventStore for testing
- [ ] Basic WorkflowExecutor (no persistence)
- [ ] Unit tests for state machines

### Phase 2: Persistence Layer (Week 2)
- [ ] PostgreSQL migrations
- [ ] PostgresEventStore implementation
- [ ] Task queue with claim/complete
- [ ] Event replay for recovery
- [ ] Integration tests with PostgreSQL

### Phase 3: Reliability Features (Week 3)
- [ ] RetryPolicy implementation
- [ ] CircuitBreaker implementation
- [ ] Timeout manager
- [ ] Dead letter queue
- [ ] Integration tests for reliability

### Phase 4: Worker Pool (Week 4)
- [ ] WorkerPool with concurrency control
- [ ] Heartbeat mechanism
- [ ] Graceful shutdown
- [ ] Stale task reclamation
- [ ] Load testing

### Phase 5: Integration with Everruns (Week 5)
- [ ] Migrate TurnWorkflow to new engine
- [ ] Update worker crate to use durable engine
- [ ] Remove Temporal dependencies
- [ ] End-to-end smoke tests

### Phase 6: Production Hardening (Week 6)
- [ ] Metrics and observability
- [ ] Admin API for DLQ management
- [ ] Documentation
- [ ] Performance tuning

---

## Migration Strategy

### Parallel Operation

1. Keep Temporal running during migration
2. New workflows use `everruns-durable`
3. Existing workflows complete on Temporal
4. Monitor both systems
5. Remove Temporal when no active workflows remain

### Rollback Plan

1. Feature flag to switch between engines
2. Keep Temporal infrastructure for 2 weeks post-migration
3. Automatic fallback if error rate exceeds threshold

---

## Open Questions

1. **Partitioning strategy** - How to scale beyond single PostgreSQL instance?
2. **Multi-tenancy** - Should workflows be isolated by tenant?
3. **Workflow versioning** - How to handle schema changes in running workflows?
4. **Signals/queries** - Do we need Temporal-style signals for workflow communication?

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
tracing = "0.1"
rand = "0.8"

[dev-dependencies]
criterion = { version = "0.5", features = ["async_tokio"] }
tokio-test = "0.4"
```
