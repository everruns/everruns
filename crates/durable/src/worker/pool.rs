//! Worker pool for task execution
//!
//! Manages concurrent task execution with backpressure and graceful shutdown.

use std::collections::HashMap;
use std::sync::Arc;
use std::time::Duration;

use chrono::Utc;
use serde::{Deserialize, Serialize};
use tokio::sync::{Semaphore, watch};
use tokio::task::JoinHandle;
use tracing::{debug, error, info, instrument, warn};
use uuid::Uuid;

use super::backpressure::{BackpressureConfig, BackpressureState, ResourceMonitor};
use super::poller::{PollerConfig, PollerError, TaskPoller};
use crate::persistence::{
    CapacitySnapshot, ClaimedTask, StoreError, WorkerInfo, WorkflowEventStore,
};

/// Worker pool configuration
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct WorkerPoolConfig {
    /// Unique worker ID (generated if not provided)
    pub worker_id: String,

    /// Worker group for logical organization
    pub worker_group: String,

    /// Activity types this worker handles
    pub activity_types: Vec<String>,

    /// Maximum concurrent task executions
    pub max_concurrency: usize,

    /// Backpressure configuration
    pub backpressure: BackpressureConfig,

    /// Poller configuration
    pub poller: PollerConfig,

    /// Heartbeat interval
    #[serde(with = "duration_millis")]
    pub heartbeat_interval: Duration,

    /// Stale task reclamation interval. Zero disables the pool's reclaim loop.
    #[serde(with = "duration_millis")]
    pub stale_reclaim_interval: Duration,

    /// How long before a task is considered stale
    #[serde(with = "duration_millis")]
    pub stale_threshold: Duration,

    /// Graceful shutdown timeout
    #[serde(with = "duration_millis")]
    pub shutdown_timeout: Duration,

    /// Resource monitoring interval (CPU/memory sampling)
    #[serde(with = "duration_millis")]
    pub resource_monitor_interval: Duration,
}

impl Default for WorkerPoolConfig {
    fn default() -> Self {
        Self {
            worker_id: format!("worker-{}", Uuid::now_v7()),
            worker_group: "default".to_string(),
            activity_types: vec![],
            max_concurrency: 50,
            backpressure: BackpressureConfig::default(),
            poller: PollerConfig::default(),
            heartbeat_interval: Duration::from_secs(5),
            stale_reclaim_interval: Duration::from_secs(30),
            stale_threshold: Duration::from_secs(60),
            shutdown_timeout: Duration::from_secs(30),
            resource_monitor_interval: Duration::from_secs(2),
        }
    }
}

impl WorkerPoolConfig {
    /// Create a new worker pool configuration
    pub fn new(activity_types: Vec<String>) -> Self {
        Self {
            activity_types,
            ..Default::default()
        }
    }

    /// Set the worker ID
    pub fn with_worker_id(mut self, id: impl Into<String>) -> Self {
        self.worker_id = id.into();
        self
    }

    /// Do not run the pool's own stale-task reclaim loop.
    ///
    /// For a host that already runs one [`StaleTaskReaper`](crate::StaleTaskReaper)
    /// with its own [`ReapHandler`](crate::ReapHandler): a second reaper
    /// without that handler could settle a dead task first, and the host
    /// would never hear about it.
    pub fn without_stale_reclaim(mut self) -> Self {
        self.stale_reclaim_interval = Duration::ZERO;
        self
    }

    /// Set the worker group
    pub fn with_worker_group(mut self, group: impl Into<String>) -> Self {
        self.worker_group = group.into();
        self
    }

    /// Set maximum concurrency
    pub fn with_max_concurrency(mut self, max: usize) -> Self {
        self.max_concurrency = max.max(1);
        self
    }

    /// Set backpressure configuration
    pub fn with_backpressure(mut self, config: BackpressureConfig) -> Self {
        self.backpressure = config;
        self
    }

    /// Set poller configuration
    pub fn with_poller(mut self, config: PollerConfig) -> Self {
        self.poller = config;
        self
    }

    /// Set heartbeat interval
    pub fn with_heartbeat_interval(mut self, interval: Duration) -> Self {
        self.heartbeat_interval = interval;
        self
    }

    /// Set shutdown timeout
    pub fn with_shutdown_timeout(mut self, timeout: Duration) -> Self {
        self.shutdown_timeout = timeout;
        self
    }
}

/// Worker pool status
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum WorkerPoolStatus {
    /// Worker is starting up
    Starting,
    /// Worker is running and accepting tasks
    Running,
    /// Worker is draining (completing current tasks, not accepting new ones)
    Draining,
    /// Worker has stopped
    Stopped,
}

/// Worker pool errors
#[derive(Debug, thiserror::Error)]
pub enum WorkerPoolError {
    /// Store error
    #[error("store error: {0}")]
    Store(#[from] StoreError),

    /// Poller error
    #[error("poller error: {0}")]
    Poller(#[from] PollerError),

    /// Task execution error
    #[error("task execution error: {0}")]
    TaskExecution(String),

    /// Worker already running
    #[error("worker pool is already running")]
    AlreadyRunning,

    /// Worker not running
    #[error("worker pool is not running")]
    NotRunning,

    /// Shutdown timeout
    #[error("graceful shutdown timed out")]
    ShutdownTimeout,

    /// Activity handler not found
    #[error("no handler registered for activity type: {0}")]
    HandlerNotFound(String),
}

/// Activity execution result
pub type ActivityResult = Result<serde_json::Value, String>;

/// Activity handler function type
pub type ActivityHandler = Arc<
    dyn Fn(
            ClaimedTask,
        ) -> std::pin::Pin<Box<dyn std::future::Future<Output = ActivityResult> + Send>>
        + Send
        + Sync,
>;

/// Worker pool for executing activities
///
/// **Experimental** (`workflows` feature): the API may change in any release.
///
/// The pool polls the store for its activity types, runs handlers with
/// bounded concurrency, heartbeats its tasks, reclaims stale work and stops
/// claiming under backpressure. It completes or fails tasks in the store; it
/// does not advance workflows, so a workflow-driven deployment reports
/// completions to [`WorkflowExecutor`](crate::WorkflowExecutor) separately.
///
/// # Example
///
/// ```
/// use std::sync::Arc;
/// use std::time::Duration;
/// use everruns_durable::{
///     ActivityOptions, InMemoryWorkflowEventStore, TaskDefinition, TaskStatus, WorkerPool,
///     TaskQueue, WorkerPoolConfig,
/// };
/// use serde_json::json;
///
/// # #[tokio::main]
/// # async fn main() -> Result<(), Box<dyn std::error::Error>> {
/// let store = Arc::new(InMemoryWorkflowEventStore::new());
/// let config = WorkerPoolConfig::new(vec!["resize_image".to_string()])
///     .with_worker_id("images-1")
///     .with_max_concurrency(8);
/// let pool = WorkerPool::new(store.clone(), config);
///
/// // A handler gets the claimed task and returns the JSON output, or an
/// // error string that fails the attempt under the task's retry policy.
/// pool.register_handler("resize_image", |task| async move {
///     let width = task.input["width"].as_u64().ok_or("missing width")?;
///     Ok(json!({ "width": width / 2 }))
/// });
/// pool.start().await?;
///
/// // A standalone task: no workflow, just the queue.
/// let task_id = store
///     .enqueue_task(TaskDefinition {
///         workflow_id: None,
///         activity_id: "img-1".into(),
///         activity_type: "resize_image".into(),
///         input: json!({ "width": 1024 }),
///         options: ActivityOptions::default(),
///     })
///     .await?;
///
/// tokio::time::timeout(Duration::from_secs(5), async {
///     while store.get_task(task_id).await.unwrap().status != TaskStatus::Completed {
///         tokio::time::sleep(Duration::from_millis(10)).await;
///     }
/// })
/// .await?;
///
/// // Drains in-flight tasks before returning.
/// pool.shutdown().await?;
/// # Ok(()) }
/// ```
pub struct WorkerPool {
    store: Arc<dyn WorkflowEventStore>,
    config: WorkerPoolConfig,
    backpressure: Arc<BackpressureState>,
    handlers: std::sync::RwLock<HashMap<String, ActivityHandler>>,
    shutdown_tx: watch::Sender<bool>,
    shutdown_rx: watch::Receiver<bool>,
    status: std::sync::RwLock<WorkerPoolStatus>,
    active_tasks: Arc<Semaphore>,
    /// EVE-639: cached system capacity snapshot. Refreshed on the heartbeat tick
    /// (every `heartbeat_interval`, default 5s) instead of being queried on every
    /// poll iteration (min interval 10ms), which ran a SUM/COUNT aggregate over
    /// `durable_workers` per poll.
    capacity_snapshot: Arc<std::sync::RwLock<CapacitySnapshot>>,
    poll_handle: std::sync::Mutex<Option<JoinHandle<()>>>,
    heartbeat_handle: std::sync::Mutex<Option<JoinHandle<()>>>,
    reclaim_handle: std::sync::Mutex<Option<JoinHandle<()>>>,
    resource_handle: std::sync::Mutex<Option<JoinHandle<()>>>,
}

impl WorkerPool {
    /// Create a new worker pool
    pub fn new(store: Arc<dyn WorkflowEventStore>, config: WorkerPoolConfig) -> Self {
        let (shutdown_tx, shutdown_rx) = watch::channel(false);
        let backpressure = Arc::new(BackpressureState::new(
            config.backpressure.clone(),
            config.max_concurrency,
        ));

        Self {
            store,
            config: config.clone(),
            backpressure,
            handlers: std::sync::RwLock::new(HashMap::new()),
            shutdown_tx,
            shutdown_rx,
            status: std::sync::RwLock::new(WorkerPoolStatus::Stopped),
            active_tasks: Arc::new(Semaphore::new(config.max_concurrency)),
            capacity_snapshot: Arc::new(std::sync::RwLock::new(CapacitySnapshot::default())),
            poll_handle: std::sync::Mutex::new(None),
            heartbeat_handle: std::sync::Mutex::new(None),
            reclaim_handle: std::sync::Mutex::new(None),
            resource_handle: std::sync::Mutex::new(None),
        }
    }

    /// Register an activity handler
    pub fn register_handler<F, Fut>(&self, activity_type: &str, handler: F)
    where
        F: Fn(ClaimedTask) -> Fut + Send + Sync + 'static,
        Fut: std::future::Future<Output = ActivityResult> + Send + 'static,
    {
        let handler: ActivityHandler = Arc::new(move |task| Box::pin(handler(task)));
        self.handlers
            .write()
            .unwrap()
            .insert(activity_type.to_string(), handler);
    }

    /// Start the worker pool
    #[instrument(skip(self), fields(worker_id = %self.config.worker_id))]
    pub async fn start(&self) -> Result<(), WorkerPoolError> {
        {
            let status = *self.status.read().unwrap();
            if status == WorkerPoolStatus::Running {
                return Err(WorkerPoolError::AlreadyRunning);
            }
        }

        info!(
            worker_id = %self.config.worker_id,
            activity_types = ?self.config.activity_types,
            max_concurrency = self.config.max_concurrency,
            "Starting worker pool"
        );

        // Register with the store
        self.register_worker().await?;

        // Seed the capacity snapshot so the first poll iterations have real
        // data instead of the (0, 0) default. Refreshed thereafter on the
        // heartbeat tick. A failure here is non-fatal (default => no cap).
        if let Ok(snap) = self.store.get_capacity_snapshot().await {
            *self.capacity_snapshot.write().unwrap() = snap;
        }

        // Update status
        *self.status.write().unwrap() = WorkerPoolStatus::Running;

        // Start background tasks
        let has_resource_thresholds = self.config.backpressure.memory_threshold.is_some()
            || self.config.backpressure.cpu_threshold.is_some();
        if has_resource_thresholds {
            self.start_resource_monitor_loop();
        }
        self.start_poll_loop();
        self.start_heartbeat_loop();
        if !self.config.stale_reclaim_interval.is_zero() {
            self.start_reclaim_loop();
        }

        Ok(())
    }

    /// Shutdown the worker pool gracefully
    #[instrument(skip(self), fields(worker_id = %self.config.worker_id))]
    pub async fn shutdown(&self) -> Result<(), WorkerPoolError> {
        {
            let status = *self.status.read().unwrap();
            if status == WorkerPoolStatus::Stopped {
                return Ok(());
            }
        }

        info!(worker_id = %self.config.worker_id, "Initiating graceful shutdown");

        // Signal shutdown
        *self.status.write().unwrap() = WorkerPoolStatus::Draining;
        let _ = self.shutdown_tx.send(true);

        // Wait for active tasks to complete (with timeout)
        let deadline = tokio::time::Instant::now() + self.config.shutdown_timeout;

        loop {
            let available = self.active_tasks.available_permits();
            if available == self.config.max_concurrency {
                debug!("All tasks completed");
                break;
            }

            if tokio::time::Instant::now() >= deadline {
                warn!(
                    remaining_tasks = self.config.max_concurrency - available,
                    "Shutdown timeout reached"
                );
                return Err(WorkerPoolError::ShutdownTimeout);
            }

            tokio::time::sleep(Duration::from_millis(100)).await;
        }

        // Deregister from store
        self.deregister_worker().await?;

        // Update status
        *self.status.write().unwrap() = WorkerPoolStatus::Stopped;

        info!(worker_id = %self.config.worker_id, "Worker pool stopped");
        Ok(())
    }

    /// Get current status
    pub fn status(&self) -> WorkerPoolStatus {
        *self.status.read().unwrap()
    }

    /// Get current load
    pub fn current_load(&self) -> usize {
        self.backpressure.current_load()
    }

    /// Get the worker ID
    pub fn worker_id(&self) -> &str {
        &self.config.worker_id
    }

    /// Check if accepting tasks
    pub fn is_accepting(&self) -> bool {
        self.backpressure.is_accepting()
            && *self.status.read().unwrap() == WorkerPoolStatus::Running
    }

    /// Register worker with store
    async fn register_worker(&self) -> Result<(), WorkerPoolError> {
        let worker_info = WorkerInfo {
            id: self.config.worker_id.clone(),
            worker_group: Some(self.config.worker_group.clone()),
            activity_types: self.config.activity_types.clone(),
            max_concurrency: self.config.max_concurrency as u32,
            current_load: 0,
            status: "active".to_string(),
            accepting_tasks: true,
            backpressure_reason: None,
            started_at: Utc::now(),
            last_heartbeat_at: Utc::now(),
            hostname: None,
            version: None,
            metadata: None,
            tasks_completed: 0,
            tasks_failed: 0,
            avg_task_duration_ms: None,
        };

        self.store.register_worker(worker_info).await?;
        Ok(())
    }

    /// Deregister worker from store
    async fn deregister_worker(&self) -> Result<(), WorkerPoolError> {
        self.store.deregister_worker(&self.config.worker_id).await?;
        Ok(())
    }

    /// Start the polling loop
    fn start_poll_loop(&self) {
        let store = Arc::clone(&self.store);
        let config = self.config.clone();
        let backpressure = Arc::clone(&self.backpressure);
        let handlers = self.handlers.read().unwrap().clone();
        let active_tasks = Arc::clone(&self.active_tasks);
        let shutdown_rx = self.shutdown_rx.clone();
        let capacity_snapshot = Arc::clone(&self.capacity_snapshot);

        let handle = tokio::spawn(async move {
            let mut poller = TaskPoller::new(
                store.clone(),
                config.worker_id.clone(),
                config.activity_types.clone(),
                config.poller.clone(),
                shutdown_rx.clone(),
            );

            loop {
                // Check for shutdown
                if poller.is_shutdown() {
                    debug!("Poll loop: shutdown requested");
                    break;
                }

                // Check backpressure
                if !backpressure.should_accept() {
                    debug!("Poll loop: under backpressure, waiting");
                    if poller.wait().await {
                        break; // Shutdown
                    }
                    continue;
                }

                // Calculate how many tasks to claim (fair-share)
                let my_available = backpressure.available_slots();
                if my_available == 0 {
                    if poller.wait().await {
                        break;
                    }
                    continue;
                }

                // Cap claim batch to fair share of total system capacity.
                // EVE-639: read the cached snapshot (refreshed on the heartbeat
                // tick) instead of running a SUM/COUNT aggregate every poll.
                let claim_limit = {
                    let snap = capacity_snapshot.read().unwrap();
                    fair_share_claim_limit(my_available, snap.total_available, snap.active_workers)
                };

                // Poll for tasks
                match poller.poll(claim_limit).await {
                    Ok(tasks) => {
                        for task in tasks {
                            // Get handler
                            let handler = match handlers.get(&task.activity_type) {
                                Some(h) => Arc::clone(h),
                                None => {
                                    // The task is already claimed. Leaving it would park it
                                    // until stale reclamation hands it back to this same
                                    // pool, over and over; fail it visibly instead.
                                    warn!(
                                        activity_type = %task.activity_type,
                                        "No handler registered"
                                    );
                                    let error = format!(
                                        "no handler registered for activity type '{}'",
                                        task.activity_type
                                    );
                                    if let Err(e) =
                                        store.fail_task_with_retry(task.id, &error, false).await
                                    {
                                        error!(task_id = %task.id, "Failed to fail task: {}", e);
                                    }
                                    continue;
                                }
                            };

                            // Acquire semaphore permit
                            let permit = match active_tasks.clone().try_acquire_owned() {
                                Ok(p) => p,
                                Err(_) => {
                                    debug!("No permits available");
                                    break;
                                }
                            };

                            // Track in backpressure
                            backpressure.task_started();

                            // Spawn task execution
                            let store = Arc::clone(&store);
                            let bp = Arc::clone(&backpressure);
                            let worker_id = config.worker_id.clone();
                            let heartbeat_interval = config.heartbeat_interval;

                            tokio::spawn(async move {
                                let task_id = task.id;
                                // Run the handler in its own task so a panic fails the
                                // attempt instead of skipping the report and leaking the
                                // backpressure slot.
                                let result = run_with_task_heartbeat(
                                    store.as_ref(),
                                    task_id,
                                    &worker_id,
                                    heartbeat_interval,
                                    tokio::spawn(handler(task)),
                                )
                                .await;

                                // Report result
                                match result {
                                    Ok(output) => {
                                        // Pass worker_id to verify ownership (prevents duplicate scheduling)
                                        if let Err(e) =
                                            store.complete_task(task_id, &worker_id, output).await
                                        {
                                            error!(%task_id, "Failed to complete task: {}", e);
                                        }
                                    }
                                    Err(error) => {
                                        if let Err(e) = store.fail_task(task_id, &error).await {
                                            error!(%task_id, "Failed to fail task: {}", e);
                                        }
                                    }
                                }

                                // Release
                                bp.task_completed();
                                drop(permit);
                            });
                        }
                    }
                    Err(e) => {
                        crate::persistence::log_database_failure(
                            "durable.worker.poll",
                            "worker poll failed",
                            &e.to_string(),
                        );
                    }
                }

                // Wait before next poll
                if poller.wait().await {
                    break;
                }
            }

            debug!("Poll loop exited");
        });

        *self.poll_handle.lock().unwrap() = Some(handle);
    }

    /// Start the heartbeat loop
    fn start_heartbeat_loop(&self) {
        let store = Arc::clone(&self.store);
        let worker_id = self.config.worker_id.clone();
        let interval = self.config.heartbeat_interval;
        let backpressure = Arc::clone(&self.backpressure);
        let mut shutdown_rx = self.shutdown_rx.clone();
        let capacity_snapshot = Arc::clone(&self.capacity_snapshot);

        let handle = tokio::spawn(async move {
            let mut ticker = tokio::time::interval(interval);

            loop {
                tokio::select! {
                    _ = ticker.tick() => {
                        let load = backpressure.current_load();
                        let accepting = backpressure.is_accepting()
                            && !backpressure.is_resource_pressured();

                        if let Err(e) = store.worker_heartbeat(&worker_id, load, accepting).await {
                            crate::persistence::log_database_failure(
                                "durable.worker.heartbeat",
                                "worker heartbeat failed",
                                &e.to_string(),
                            );
                        }

                        // EVE-639: refresh the cached capacity snapshot here so the
                        // poll loop never has to run the aggregate itself. Done
                        // after the heartbeat so this worker's own fresh load is
                        // reflected. A failure is non-fatal: keep the prior value.
                        match store.get_capacity_snapshot().await {
                            Ok(snap) => *capacity_snapshot.write().unwrap() = snap,
                            Err(e) => debug!("Capacity snapshot refresh failed: {}", e),
                        }
                    }
                    _ = shutdown_rx.changed() => {
                        debug!("Heartbeat loop: shutdown requested");
                        break;
                    }
                }
            }

            debug!("Heartbeat loop exited");
        });

        *self.heartbeat_handle.lock().unwrap() = Some(handle);
    }

    /// Start the resource monitor loop (CPU/memory sampling)
    fn start_resource_monitor_loop(&self) {
        let interval = self.config.resource_monitor_interval;
        let backpressure = Arc::clone(&self.backpressure);
        let mut shutdown_rx = self.shutdown_rx.clone();

        let handle = tokio::spawn(async move {
            let mut monitor = ResourceMonitor::new();
            let mut ticker = tokio::time::interval(interval);

            // Take an initial sample so metrics are available immediately
            if let Some(metrics) = monitor.sample() {
                backpressure.update_resources(metrics);
            }

            loop {
                tokio::select! {
                    _ = ticker.tick() => {
                        if let Some(metrics) = monitor.sample() {
                            backpressure.update_resources(metrics);
                            debug!(
                                cpu = format!("{:.1}%", metrics.cpu_usage * 100.0),
                                memory_mb = format!("{:.0}", metrics.memory_used_bytes as f64 / 1_048_576.0),
                                "Resource sample"
                            );
                        }
                    }
                    _ = shutdown_rx.changed() => {
                        debug!("Resource monitor loop: shutdown requested");
                        break;
                    }
                }
            }

            debug!("Resource monitor loop exited");
        });

        *self.resource_handle.lock().unwrap() = Some(handle);
    }

    /// Start the stale task reclamation loop
    fn start_reclaim_loop(&self) {
        let store = Arc::clone(&self.store);
        let interval = self.config.stale_reclaim_interval;
        let threshold = self.config.stale_threshold;
        let mut shutdown_rx = self.shutdown_rx.clone();

        let handle = tokio::spawn(async move {
            let mut ticker = tokio::time::interval(interval);

            loop {
                tokio::select! {
                    _ = ticker.tick() => {
                        // Dead and sealed tasks are settled by the shared reaper;
                        // a seal means nothing more to a bare pool.
                        if let Err(e) = crate::maintenance::reap_stale_tasks(
                            store.as_ref(),
                            threshold,
                            &crate::maintenance::NoopReapHandler,
                        )
                        .await
                        {
                            crate::persistence::log_database_failure(
                                "durable.worker.reclaim_stale",
                                "stale task reclamation failed",
                                &e.to_string(),
                            );
                        }
                    }
                    _ = shutdown_rx.changed() => {
                        debug!("Reclaim loop: shutdown requested");
                        break;
                    }
                }
            }

            debug!("Reclaim loop exited");
        });

        *self.reclaim_handle.lock().unwrap() = Some(handle);
    }
}

/// Await an activity handler, heartbeating its task every `interval` so a
/// long-running activity is not reclaimed as stale while it still runs.
async fn run_with_task_heartbeat(
    store: &dyn WorkflowEventStore,
    task_id: Uuid,
    worker_id: &str,
    interval: Duration,
    mut handler: JoinHandle<ActivityResult>,
) -> ActivityResult {
    let mut ticker = tokio::time::interval(interval.max(Duration::from_millis(10)));
    ticker.tick().await; // the claim itself just stamped the heartbeat
    loop {
        tokio::select! {
            joined = &mut handler => {
                return joined.unwrap_or_else(|join_error| {
                    Err(format!("activity handler panicked: {join_error}"))
                });
            }
            _ = ticker.tick() => {
                if let Err(e) = store.heartbeat_task(task_id, worker_id, None).await {
                    debug!(%task_id, "Task heartbeat failed: {}", e);
                }
            }
        }
    }
}

/// Compute fair-share claim limit for a worker given its available slots
/// and the total system capacity. Returns the capped number of tasks to claim.
///
/// When there's only one worker (or capacity is unknown), returns my_available.
/// With multiple workers, each gets a proportional share:
///   ceil(my_available² / total_available), minimum 1.
pub(crate) fn fair_share_claim_limit(
    my_available: usize,
    total_available: u32,
    active_workers: u32,
) -> usize {
    if active_workers <= 1 || total_available == 0 {
        return my_available;
    }
    let share = (my_available as f64 / total_available as f64) * my_available as f64;
    (share.ceil() as usize).max(1)
}

/// Serde support for Duration as milliseconds
mod duration_millis {
    use serde::{Deserialize, Deserializer, Serialize, Serializer};
    use std::time::Duration;

    pub fn serialize<S>(duration: &Duration, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: Serializer,
    {
        duration.as_millis().serialize(serializer)
    }

    pub fn deserialize<'de, D>(deserializer: D) -> Result<Duration, D::Error>
    where
        D: Deserializer<'de>,
    {
        let millis = u64::deserialize(deserializer)?;
        Ok(Duration::from_millis(millis))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::persistence::TaskQueue;

    #[test]
    fn test_default_config() {
        let config = WorkerPoolConfig::default();
        assert!(!config.worker_id.is_empty());
        assert_eq!(config.worker_group, "default");
        assert_eq!(config.max_concurrency, 50);
        assert_eq!(config.heartbeat_interval, Duration::from_secs(5));
    }

    #[test]
    fn test_config_builder() {
        let config =
            WorkerPoolConfig::new(vec!["activity_a".to_string(), "activity_b".to_string()])
                .with_worker_id("test-worker")
                .with_worker_group("high-priority")
                .with_max_concurrency(20)
                .with_heartbeat_interval(Duration::from_secs(10));

        assert_eq!(config.worker_id, "test-worker");
        assert_eq!(config.worker_group, "high-priority");
        assert_eq!(config.activity_types, vec!["activity_a", "activity_b"]);
        assert_eq!(config.max_concurrency, 20);
        assert_eq!(config.heartbeat_interval, Duration::from_secs(10));
    }

    #[test]
    fn test_fair_share_single_worker() {
        // Single worker gets full allocation
        assert_eq!(fair_share_claim_limit(10, 10, 1), 10);
        assert_eq!(fair_share_claim_limit(5, 5, 0), 5);
    }

    #[test]
    fn test_fair_share_equal_workers() {
        // 3 workers, each with 10 available (total 30)
        // Each should get ceil(10 * 10/30) = ceil(3.33) = 4
        assert_eq!(fair_share_claim_limit(10, 30, 3), 4);
    }

    #[test]
    fn test_fair_share_unequal_workers() {
        // Worker A: 8 available, Worker B: 2 available, total: 10
        // A gets ceil(8 * 8/10) = ceil(6.4) = 7
        assert_eq!(fair_share_claim_limit(8, 10, 2), 7);
        // B gets ceil(2 * 2/10) = ceil(0.4) = 1 (minimum 1)
        assert_eq!(fair_share_claim_limit(2, 10, 2), 1);
    }

    #[test]
    fn test_fair_share_minimum_one() {
        // Even with tiny share, always claim at least 1
        assert_eq!(fair_share_claim_limit(1, 100, 10), 1);
    }

    use crate::persistence::{
        InMemoryWorkflowEventStore, TaskDefinition, TaskStatus, WorkerInfo, WorkerRegistry,
    };
    use crate::reliability::RetryPolicy;
    use crate::workflow::ActivityOptions;

    fn fast_config(activity_types: &[&str]) -> WorkerPoolConfig {
        WorkerPoolConfig::new(activity_types.iter().map(|t| t.to_string()).collect())
            .with_worker_id("pool-test")
            .with_poller(PollerConfig {
                min_interval: Duration::from_millis(5),
                max_interval: Duration::from_millis(20),
                ..PollerConfig::default()
            })
            .with_shutdown_timeout(Duration::from_secs(5))
    }

    async fn enqueue(store: &InMemoryWorkflowEventStore, activity_type: &str) -> Uuid {
        store
            .enqueue_task(TaskDefinition {
                workflow_id: None,
                activity_id: format!("{activity_type}-1"),
                activity_type: activity_type.to_string(),
                input: serde_json::json!({ "n": 2 }),
                options: ActivityOptions::default().with_retry(RetryPolicy::no_retry()),
            })
            .await
            .unwrap()
    }

    async fn wait_for_status(
        store: &InMemoryWorkflowEventStore,
        task_id: Uuid,
        status: TaskStatus,
    ) -> crate::persistence::TaskInfo {
        tokio::time::timeout(Duration::from_secs(5), async {
            loop {
                let info = store.get_task(task_id).await.unwrap();
                if info.status == status {
                    return info;
                }
                tokio::time::sleep(Duration::from_millis(5)).await;
            }
        })
        .await
        .unwrap_or_else(|_| panic!("task {task_id} never reached {status:?}"))
    }

    #[tokio::test]
    async fn test_pool_runs_handler_and_completes_task() {
        let store = Arc::new(InMemoryWorkflowEventStore::new());
        let pool = WorkerPool::new(store.clone(), fast_config(&["double"]));
        pool.register_handler("double", |task| async move {
            Ok(serde_json::json!(task.input["n"].as_i64().unwrap() * 2))
        });

        assert_eq!(pool.status(), WorkerPoolStatus::Stopped);
        pool.start().await.unwrap();
        assert_eq!(pool.status(), WorkerPoolStatus::Running);
        assert!(pool.is_accepting());
        assert!(matches!(
            pool.start().await,
            Err(WorkerPoolError::AlreadyRunning)
        ));

        let task_id = enqueue(&store, "double").await;
        let info = wait_for_status(&store, task_id, TaskStatus::Completed).await;
        assert_eq!(info.claimed_by.as_deref(), Some("pool-test"));

        pool.shutdown().await.unwrap();
        assert_eq!(pool.status(), WorkerPoolStatus::Stopped);
        assert!(!pool.is_accepting());
        // Shutting down a stopped pool is a no-op.
        pool.shutdown().await.unwrap();
    }

    #[tokio::test]
    async fn test_pool_heartbeats_a_long_task_so_it_is_not_reclaimed() {
        let store = Arc::new(InMemoryWorkflowEventStore::new());
        let mut config = fast_config(&["slow"]).with_heartbeat_interval(Duration::from_millis(20));
        config.stale_threshold = Duration::from_millis(100);
        config.stale_reclaim_interval = Duration::from_millis(20);
        let pool = WorkerPool::new(store.clone(), config);
        pool.register_handler("slow", |_task| async move {
            tokio::time::sleep(Duration::from_millis(400)).await;
            Ok(serde_json::json!("done"))
        });
        pool.start().await.unwrap();

        let task_id = enqueue(&store, "slow").await;
        let info = wait_for_status(&store, task_id, TaskStatus::Completed).await;
        assert_eq!(info.attempt, 1, "the running task must not be reclaimed");

        pool.shutdown().await.unwrap();
    }

    #[tokio::test]
    async fn test_pool_without_stale_reclaim_leaves_stale_tasks_alone() {
        let store = Arc::new(InMemoryWorkflowEventStore::new());
        store
            .register_worker(WorkerInfo::new("crashed", ["work"]))
            .await
            .unwrap();
        let task_id = enqueue(&store, "work").await;
        store
            .claim_task("crashed", &["work".to_string()], 1)
            .await
            .unwrap();

        let mut config = fast_config(&["other"]).without_stale_reclaim();
        config.stale_threshold = Duration::ZERO;
        let pool = WorkerPool::new(store.clone(), config);
        pool.start().await.unwrap();
        tokio::time::sleep(Duration::from_millis(100)).await;

        assert_eq!(
            store.get_task(task_id).await.unwrap().status,
            TaskStatus::Claimed,
            "another reaper owns stale tasks"
        );
        pool.shutdown().await.unwrap();
    }

    #[tokio::test]
    async fn test_pool_handler_error_fails_task() {
        let store = Arc::new(InMemoryWorkflowEventStore::new());
        let pool = WorkerPool::new(store.clone(), fast_config(&["flaky"]));
        pool.register_handler(
            "flaky",
            |_task| async move { Err("upstream 503".to_string()) },
        );
        pool.start().await.unwrap();

        let task_id = enqueue(&store, "flaky").await;
        let info = wait_for_status(&store, task_id, TaskStatus::Dead).await;
        assert_eq!(info.last_error.as_deref(), Some("upstream 503"));

        pool.shutdown().await.unwrap();
    }

    #[tokio::test]
    async fn test_pool_handler_panic_fails_task_and_frees_slot() {
        let store = Arc::new(InMemoryWorkflowEventStore::new());
        let pool = WorkerPool::new(store.clone(), fast_config(&["boom"]));
        pool.register_handler("boom", |_task| async move {
            if true {
                panic!("handler bug");
            }
            Ok(serde_json::json!(null))
        });
        pool.start().await.unwrap();

        let task_id = enqueue(&store, "boom").await;
        let info = wait_for_status(&store, task_id, TaskStatus::Dead).await;
        assert!(
            info.last_error.as_deref().unwrap().contains("panicked"),
            "unexpected error: {:?}",
            info.last_error
        );
        tokio::time::timeout(Duration::from_secs(5), async {
            while pool.current_load() != 0 {
                tokio::time::sleep(Duration::from_millis(5)).await;
            }
        })
        .await
        .expect("panicked task should release its slot");

        pool.shutdown().await.unwrap();
    }

    #[tokio::test]
    async fn test_pool_fails_task_without_handler() {
        let store = Arc::new(InMemoryWorkflowEventStore::new());
        let pool = WorkerPool::new(store.clone(), fast_config(&["known", "unknown"]));
        pool.register_handler("known", |_task| async move { Ok(serde_json::json!(null)) });
        pool.start().await.unwrap();

        let task_id = enqueue(&store, "unknown").await;
        let info = wait_for_status(&store, task_id, TaskStatus::Dead).await;
        assert!(info.last_error.unwrap().contains("no handler registered"));

        pool.shutdown().await.unwrap();
    }

    #[test]
    fn test_fair_share_zero_total() {
        // Edge case: total_available = 0
        assert_eq!(fair_share_claim_limit(5, 0, 3), 5);
    }
}
