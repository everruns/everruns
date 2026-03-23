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
use crate::persistence::{ClaimedTask, StoreError, WorkerInfo, WorkflowEventStore};

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

    /// Stale task reclamation interval
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
            max_concurrency: 1000,
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
/// # Example
///
/// ```ignore
/// use everruns_durable::worker::{WorkerPool, WorkerPoolConfig};
///
/// let config = WorkerPoolConfig::new(vec!["my_activity".to_string()])
///     .with_max_concurrency(10);
///
/// let pool = WorkerPool::new(store, config);
///
/// // Register activity handlers
/// pool.register_handler("my_activity", |task| async move {
///     // Execute activity
///     Ok(json!({"result": "success"}))
/// });
///
/// // Start the worker pool
/// pool.start().await?;
///
/// // ... later, graceful shutdown
/// pool.shutdown().await?;
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
        self.start_reclaim_loop();

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

                // Cap claim batch to fair share of total system capacity
                let claim_limit = match store.get_capacity_snapshot().await {
                    Ok(snap) => fair_share_claim_limit(
                        my_available,
                        snap.total_available,
                        snap.active_workers,
                    ),
                    Err(_) => my_available, // error: no cap
                };

                // Poll for tasks
                match poller.poll(claim_limit).await {
                    Ok(tasks) => {
                        for task in tasks {
                            // Get handler
                            let handler = match handlers.get(&task.activity_type) {
                                Some(h) => Arc::clone(h),
                                None => {
                                    warn!(
                                        activity_type = %task.activity_type,
                                        "No handler registered"
                                    );
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

                            tokio::spawn(async move {
                                let task_id = task.id;
                                let result = handler(task).await;

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
                        error!("Poll error: {}", e);
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

        let handle = tokio::spawn(async move {
            let mut ticker = tokio::time::interval(interval);

            loop {
                tokio::select! {
                    _ = ticker.tick() => {
                        let load = backpressure.current_load();
                        let accepting = backpressure.is_accepting()
                            && !backpressure.is_resource_pressured();

                        if let Err(e) = store.worker_heartbeat(&worker_id, load, accepting).await {
                            error!("Heartbeat failed: {}", e);
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
            let metrics = monitor.sample();
            backpressure.update_resources(metrics);

            loop {
                tokio::select! {
                    _ = ticker.tick() => {
                        let metrics = monitor.sample();
                        backpressure.update_resources(metrics);
                        debug!(
                            cpu = format!("{:.1}%", metrics.cpu_usage * 100.0),
                            memory_mb = format!("{:.0}", metrics.memory_used_bytes as f64 / 1_048_576.0),
                            "Resource sample"
                        );
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
                        match store.reclaim_stale_tasks(threshold).await {
                            Ok(result) => {
                                if !result.reclaimed_ids.is_empty() {
                                    info!(count = result.reclaimed_ids.len(), "Reclaimed stale tasks");
                                }

                                // Notify workflows about dead tasks so they can transition
                                // to failed instead of staying stuck in running forever.
                                for dead in &result.dead_tasks {
                                    let error_msg = dead.last_error.clone().unwrap_or_else(|| {
                                        "Worker became unresponsive after exhausting all retry attempts".to_string()
                                    });
                                    info!(
                                        task_id = %dead.task_id,
                                        workflow_id = ?dead.workflow_id,
                                        activity_id = %dead.activity_id,
                                        "Notifying workflow of dead task"
                                    );
                                    crate::task_events::record_activity_failed(
                                        store.as_ref(),
                                        dead.workflow_id,
                                        dead.activity_id.clone(),
                                        error_msg,
                                        false, // will_retry = false, all attempts exhausted
                                    )
                                    .await;
                                }
                            }
                            Err(e) => {
                                error!("Stale task reclamation failed: {}", e);
                            }
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

    #[test]
    fn test_default_config() {
        let config = WorkerPoolConfig::default();
        assert!(!config.worker_id.is_empty());
        assert_eq!(config.worker_group, "default");
        assert_eq!(config.max_concurrency, 1000);
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
    fn test_worker_pool_status() {
        assert_ne!(WorkerPoolStatus::Running, WorkerPoolStatus::Stopped);
        assert_ne!(WorkerPoolStatus::Draining, WorkerPoolStatus::Running);
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

    #[test]
    fn test_fair_share_zero_total() {
        // Edge case: total_available = 0
        assert_eq!(fair_share_claim_limit(5, 0, 3), 5);
    }
}
