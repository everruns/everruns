// Task Worker Implementation
//
// Decision: Single worker implementation generic over WorkerAdapters
// Decision: Works with both gRPC (external) and Direct (in-process) adapters
// Decision: Replaces both InProcessWorker and DurableWorker
//
// TaskWorker executes activities (input, reason, act) from the durable task queue.
// It unifies the two worker implementations into one, eliminating code duplication
// while preserving the different deployment models (in-process vs external).
//
// Decision: the worker owns the poll loop, registration, heartbeats and
// concurrency; durable-engine's `TurnTaskDriver` runs each claimed task (turn
// steps, checkpoints, wake drains, failure sealing). `WorkerTurnHost` plugs
// the worker's adapters and non-turn activities into that driver.

use crate::durable::WorkerInfo;
use anyhow::Result;
use std::sync::Arc;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::time::Duration;
use tokio::sync::{Notify, watch};
use tokio::task::JoinSet;
use tracing::{debug, error, info, warn};
use uuid::Uuid;

use crate::activities::activity_types;
use crate::task_error::summarize_task_failure;
use crate::task_wakeup::spawn_wakeup_listener;
use crate::turn_driver::TurnTaskDriver;
use crate::turn_host::WorkerTurnHost;
use crate::worker_adapters::WorkerAdapters;

// Re-export atom types
pub use crate::engine::{InputAtomInput, ReasonInput, ReasonResult};

// =============================================================================
// Configuration
// =============================================================================

/// Default execution concurrency for workers.
///
/// Historically 1000, which let a single worker advertise too much capacity for
/// small Postgres instances. 1000-way concurrency is now opt-in via
/// `MAX_CONCURRENT_TASKS`; the default mirrors the server's default DB pool.
pub const DEFAULT_MAX_CONCURRENT_TASKS: usize = 50;

/// Upper bound on a single claim request, independent of execution concurrency.
pub const DEFAULT_CLAIM_BATCH_SIZE: usize = 50;

/// Base fallback poll interval when push notifications are unavailable.
pub const DEFAULT_POLL_INTERVAL: Duration = Duration::from_millis(100);

/// Maximum fallback poll interval while the queue is empty.
pub const DEFAULT_POLL_BACKOFF_MAX: Duration = Duration::from_secs(5);

/// How many tasks to claim in one request, given currently-free execution slots.
pub(crate) fn claim_limit(available_slots: usize, claim_batch_size: usize) -> usize {
    available_slots.min(claim_batch_size)
}

/// Next fallback poll interval.
pub(crate) fn next_poll_backoff(
    current: Duration,
    base: Duration,
    max: Duration,
    found_work: bool,
) -> Duration {
    if found_work {
        base.min(max)
    } else {
        current.checked_mul(2).unwrap_or(max).min(max)
    }
}

/// Configuration for the task worker
#[derive(Debug, Clone)]
pub struct TaskWorkerConfig {
    /// Worker ID (unique identifier for this worker instance)
    pub worker_id: String,
    /// Activity types this worker handles
    pub activity_types: Vec<String>,
    /// Maximum concurrent tasks (execution concurrency / advertised capacity)
    pub max_concurrent_tasks: usize,
    /// Maximum tasks claimed in a single request, regardless of available slots.
    pub claim_batch_size: usize,
    /// Base poll interval when no tasks available
    pub poll_interval: Duration,
    /// Cap for the exponential poll backoff while the queue is empty.
    pub poll_backoff_max: Duration,
    /// Heartbeat interval for worker registration
    pub heartbeat_interval: Duration,
    /// Worker group name (optional, for routing)
    pub worker_group: Option<String>,
    /// gRPC address for standalone worker control-plane communication.
    pub grpc_address: String,
    /// Timeout for initial connection to control-plane gRPC.
    pub connect_timeout: Duration,
    /// Run a turn's next step on this worker right away, enqueued already
    /// claimed, instead of through the queue (`WORKER_CHAIN_STEPS`, on by
    /// default). See the turn driver's module notes.
    pub chain_steps: bool,
}

impl Default for TaskWorkerConfig {
    fn default() -> Self {
        Self {
            worker_id: format!("worker-{}", Uuid::now_v7()),
            activity_types: vec![
                "process_input".to_string(),
                "reason".to_string(),
                "act".to_string(),
                "leased_resource_cleanup".to_string(),
                "session_task_reaper".to_string(),
                activity_types::INVOKE_SCHEDULED_CHANNEL.to_string(),
                activity_types::INVOKE_AGENT_TRIGGER.to_string(),
            ],
            max_concurrent_tasks: DEFAULT_MAX_CONCURRENT_TASKS,
            claim_batch_size: DEFAULT_CLAIM_BATCH_SIZE,
            poll_interval: DEFAULT_POLL_INTERVAL,
            poll_backoff_max: DEFAULT_POLL_BACKOFF_MAX,
            heartbeat_interval: Duration::from_secs(10),
            worker_group: None,
            grpc_address: "127.0.0.1:9001".to_string(),
            connect_timeout: Duration::from_secs(30),
            chain_steps: true,
        }
    }
}

impl TaskWorkerConfig {
    /// Create dev mode configuration (faster pickup, lower concurrency).
    /// Keeps a short backoff cap so an idle dev queue still picks up new work
    /// quickly.
    pub fn dev_mode() -> Self {
        let max_concurrent_tasks = 10;
        Self {
            worker_id: format!("dev-worker-{}", Uuid::now_v7()),
            worker_group: Some("dev".to_string()),
            max_concurrent_tasks,
            // Keep the claim batch within execution concurrency.
            claim_batch_size: DEFAULT_CLAIM_BATCH_SIZE.min(max_concurrent_tasks),
            poll_interval: Duration::from_millis(10),
            poll_backoff_max: Duration::from_millis(250),
            ..Default::default()
        }
    }

    /// Create a high-concurrency configuration. This is the explicit opt-in for
    /// 1000-way execution; the claim batch stays bounded by `claim_batch_size`.
    pub fn production() -> Self {
        Self {
            max_concurrent_tasks: 1000,
            ..Default::default()
        }
    }

    /// Create configuration from environment variables.
    ///
    /// Execution concurrency (`MAX_CONCURRENT_TASKS`), claim batch size
    /// (`CLAIM_BATCH_SIZE`), and the poll interval/backoff cap
    /// (`WORKER_POLL_INTERVAL_MS` / `WORKER_POLL_BACKOFF_MAX_MS`) are independent.
    /// The claim batch is clamped to the execution concurrency.
    pub fn from_env() -> Self {
        use crate::core::config::{env_duration_ms, env_duration_secs, env_or, env_string_any};

        let worker_id =
            std::env::var("WORKER_ID").unwrap_or_else(|_| format!("worker-{}", Uuid::now_v7()));
        let defaults = Self::default();
        let max_concurrent_tasks = env_or("MAX_CONCURRENT_TASKS", defaults.max_concurrent_tasks);
        let claim_batch_size =
            env_or("CLAIM_BATCH_SIZE", defaults.claim_batch_size).min(max_concurrent_tasks);

        Self {
            worker_id,
            max_concurrent_tasks,
            claim_batch_size,
            poll_interval: env_duration_ms("WORKER_POLL_INTERVAL_MS", defaults.poll_interval),
            poll_backoff_max: env_duration_ms(
                "WORKER_POLL_BACKOFF_MAX_MS",
                defaults.poll_backoff_max,
            ),
            worker_group: std::env::var("WORKER_GROUP").ok(),
            grpc_address: env_string_any(
                &["SERVER_GRPC_ADDRESS", "WORKER_GRPC_ADDRESS"],
                &defaults.grpc_address,
            ),
            connect_timeout: env_duration_secs(
                "WORKER_GRPC_CONNECT_TIMEOUT",
                defaults.connect_timeout,
            ),
            chain_steps: env_or("WORKER_CHAIN_STEPS", defaults.chain_steps),
            ..defaults
        }
    }
}

// =============================================================================
// Unified Worker
// =============================================================================

pub use everruns_durable_engine::turn_store::TurnStore;

/// Unified worker that executes tasks from the durable task queue
///
/// This worker is generic over:
/// - `S`: TurnStore implementation (direct store or gRPC store)
/// - `A`: WorkerAdapters implementation (Direct or gRPC)
pub struct TaskWorker<S, A>
where
    S: TurnStore,
    A: WorkerAdapters,
{
    config: TaskWorkerConfig,
    store: Arc<S>,
    /// Runs each claimed task (see durable-engine's `turn_driver`).
    driver: TurnTaskDriver<S, WorkerTurnHost<A>>,
    shutdown_tx: watch::Sender<bool>,
    shutdown_rx: watch::Receiver<bool>,
    in_flight: Arc<AtomicUsize>,
    /// Cuts the poll backoff short when new work may be claimable.
    wake: Arc<Notify>,
}

impl<S, A> TaskWorker<S, A>
where
    S: TurnStore,
    A: WorkerAdapters,
{
    /// Create a new unified worker
    pub fn new(config: TaskWorkerConfig, store: Arc<S>, adapters: A) -> Self {
        let (shutdown_tx, shutdown_rx) = watch::channel(false);

        info!(
            worker_id = %config.worker_id,
            max_concurrent_tasks = config.max_concurrent_tasks,
            claim_batch_size = config.claim_batch_size,
            poll_interval_ms = config.poll_interval.as_millis(),
            poll_backoff_max_ms = config.poll_backoff_max.as_millis(),
            heartbeat_interval_ms = config.heartbeat_interval.as_millis(),
            chain_steps = config.chain_steps,
            "Initialized unified worker"
        );

        let driver = TurnTaskDriver::new(
            store.clone(),
            WorkerTurnHost::new(adapters),
            config.worker_id.clone(),
            config.heartbeat_interval,
        )
        .chain_steps(config.chain_steps);

        Self {
            config,
            store,
            driver,
            shutdown_tx,
            shutdown_rx,
            in_flight: Arc::new(AtomicUsize::new(0)),
            wake: Arc::new(Notify::new()),
        }
    }

    /// Run the worker (blocking until shutdown)
    pub async fn run(&mut self) -> Result<()> {
        info!(
            worker_id = %self.config.worker_id,
            "Starting unified worker"
        );

        // Register worker
        let worker_info = WorkerInfo {
            id: self.config.worker_id.clone(),
            worker_group: self.config.worker_group.clone(),
            activity_types: self.config.activity_types.clone(),
            max_concurrency: self.config.max_concurrent_tasks as u32,
            current_load: 0,
            status: "active".to_string(),
            accepting_tasks: true,
            backpressure_reason: None,
            started_at: chrono::Utc::now(),
            last_heartbeat_at: chrono::Utc::now(),
            hostname: None,
            version: None,
            metadata: None,
            tasks_completed: 0,
            tasks_failed: 0,
            avg_task_duration_ms: None,
        };
        if let Err(e) = self.store.register_worker(worker_info).await {
            warn!(error = %e, "Failed to register worker (will continue anyway)");
        } else {
            info!(worker_id = %self.config.worker_id, "Worker registered");
        }

        // Spawn heartbeat task
        let heartbeat_store = self.store.clone();
        let heartbeat_worker_id = self.config.worker_id.clone();
        let heartbeat_interval = self.config.heartbeat_interval;
        let mut heartbeat_shutdown_rx = self.shutdown_rx.clone();
        let in_flight_for_heartbeat = self.in_flight.clone();

        let heartbeat_handle = tokio::spawn(async move {
            loop {
                tokio::select! {
                    _ = tokio::time::sleep(heartbeat_interval) => {
                        let current_load = in_flight_for_heartbeat.load(Ordering::SeqCst);
                        if let Err(e) = heartbeat_store.worker_heartbeat(
                            &heartbeat_worker_id,
                            current_load,
                            true
                        ).await {
                            warn!(error = %e, "Failed to send heartbeat");
                        }
                    }
                    _ = heartbeat_shutdown_rx.changed() => {
                        break;
                    }
                }
            }
        });

        let wakeup_handle = spawn_wakeup_listener(
            self.store.clone(),
            self.config.worker_id.clone(),
            self.config.activity_types.clone(),
            self.wake.clone(),
            self.shutdown_rx.clone(),
        );

        // Adaptive poll backoff: doubles from `poll_interval` up to
        // `poll_backoff_max` while the queue is empty, so an idle worker stops
        // issuing tight-loop `claim_task` requests. Reset to base on any work.
        let mut poll_backoff = self.config.poll_interval.min(self.config.poll_backoff_max);
        let mut task_handles = JoinSet::new();

        // Main poll loop
        loop {
            if *self.shutdown_rx.borrow() {
                info!("Shutdown signal received, stopping worker");
                break;
            }

            Self::drain_finished_tasks(&mut task_handles);

            match self.poll_and_execute(&mut task_handles).await {
                Ok(executed) => {
                    let mut woken = false;
                    if executed == 0 {
                        tokio::select! {
                            _ = tokio::time::sleep(poll_backoff) => {}
                            _ = self.wake.notified() => woken = true,
                            _ = self.shutdown_rx.changed() => {
                                info!("Shutdown during poll wait");
                                break;
                            }
                        }
                    }
                    poll_backoff = next_poll_backoff(
                        poll_backoff,
                        self.config.poll_interval,
                        self.config.poll_backoff_max,
                        executed > 0 || woken,
                    );
                }
                Err(e) => {
                    error!("Error polling tasks: {}", e);
                    tokio::time::sleep(Duration::from_secs(1)).await;
                }
            }
        }

        task_handles.abort_all();
        while let Some(result) = task_handles.join_next().await {
            if let Err(error) = result
                && !error.is_cancelled()
            {
                warn!(error = %error, "Worker task join failed during shutdown");
            }
        }
        let _ = self.shutdown_tx.send(true);
        let _ = heartbeat_handle.await;
        let _ = wakeup_handle.await;

        // Deregister on shutdown
        if let Err(e) = self.store.deregister_worker(&self.config.worker_id).await {
            warn!(error = %e, "Failed to deregister worker");
        }

        info!("Unified worker stopped");
        Ok(())
    }

    /// Signal the worker to shutdown
    pub fn shutdown(&self) {
        let _ = self.shutdown_tx.send(true);
    }

    /// Get shutdown handle for external shutdown signaling
    pub fn shutdown_handle(&self) -> ShutdownHandle {
        ShutdownHandle {
            tx: self.shutdown_tx.clone(),
        }
    }

    fn drain_finished_tasks(task_handles: &mut JoinSet<()>) {
        while let Some(result) = task_handles.try_join_next() {
            if let Err(error) = result
                && !error.is_cancelled()
            {
                warn!(error = %error, "Worker task join failed");
            }
        }
    }

    /// Poll for tasks and execute them
    async fn poll_and_execute(&self, task_handles: &mut JoinSet<()>) -> Result<usize> {
        let current_in_flight = self.in_flight.load(Ordering::SeqCst);
        let available_slots = self
            .config
            .max_concurrent_tasks
            .saturating_sub(current_in_flight);

        if available_slots == 0 {
            debug!(
                current_in_flight = current_in_flight,
                max = self.config.max_concurrent_tasks,
                "No available slots, skipping claim"
            );
            return Ok(0);
        }

        // Bound the claim by claim_batch_size so a single claim_task stays cheap
        // even when execution concurrency is high.
        let to_claim = claim_limit(available_slots, self.config.claim_batch_size);
        let tasks = self
            .store
            .claim_task(
                &self.config.worker_id,
                &self.config.activity_types,
                to_claim,
            )
            .await
            .map_err(|e| anyhow::anyhow!("Failed to claim tasks: {}", e))?;

        if tasks.is_empty() {
            return Ok(0);
        }

        debug!(
            worker_id = %self.config.worker_id,
            task_count = tasks.len(),
            "Claimed tasks"
        );

        let task_count = tasks.len();

        // Execute tasks concurrently
        for task in tasks {
            let driver = self.driver.clone();
            let in_flight_guard = InFlightTaskGuard::increment(self.in_flight.clone());
            let wake = self.wake.clone();

            task_handles.spawn(async move {
                let _in_flight_guard = in_flight_guard;
                let result = driver.execute_task(&task).await;
                // A finished phase usually enqueued the next one (reason -> act
                // -> reason); claim it now instead of after the poll backoff.
                wake.notify_one();

                if let Err(e) = result {
                    let failure = summarize_task_failure(
                        task.id,
                        task.workflow_id,
                        &task.activity_type,
                        task.attempt,
                        Some(task.max_attempts),
                        &task.input,
                        &e,
                    );
                    error!(
                        task_id = %task.id,
                        workflow_id = ?task.workflow_id,
                        activity_type = %task.activity_type,
                        attempt = task.attempt,
                        max_attempts = task.max_attempts,
                        session_id = ?failure.session_id,
                        tool_identifiers = ?failure.tool_identifiers,
                        error_chain = %failure.error_chain,
                        "Task execution failed"
                    );
                }
            });
        }

        Ok(task_count)
    }
}

/// Handle for triggering worker shutdown
#[derive(Clone)]
pub struct ShutdownHandle {
    tx: watch::Sender<bool>,
}

impl ShutdownHandle {
    /// Trigger shutdown of the worker
    pub fn shutdown(&self) {
        let _ = self.tx.send(true);
    }
}

struct InFlightTaskGuard {
    in_flight: Arc<AtomicUsize>,
}

impl InFlightTaskGuard {
    fn increment(in_flight: Arc<AtomicUsize>) -> Self {
        in_flight.fetch_add(1, Ordering::SeqCst);
        Self { in_flight }
    }
}

impl Drop for InFlightTaskGuard {
    fn drop(&mut self) {
        self.in_flight.fetch_sub(1, Ordering::SeqCst);
    }
}

#[cfg(test)]
mod tests {
    use crate::unified_worker_test_adapters::NoopAdapters;

    use super::*;
    use crate::durable::{
        ClaimedTask, HeartbeatResponse, StoreError, TaskFailureOutcome, WorkflowError,
        WorkflowStatus,
    };
    use std::sync::atomic::AtomicBool;

    #[test]
    fn test_config_default() {
        let config = TaskWorkerConfig::default();
        assert!(config.worker_id.starts_with("worker-"));
        // Safe-by-default concurrency; 1000-way is opt-in.
        assert_eq!(config.max_concurrent_tasks, DEFAULT_MAX_CONCURRENT_TASKS);
        assert_eq!(config.claim_batch_size, DEFAULT_CLAIM_BATCH_SIZE);
        assert!(config.claim_batch_size <= config.max_concurrent_tasks);
    }

    #[test]
    fn test_config_dev_mode() {
        let config = TaskWorkerConfig::dev_mode();
        assert!(config.worker_id.starts_with("dev-worker-"));
        assert_eq!(config.worker_group, Some("dev".to_string()));
        // Claim batch must stay within execution concurrency in every preset.
        assert!(config.claim_batch_size <= config.max_concurrent_tasks);
    }

    #[test]
    fn test_config_production() {
        // production() is the explicit opt-in for high concurrency, but the claim
        // batch stays bounded independently of execution concurrency.
        let config = TaskWorkerConfig::production();
        assert_eq!(config.max_concurrent_tasks, 1000);
        assert_eq!(config.claim_batch_size, DEFAULT_CLAIM_BATCH_SIZE);
        assert!(config.claim_batch_size < config.max_concurrent_tasks);
    }

    #[test]
    fn test_claim_limit_bounds_to_batch() {
        assert_eq!(
            claim_limit(1000, DEFAULT_CLAIM_BATCH_SIZE),
            DEFAULT_CLAIM_BATCH_SIZE
        );
        assert_eq!(claim_limit(1000, 50), 50);
        assert_eq!(claim_limit(10, 50), 10);
        assert_eq!(claim_limit(0, 50), 0);
    }

    #[test]
    fn test_next_poll_backoff() {
        let base = Duration::from_millis(100);
        let max = Duration::from_secs(5);
        let mut backoff = base;

        backoff = next_poll_backoff(backoff, base, max, false);
        assert_eq!(backoff, Duration::from_millis(200));
        backoff = next_poll_backoff(backoff, base, max, false);
        assert_eq!(backoff, Duration::from_millis(400));

        for _ in 0..10 {
            backoff = next_poll_backoff(backoff, base, max, false);
        }
        assert_eq!(backoff, max);

        assert_eq!(next_poll_backoff(backoff, base, max, true), base);
        assert_eq!(next_poll_backoff(base, max * 2, max, true), max);
        assert_eq!(next_poll_backoff(Duration::MAX, base, max, false), max);
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn poll_and_execute_allows_concurrent_store_calls() {
        #[derive(Clone, Default)]
        struct ConcurrentStore {
            claimed: Arc<AtomicBool>,
            current: Arc<AtomicUsize>,
            max: Arc<AtomicUsize>,
        }

        #[async_trait::async_trait]
        impl TurnStore for ConcurrentStore {
            async fn register_worker(&self, _worker: WorkerInfo) -> Result<(), StoreError> {
                Ok(())
            }

            async fn worker_heartbeat(
                &self,
                _worker_id: &str,
                _current_load: usize,
                _accepting_tasks: bool,
            ) -> Result<(), StoreError> {
                Ok(())
            }

            async fn deregister_worker(&self, _worker_id: &str) -> Result<usize, StoreError> {
                Ok(0)
            }

            async fn claim_task(
                &self,
                _worker_id: &str,
                _activity_types: &[String],
                _max_tasks: usize,
            ) -> Result<Vec<ClaimedTask>, StoreError> {
                if self.claimed.swap(true, Ordering::SeqCst) {
                    return Ok(vec![]);
                }

                Ok((0..2)
                    .map(|_| ClaimedTask {
                        id: Uuid::now_v7(),
                        workflow_id: None,
                        activity_id: format!("unknown_{}", Uuid::now_v7()),
                        activity_type: "unknown".to_string(),
                        input: serde_json::json!({}),
                        attempt: 1,
                        max_attempts: 1,
                        ..Default::default()
                    })
                    .collect())
            }

            async fn heartbeat_task(
                &self,
                _task_id: Uuid,
                _worker_id: &str,
                _details: Option<serde_json::Value>,
            ) -> Result<HeartbeatResponse, StoreError> {
                Ok(HeartbeatResponse {
                    accepted: true,
                    should_cancel: false,
                })
            }

            async fn get_workflow(
                &self,
                _workflow_id: Uuid,
            ) -> Result<everruns_durable_engine::turn_store::WorkflowSnapshot, StoreError>
            {
                Ok(everruns_durable_engine::turn_store::WorkflowSnapshot {
                    status: WorkflowStatus::Running,
                    output: None,
                    error: None,
                })
            }

            async fn start_turn(
                &self,
                _workflow_id: Uuid,
                _workflow_type: &str,
                _input: serde_json::Value,
                _activity_id: String,
                _activity_type: String,
            ) -> Result<crate::durable::RunStart, StoreError> {
                Ok(crate::durable::RunStart::Active)
            }

            async fn cancel_pending_tasks(&self, _workflow_id: Uuid) -> Result<u64, StoreError> {
                Ok(0)
            }

            async fn count_active_workflows(&self) -> Result<usize, StoreError> {
                Ok(0)
            }

            async fn send_signal(
                &self,
                _workflow_id: Uuid,
                _signal: crate::durable::WorkflowSignal,
            ) -> Result<(), StoreError> {
                Ok(())
            }

            async fn record_activity_started(&self, _task: &ClaimedTask, _worker_id: &str) {}

            async fn complete_task_and_record(
                &self,
                _task: &ClaimedTask,
                _worker_id: &str,
                _output: serde_json::Value,
            ) -> Result<(), StoreError> {
                Ok(())
            }

            async fn fail_task_and_record(
                &self,
                _task: &ClaimedTask,
                _error: &str,
                _retryable: bool,
            ) -> Result<TaskFailureOutcome, StoreError> {
                let current = self.current.fetch_add(1, Ordering::SeqCst) + 1;
                self.max.fetch_max(current, Ordering::SeqCst);
                tokio::time::sleep(Duration::from_millis(50)).await;
                self.current.fetch_sub(1, Ordering::SeqCst);
                Ok(TaskFailureOutcome::MovedToDlq)
            }

            async fn enqueue_task_and_record(
                &self,
                _workflow_id: Uuid,
                _activity_id: String,
                _activity_type: String,
                _input: serde_json::Value,
            ) -> Result<Uuid, StoreError> {
                Ok(Uuid::now_v7())
            }

            async fn update_workflow_status(
                &self,
                _workflow_id: Uuid,
                _status: WorkflowStatus,
                _output: Option<serde_json::Value>,
                _error: Option<WorkflowError>,
            ) -> Result<(), StoreError> {
                Ok(())
            }

            async fn complete_workflow(
                &self,
                _workflow_id: Uuid,
                _event_output: serde_json::Value,
                _stored_output: Option<serde_json::Value>,
                _error: Option<WorkflowError>,
            ) -> Result<(), StoreError> {
                Ok(())
            }

            async fn consume_pending_signals(
                &self,
                _workflow_id: Uuid,
            ) -> Result<Vec<crate::durable::WorkflowSignal>, StoreError> {
                Ok(vec![])
            }

            async fn consume_pending_signals_by_type(
                &self,
                _workflow_id: Uuid,
                _signal_type: &str,
            ) -> Result<Vec<crate::durable::WorkflowSignal>, StoreError> {
                Ok(vec![])
            }
        }

        let store = Arc::new(ConcurrentStore::default());
        let config = TaskWorkerConfig {
            max_concurrent_tasks: 2,
            claim_batch_size: 2,
            heartbeat_interval: Duration::from_secs(60),
            ..Default::default()
        };

        let worker = TaskWorker::new(config, store.clone(), NoopAdapters);
        let mut task_handles = JoinSet::new();

        let claimed = worker
            .poll_and_execute(&mut task_handles)
            .await
            .expect("poll succeeds");

        assert_eq!(claimed, 2);
        while task_handles.join_next().await.is_some() {}

        assert!(
            store.max.load(Ordering::SeqCst) > 1,
            "store calls were serialized"
        );
    }
}
