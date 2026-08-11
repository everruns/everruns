// Task Worker Implementation
//
// Decision: Single worker implementation generic over WorkerAdapters
// Decision: Works with both gRPC (external) and Direct (in-process) adapters
// Decision: Replaces both InProcessWorker and DurableWorker
//
// TaskWorker executes activities (input, reason, act) from the durable task queue.
// It unifies the two worker implementations into one, eliminating code duplication
// while preserving the different deployment models (in-process vs external).

use anyhow::Result;
use async_trait::async_trait;
use everruns_core::ActInput;
use everruns_core::atoms::AtomContext;
use everruns_core::typed_id::{ExecId, TurnId};
use everruns_durable::{
    ActivityOptions, ClaimedTask, HeartbeatResponse, StoreError, TaskDefinition,
    TaskFailureOutcome, WorkerInfo, WorkflowError, WorkflowEvent, WorkflowEventStore,
    WorkflowStatus, append_event, record_activity_completed, record_activity_failed,
    record_activity_started,
};
use everruns_host::{
    RuntimeActPlan, RuntimeTurnPlan, execute_act_activity as runtime_execute_act_activity,
    execute_input_activity as runtime_execute_input_activity,
    execute_reason_activity as runtime_execute_reason_activity, plan_next_host_turn,
};
use std::sync::Arc;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::time::Duration;
use tokio::sync::watch;
use tokio::task::JoinSet;
use tracing::{debug, error, info, warn};
use uuid::Uuid;

use crate::durable_runner::DurableTurnInput;
use crate::grpc_durable_store::GrpcDurableStore;
use crate::runtime_host::WorkerRuntimeHost;
use crate::task_error::summarize_task_failure;
use crate::worker_adapters::WorkerAdapters;
use crate::{
    activities::ScheduledAgentTriggerInput, activities::ScheduledAppChannelInput,
    activities::activity_types,
};

// Re-export atom types
pub use everruns_core::atoms::{InputAtomInput, ReasonInput, ReasonResult};

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
                activity_types::INVOKE_SCHEDULED_APP_CHANNEL.to_string(),
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
        use everruns_core::config::{env_duration_ms, env_duration_secs, env_or, env_string_any};

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
            ..defaults
        }
    }
}

// =============================================================================
// Unified Worker
// =============================================================================

#[async_trait]
pub trait TaskStore: Send + Sync + 'static {
    async fn register_worker(&self, worker: WorkerInfo) -> Result<(), StoreError>;

    async fn worker_heartbeat(
        &self,
        worker_id: &str,
        current_load: usize,
        accepting_tasks: bool,
    ) -> Result<(), StoreError>;

    async fn deregister_worker(&self, worker_id: &str) -> Result<usize, StoreError>;

    async fn claim_task(
        &self,
        worker_id: &str,
        activity_types: &[String],
        max_tasks: usize,
    ) -> Result<Vec<ClaimedTask>, StoreError>;

    async fn heartbeat_task(
        &self,
        task_id: Uuid,
        worker_id: &str,
        details: Option<serde_json::Value>,
    ) -> Result<HeartbeatResponse, StoreError>;

    async fn get_workflow_status(&self, workflow_id: Uuid) -> Result<WorkflowStatus, StoreError>;

    async fn record_activity_started(&self, task: &ClaimedTask, worker_id: &str);

    async fn complete_task_and_record(
        &self,
        task: &ClaimedTask,
        worker_id: &str,
        output: serde_json::Value,
    ) -> Result<(), StoreError>;

    async fn fail_task_and_record(
        &self,
        task: &ClaimedTask,
        error: &str,
    ) -> Result<TaskFailureOutcome, StoreError>;

    async fn enqueue_task_and_record(
        &self,
        workflow_id: Uuid,
        activity_id: String,
        activity_type: String,
        input: serde_json::Value,
    ) -> Result<Uuid, StoreError>;

    async fn update_workflow_status(
        &self,
        workflow_id: Uuid,
        status: WorkflowStatus,
        output: Option<serde_json::Value>,
        error: Option<WorkflowError>,
    ) -> Result<(), StoreError>;

    async fn complete_workflow(
        &self,
        workflow_id: Uuid,
        event_output: serde_json::Value,
        stored_output: Option<serde_json::Value>,
        error: Option<WorkflowError>,
    ) -> Result<(), StoreError>;

    async fn consume_pending_signals(
        &self,
        workflow_id: Uuid,
    ) -> Result<Vec<everruns_durable::WorkflowSignal>, StoreError>;

    async fn consume_pending_signals_by_type(
        &self,
        workflow_id: Uuid,
        signal_type: &str,
    ) -> Result<Vec<everruns_durable::WorkflowSignal>, StoreError>;
}

#[async_trait]
impl<S> TaskStore for S
where
    S: WorkflowEventStore,
{
    async fn register_worker(&self, worker: WorkerInfo) -> Result<(), StoreError> {
        WorkflowEventStore::register_worker(self, worker).await
    }

    async fn worker_heartbeat(
        &self,
        worker_id: &str,
        current_load: usize,
        accepting_tasks: bool,
    ) -> Result<(), StoreError> {
        WorkflowEventStore::worker_heartbeat(self, worker_id, current_load, accepting_tasks).await
    }

    async fn deregister_worker(&self, worker_id: &str) -> Result<usize, StoreError> {
        WorkflowEventStore::deregister_worker(self, worker_id).await
    }

    async fn claim_task(
        &self,
        worker_id: &str,
        activity_types: &[String],
        max_tasks: usize,
    ) -> Result<Vec<ClaimedTask>, StoreError> {
        WorkflowEventStore::claim_task(self, worker_id, activity_types, max_tasks).await
    }

    async fn heartbeat_task(
        &self,
        task_id: Uuid,
        worker_id: &str,
        details: Option<serde_json::Value>,
    ) -> Result<HeartbeatResponse, StoreError> {
        WorkflowEventStore::heartbeat_task(self, task_id, worker_id, details).await
    }

    async fn get_workflow_status(&self, workflow_id: Uuid) -> Result<WorkflowStatus, StoreError> {
        WorkflowEventStore::get_workflow_status(self, workflow_id).await
    }

    async fn record_activity_started(&self, task: &ClaimedTask, worker_id: &str) {
        record_activity_started(
            self,
            task.workflow_id,
            task.activity_id.clone(),
            task.attempt,
            worker_id.to_string(),
        )
        .await;
    }

    async fn complete_task_and_record(
        &self,
        task: &ClaimedTask,
        worker_id: &str,
        output: serde_json::Value,
    ) -> Result<(), StoreError> {
        WorkflowEventStore::complete_task(self, task.id, worker_id, output.clone()).await?;
        record_activity_completed(self, task.workflow_id, task.activity_id.clone(), output).await;
        Ok(())
    }

    async fn fail_task_and_record(
        &self,
        task: &ClaimedTask,
        error: &str,
    ) -> Result<TaskFailureOutcome, StoreError> {
        let will_retry = task.attempt < task.max_attempts;
        record_activity_failed(
            self,
            task.workflow_id,
            task.activity_id.clone(),
            error.to_string(),
            will_retry,
        )
        .await;
        WorkflowEventStore::fail_task(self, task.id, error).await
    }

    async fn enqueue_task_and_record(
        &self,
        workflow_id: Uuid,
        activity_id: String,
        activity_type: String,
        input: serde_json::Value,
    ) -> Result<Uuid, StoreError> {
        let event = WorkflowEvent::ActivityScheduled {
            activity_id: activity_id.clone(),
            activity_type: activity_type.clone(),
            input: input.clone(),
            options: ActivityOptions::default(),
        };
        append_event(self, workflow_id, event).await?;
        WorkflowEventStore::enqueue_task(
            self,
            TaskDefinition {
                workflow_id: Some(workflow_id),
                activity_id,
                activity_type,
                input,
                options: ActivityOptions::default(),
            },
        )
        .await
    }

    async fn update_workflow_status(
        &self,
        workflow_id: Uuid,
        status: WorkflowStatus,
        output: Option<serde_json::Value>,
        error: Option<WorkflowError>,
    ) -> Result<(), StoreError> {
        WorkflowEventStore::update_workflow_status(self, workflow_id, status, output, error).await
    }

    async fn complete_workflow(
        &self,
        workflow_id: Uuid,
        event_output: serde_json::Value,
        stored_output: Option<serde_json::Value>,
        error: Option<WorkflowError>,
    ) -> Result<(), StoreError> {
        everruns_durable::record_workflow_completed(self, workflow_id, event_output).await;
        WorkflowEventStore::update_workflow_status(
            self,
            workflow_id,
            WorkflowStatus::Completed,
            stored_output,
            error,
        )
        .await
    }

    async fn consume_pending_signals(
        &self,
        workflow_id: Uuid,
    ) -> Result<Vec<everruns_durable::WorkflowSignal>, StoreError> {
        WorkflowEventStore::consume_pending_signals(self, workflow_id).await
    }

    async fn consume_pending_signals_by_type(
        &self,
        workflow_id: Uuid,
        signal_type: &str,
    ) -> Result<Vec<everruns_durable::WorkflowSignal>, StoreError> {
        WorkflowEventStore::consume_pending_signals_by_type(self, workflow_id, signal_type).await
    }
}

#[async_trait]
impl TaskStore for GrpcDurableStore {
    async fn register_worker(&self, worker: WorkerInfo) -> Result<(), StoreError> {
        let mut store = self.clone();
        GrpcDurableStore::register_worker(
            &mut store,
            &worker.id,
            worker.worker_group,
            worker.activity_types,
            worker.max_concurrency,
        )
        .await
        .map_err(store_error)
    }

    async fn worker_heartbeat(
        &self,
        worker_id: &str,
        current_load: usize,
        accepting_tasks: bool,
    ) -> Result<(), StoreError> {
        let mut store = self.clone();
        GrpcDurableStore::heartbeat_worker(
            &mut store,
            worker_id,
            current_load as u32,
            accepting_tasks,
        )
        .await
        .map_err(store_error)
    }

    async fn deregister_worker(&self, worker_id: &str) -> Result<usize, StoreError> {
        let mut store = self.clone();
        GrpcDurableStore::deregister_worker(&mut store, worker_id)
            .await
            .map_err(store_error)
    }

    async fn claim_task(
        &self,
        worker_id: &str,
        activity_types: &[String],
        max_tasks: usize,
    ) -> Result<Vec<ClaimedTask>, StoreError> {
        let mut store = self.clone();
        GrpcDurableStore::claim_tasks(&mut store, worker_id, activity_types, max_tasks)
            .await
            .map_err(store_error)
    }

    async fn heartbeat_task(
        &self,
        task_id: Uuid,
        worker_id: &str,
        details: Option<serde_json::Value>,
    ) -> Result<HeartbeatResponse, StoreError> {
        let mut store = self.clone();
        let response = GrpcDurableStore::heartbeat_task(&mut store, task_id, worker_id, details)
            .await
            .map_err(store_error)?;
        Ok(HeartbeatResponse {
            accepted: response.acknowledged,
            should_cancel: response.should_cancel,
        })
    }

    async fn get_workflow_status(&self, workflow_id: Uuid) -> Result<WorkflowStatus, StoreError> {
        let mut store = self.clone();
        let (status, _, _) = GrpcDurableStore::get_workflow_status(&mut store, workflow_id)
            .await
            .map_err(store_error)?;
        Ok(grpc_status_to_workflow_status(status))
    }

    async fn record_activity_started(&self, _task: &ClaimedTask, _worker_id: &str) {
        // The control plane owns workflow history for gRPC workers. Claim,
        // complete, fail, and enqueue RPCs record the durable task events.
    }

    async fn complete_task_and_record(
        &self,
        task: &ClaimedTask,
        worker_id: &str,
        output: serde_json::Value,
    ) -> Result<(), StoreError> {
        let mut store = self.clone();
        GrpcDurableStore::complete_task(&mut store, task.id, worker_id, output)
            .await
            .map_err(store_error)
    }

    async fn fail_task_and_record(
        &self,
        task: &ClaimedTask,
        error: &str,
    ) -> Result<TaskFailureOutcome, StoreError> {
        let mut store = self.clone();
        let will_retry = GrpcDurableStore::fail_task(&mut store, task.id, error)
            .await
            .map_err(store_error)?;
        Ok(if will_retry {
            TaskFailureOutcome::WillRetry {
                next_attempt: task.attempt + 1,
                delay: Duration::ZERO,
            }
        } else {
            TaskFailureOutcome::MovedToDlq
        })
    }

    async fn enqueue_task_and_record(
        &self,
        workflow_id: Uuid,
        activity_id: String,
        activity_type: String,
        input: serde_json::Value,
    ) -> Result<Uuid, StoreError> {
        let mut store = self.clone();
        GrpcDurableStore::enqueue_task(&mut store, workflow_id, activity_id, activity_type, input)
            .await
            .map_err(store_error)
    }

    async fn update_workflow_status(
        &self,
        workflow_id: Uuid,
        status: WorkflowStatus,
        output: Option<serde_json::Value>,
        error: Option<WorkflowError>,
    ) -> Result<(), StoreError> {
        let mut store = self.clone();
        GrpcDurableStore::update_workflow_status(
            &mut store,
            workflow_id,
            workflow_status_to_grpc_status(status),
            output,
            error.map(|err| err.message),
        )
        .await
        .map_err(store_error)
    }

    async fn complete_workflow(
        &self,
        workflow_id: Uuid,
        _event_output: serde_json::Value,
        stored_output: Option<serde_json::Value>,
        error: Option<WorkflowError>,
    ) -> Result<(), StoreError> {
        self.update_workflow_status(workflow_id, WorkflowStatus::Completed, stored_output, error)
            .await
    }

    async fn consume_pending_signals(
        &self,
        workflow_id: Uuid,
    ) -> Result<Vec<everruns_durable::WorkflowSignal>, StoreError> {
        let mut store = self.clone();
        GrpcDurableStore::get_and_consume_signals(&mut store, workflow_id)
            .await
            .map_err(store_error)
    }

    async fn consume_pending_signals_by_type(
        &self,
        workflow_id: Uuid,
        signal_type: &str,
    ) -> Result<Vec<everruns_durable::WorkflowSignal>, StoreError> {
        let mut store = self.clone();
        GrpcDurableStore::get_and_consume_signals_by_type(&mut store, workflow_id, signal_type)
            .await
            .map_err(store_error)
    }
}

fn store_error(error: anyhow::Error) -> StoreError {
    StoreError::Database(error.to_string())
}

fn grpc_status_to_workflow_status(
    status: crate::grpc_durable_store::WorkflowStatus,
) -> WorkflowStatus {
    match status {
        crate::grpc_durable_store::WorkflowStatus::Pending => WorkflowStatus::Pending,
        crate::grpc_durable_store::WorkflowStatus::Running => WorkflowStatus::Running,
        crate::grpc_durable_store::WorkflowStatus::Completed => WorkflowStatus::Completed,
        crate::grpc_durable_store::WorkflowStatus::Failed => WorkflowStatus::Failed,
        crate::grpc_durable_store::WorkflowStatus::Cancelled => WorkflowStatus::Cancelled,
        crate::grpc_durable_store::WorkflowStatus::ContinuedAsNew => WorkflowStatus::ContinuedAsNew,
    }
}

fn workflow_status_to_grpc_status(
    status: WorkflowStatus,
) -> crate::grpc_durable_store::WorkflowStatus {
    match status {
        WorkflowStatus::Pending => crate::grpc_durable_store::WorkflowStatus::Pending,
        WorkflowStatus::Running => crate::grpc_durable_store::WorkflowStatus::Running,
        WorkflowStatus::Completed => crate::grpc_durable_store::WorkflowStatus::Completed,
        WorkflowStatus::Failed => crate::grpc_durable_store::WorkflowStatus::Failed,
        WorkflowStatus::Cancelled => crate::grpc_durable_store::WorkflowStatus::Cancelled,
        WorkflowStatus::ContinuedAsNew => crate::grpc_durable_store::WorkflowStatus::ContinuedAsNew,
    }
}

/// Unified worker that executes tasks from the durable task queue
///
/// This worker is generic over:
/// - `S`: TaskStore implementation (direct store or gRPC store)
/// - `A`: WorkerAdapters implementation (Direct or gRPC)
pub struct TaskWorker<S, A>
where
    S: TaskStore,
    A: WorkerAdapters,
{
    config: TaskWorkerConfig,
    store: Arc<S>,
    adapters: A,
    shutdown_tx: watch::Sender<bool>,
    shutdown_rx: watch::Receiver<bool>,
    in_flight: Arc<AtomicUsize>,
}

impl<S, A> TaskWorker<S, A>
where
    S: TaskStore,
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
            "Initialized unified worker"
        );

        Self {
            config,
            store,
            adapters,
            shutdown_tx,
            shutdown_rx,
            in_flight: Arc::new(AtomicUsize::new(0)),
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
                    if executed == 0 {
                        tokio::select! {
                            _ = tokio::time::sleep(poll_backoff) => {}
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
                        executed > 0,
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
            let store = self.store.clone();
            let adapters = self.adapters.clone();
            let worker_id = self.config.worker_id.clone();
            let in_flight_guard = InFlightTaskGuard::increment(self.in_flight.clone());
            let heartbeat_interval = self.config.heartbeat_interval;

            task_handles.spawn(async move {
                let _in_flight_guard = in_flight_guard;
                let result =
                    execute_task(&store, &adapters, &worker_id, heartbeat_interval, &task).await;

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

// =============================================================================
// Task Execution
// =============================================================================

/// Execute a single task
async fn execute_task<S, A>(
    store: &Arc<S>,
    adapters: &A,
    worker_id: &str,
    heartbeat_interval: Duration,
    task: &ClaimedTask,
) -> Result<()>
where
    S: TaskStore,
    A: WorkerAdapters,
{
    info!(
        task_id = %task.id,
        workflow_id = ?task.workflow_id,
        activity_type = %task.activity_type,
        attempt = task.attempt,
        "Executing task"
    );

    // Check if workflow is cancelled (only for workflow-bound tasks)
    if let Some(wf_id) = task.workflow_id {
        let workflow_status = store.get_workflow_status(wf_id).await;
        if let Ok(status) = workflow_status
            && status == WorkflowStatus::Cancelled
        {
            info!(
                task_id = %task.id,
                workflow_id = %wf_id,
                "Workflow cancelled, skipping task"
            );
            let _ = store.fail_task_and_record(task, "Workflow cancelled").await;
            return Ok(());
        }
    }

    store.record_activity_started(task, worker_id).await;

    let (heartbeat_cancel_tx, heartbeat_handle) = spawn_task_heartbeat(
        store.clone(),
        task.id,
        worker_id.to_string(),
        heartbeat_interval,
    );

    // Execute based on activity type. Keep fallible parsing inside this result so
    // cleanup below runs before malformed tasks are failed.
    let execution = async {
        let (result, turn_input_opt) = match task.activity_type.as_str() {
            "process_input" | "reason" => {
                let turn_input: DurableTurnInput = serde_json::from_value(task.input.clone())
                    .map_err(|e| anyhow::anyhow!("Failed to parse task input: {}", e))?;

                let res = match task.activity_type.as_str() {
                    "process_input" => execute_input_activity(adapters, &turn_input).await,
                    "reason" => execute_reason_activity(adapters, &turn_input).await,
                    _ => unreachable!(),
                };
                (res, Some(turn_input))
            }
            "act" => {
                let act_input: ActInput = serde_json::from_value(task.input.clone())
                    .map_err(|e| anyhow::anyhow!("Failed to parse ActInput: {}", e))?;

                // Extract previous_response_id injected by reason→act scheduling
                let previous_response_id = task
                    .input
                    .get("previous_response_id")
                    .and_then(|v| v.as_str())
                    .map(|s| s.to_string());

                // Extract iteration count from act task input (carried through from reason)
                let iteration = task
                    .input
                    .get("iteration")
                    .and_then(|v| v.as_u64())
                    .and_then(|raw| u32::try_from(raw).ok())
                    .filter(|&it| it > 0)
                    .unwrap_or(1);

                let act_request_id = task
                    .input
                    .get("request_id")
                    .and_then(|v| v.as_str())
                    .map(|s| s.to_string());
                let resume_state = parse_resume_state(&task.input)?;

                // Create DurableTurnInput from ActInput context
                let mut turn_input = resume_state.unwrap_or(DurableTurnInput {
                    org_id: act_input.org_id.ok_or_else(|| {
                        anyhow::anyhow!("ActInput.org_id must be set for durable turns")
                    })?,
                    session_id: act_input.context.session_id,
                    harness_id: act_input.harness_id,
                    agent_id: act_input.agent_id,
                    input_message_id: act_input.context.input_message_id,
                    turn_id: Some(act_input.context.turn_id),
                    previous_response_id: previous_response_id.clone(),
                    iteration,
                    request_id: act_request_id.clone(),
                    started_at: None,
                    cumulative_usage: None,
                    tool_call_count: 0,
                    llm_call_count: 0,
                    time_to_first_token_ms: None,
                    final_message_id: None,
                    final_answer_preview: None,
                });
                turn_input.previous_response_id = previous_response_id;
                turn_input.iteration = iteration;
                turn_input.request_id = act_request_id;

                let res = execute_act_activity(adapters, &act_input).await;
                (res, Some(turn_input))
            }
            "leased_resource_cleanup" => {
                let cleanup_input: crate::leased_resource_cleanup::LeasedResourceCleanupInput =
                    serde_json::from_value(task.input.clone())
                        .map_err(|e| anyhow::anyhow!("Failed to parse cleanup input: {}", e))?;
                let res = crate::leased_resource_cleanup::execute_cleanup_activity(
                    adapters,
                    &cleanup_input,
                )
                .await;
                (res, None)
            }
            "session_task_reaper" => {
                let reaper_input: crate::session_task_reaper::SessionTaskReaperInput =
                    serde_json::from_value(task.input.clone())
                        .map_err(|e| anyhow::anyhow!("Failed to parse reaper input: {}", e))?;
                let res =
                    crate::session_task_reaper::execute_reaper_activity(adapters, &reaper_input)
                        .await;
                (res, None)
            }
            activity_types::INVOKE_SCHEDULED_APP_CHANNEL => {
                let input: ScheduledAppChannelInput = serde_json::from_value(task.input.clone())
                    .map_err(|e| anyhow::anyhow!("Failed to parse scheduled app input: {}", e))?;
                let res = adapters
                    .invoke_scheduled_app_channel(input.org_id, &input.app_id, &input.channel_id)
                    .await
                    .map_err(anyhow::Error::from);
                (res, None)
            }
            activity_types::INVOKE_AGENT_TRIGGER => {
                let input: ScheduledAgentTriggerInput = serde_json::from_value(task.input.clone())
                    .map_err(|e| anyhow::anyhow!("Failed to parse agent trigger input: {}", e))?;
                let res = adapters
                    .invoke_agent_trigger(input.org_id, &input.agent_id, &input.trigger_id)
                    .await
                    .map_err(anyhow::Error::from);
                (res, None)
            }
            _ => (
                Err(anyhow::anyhow!(
                    "Unknown activity type: {}",
                    task.activity_type
                )),
                None,
            ),
        };
        Ok::<_, anyhow::Error>((result, turn_input_opt))
    }
    .await;

    let _ = heartbeat_cancel_tx.send(());
    let _ = heartbeat_handle.await;

    let (result, turn_input_opt) = match execution {
        Ok(execution) => execution,
        Err(e) => {
            let failure = summarize_task_failure(
                task.id,
                task.workflow_id,
                &task.activity_type,
                task.attempt,
                Some(task.max_attempts),
                &task.input,
                &e,
            );
            let _ = store
                .fail_task_and_record(task, &failure.persisted_message)
                .await;

            return Err(e);
        }
    };

    match result {
        Ok(output) => {
            // Complete the task - verify ownership
            let complete_result = store
                .complete_task_and_record(task, worker_id, output.clone())
                .await;

            match complete_result {
                Ok(()) => {
                    info!(
                        task_id = %task.id,
                        activity_type = %task.activity_type,
                        "Task completed successfully"
                    );

                    // Schedule next activity if needed (only for workflow-bound tasks)
                    if let (Some(turn_input), Some(wf_id)) = (turn_input_opt, task.workflow_id) {
                        schedule_next_activity(
                            store,
                            adapters,
                            wf_id,
                            &task.activity_type,
                            &turn_input,
                            &output,
                        )
                        .await?;
                    }
                }
                Err(e) => {
                    warn!(
                        task_id = %task.id,
                        error = %e,
                        "Task completion rejected - skipping next activity"
                    );
                }
            }
        }
        Err(e) => {
            let failure = summarize_task_failure(
                task.id,
                task.workflow_id,
                &task.activity_type,
                task.attempt,
                Some(task.max_attempts),
                &task.input,
                &e,
            );
            let _ = store
                .fail_task_and_record(task, &failure.persisted_message)
                .await;

            return Err(e);
        }
    }

    Ok(())
}

fn spawn_task_heartbeat<S: TaskStore>(
    store: Arc<S>,
    task_id: Uuid,
    worker_id: String,
    heartbeat_interval: Duration,
) -> (
    tokio::sync::oneshot::Sender<()>,
    tokio::task::JoinHandle<()>,
) {
    let (cancel_tx, mut cancel_rx) = tokio::sync::oneshot::channel::<()>();
    let handle = tokio::spawn(async move {
        let mut interval = tokio::time::interval(heartbeat_interval);
        loop {
            tokio::select! {
                _ = interval.tick() => {
                    match store.heartbeat_task(task_id, &worker_id, None).await {
                        Ok(response) => {
                            if response.should_cancel {
                                warn!(task_id = %task_id, "Task cancellation requested via heartbeat");
                                break;
                            }
                            debug!(task_id = %task_id, "Task heartbeat sent");
                        }
                        Err(error) => {
                            warn!(task_id = %task_id, error = %error, "Failed to send task heartbeat");
                        }
                    }
                }
                _ = &mut cancel_rx => {
                    debug!(task_id = %task_id, "Task heartbeat loop cancelled");
                    break;
                }
            }
        }
    });
    (cancel_tx, handle)
}

fn parse_resume_state(input: &serde_json::Value) -> Result<Option<DurableTurnInput>> {
    match input.get("resume_state") {
        Some(value) if value.is_null() => Ok(None),
        Some(value) => serde_json::from_value(value.clone())
            .map(Some)
            .map_err(|e| anyhow::anyhow!("Failed to parse resume_state: {}", e)),
        None => Ok(None),
    }
}

// =============================================================================
// Activity Implementations
// =============================================================================

/// Execute input processing activity
async fn execute_input_activity<A: WorkerAdapters>(
    adapters: &A,
    input: &DurableTurnInput,
) -> Result<serde_json::Value> {
    debug!(
        session_id = %input.session_id,
        "Executing input activity"
    );

    // Create AtomContext
    let context = AtomContext {
        // Input/Reason atoms do not key file I/O by AtomContext; the act
        // path receives its workspace-set context from the orchestration.
        workspace_id: None,
        session_id: input.session_id,
        turn_id: TurnId::new(),
        input_message_id: input.input_message_id,
        exec_id: ExecId::new(),
    };

    let atom_input = everruns_core::InputAtomInput {
        context: context.clone(),
    };
    let result = runtime_execute_input_activity(
        &WorkerRuntimeHost::new(adapters.clone()),
        input.org_id,
        atom_input,
    )
    .await?;

    // Include turn_id in output for propagation
    let mut output = serde_json::to_value(&result)?;
    if let serde_json::Value::Object(ref mut map) = output {
        map.insert(
            "turn_id".to_string(),
            serde_json::json!(context.turn_id.to_string()),
        );
    }
    Ok(output)
}

/// Execute reasoning activity (LLM call)
async fn execute_reason_activity<A: WorkerAdapters>(
    adapters: &A,
    input: &DurableTurnInput,
) -> Result<serde_json::Value> {
    debug!(
        session_id = %input.session_id,
        turn_id = ?input.turn_id,
        "Executing reason activity"
    );

    // Create AtomContext - use turn_id from input if available
    let turn_id = input.turn_id.unwrap_or_default();
    let context = AtomContext {
        // Input/Reason atoms do not key file I/O by AtomContext; the act
        // path receives its workspace-set context from the orchestration.
        workspace_id: None,
        session_id: input.session_id,
        turn_id,
        input_message_id: input.input_message_id,
        exec_id: ExecId::new(),
    };

    let reason_input = everruns_core::ReasonInput {
        context: context.clone(),
        harness_id: input.harness_id,
        agent_id: input.agent_id,
        org_id: input.org_id,
        mcp_tool_definitions: vec![],
        previous_response_id: input.previous_response_id.clone(),
        iteration: input.iteration,
    };

    let event_metadata = input.agent_id.map(|agent_id| {
        let mut metadata = serde_json::Map::new();
        metadata.insert(
            "agent_id".to_string(),
            serde_json::Value::String(agent_id.to_string()),
        );
        metadata
    });
    let result = runtime_execute_reason_activity(
        &WorkerRuntimeHost::with_event_metadata(adapters.clone(), event_metadata),
        input.org_id,
        reason_input,
    )
    .await?;

    // Turn lifecycle events (turn.completed, turn.failed, session.idled) are NOT
    // emitted here. They are deferred to the workflow scheduler which checks for
    // pending steering signals before deciding whether the turn is truly done.
    // This ensures turn.completed is emitted exactly once and prevents the
    // idle→active flicker when steering continues the turn.

    Ok(serde_json::to_value(&result)?)
}

/// Execute act activity (tool execution)
async fn execute_act_activity<A: WorkerAdapters>(
    adapters: &A,
    input: &ActInput,
) -> Result<serde_json::Value> {
    debug!(
        session_id = %input.context.session_id,
        tool_count = input.tool_calls.len(),
        "Executing act activity"
    );

    // Extract org_id early — must be set by callers for proper tenant isolation.
    let result =
        runtime_execute_act_activity(&WorkerRuntimeHost::new(adapters.clone()), input.clone())
            .await?;

    Ok(serde_json::to_value(&result)?)
}

// =============================================================================
// Activity Scheduling
// =============================================================================

/// Schedule the next activity based on current activity completion
async fn schedule_next_activity<S: TaskStore, A: WorkerAdapters + Clone>(
    store: &Arc<S>,
    adapters: &A,
    workflow_id: Uuid,
    completed_activity: &str,
    input: &DurableTurnInput,
    output: &serde_json::Value,
) -> Result<()> {
    // A reason that produced a final answer (no tool calls) is winding the turn
    // down; a reason that emitted tool calls is not.
    let reason_final_answer = if completed_activity == "reason" {
        let reason_result: everruns_core::ReasonResult = serde_json::from_value(output.clone())
            .map_err(|error| anyhow::anyhow!("Invalid reason output payload: {}", error))?;
        reason_result.success && !reason_result.has_tool_calls
    } else {
        false
    };

    // Drain queued USER_MESSAGE steering signals (task wakes) at the boundaries
    // that precede another reason iteration. The already-persisted wake message
    // is picked up by that reason (it re-reads full history); consuming the
    // signal here is what governs turn continuation and, being destructive,
    // gives exactly-once delivery — see `drains_wake_signals_after`.
    let pending_user_message_count =
        count_drained_wakes(store, workflow_id, completed_activity, reason_final_answer).await?;

    if completed_activity == "act" && pending_user_message_count > 0 {
        debug!(
            %workflow_id,
            pending_user_message_count,
            "delivering mid-turn task wake(s) at the act→reason boundary"
        );
    }

    match plan_next_host_turn(
        &WorkerRuntimeHost::new(adapters.clone()),
        completed_activity,
        input,
        output,
        pending_user_message_count,
    )
    .await?
    {
        RuntimeTurnPlan::ScheduleReason(next) => {
            enqueue_reason_task(store, workflow_id, &next).await?;
        }
        RuntimeTurnPlan::ScheduleAct(plan) => {
            enqueue_act_task(store, workflow_id, &plan).await?;
        }
        RuntimeTurnPlan::Complete { stop_reason, error } => {
            let turn_output = turn_output_with_stop_reason(output.clone(), stop_reason);
            store
                .complete_workflow(
                    workflow_id,
                    turn_output,
                    None,
                    error.map(everruns_durable::WorkflowError::new),
                )
                .await
                .map_err(|e| anyhow::anyhow!("Failed to update workflow status: {}", e))?;
        }
        RuntimeTurnPlan::WaitForToolResults { resume } => {
            store
                .complete_workflow(
                    workflow_id,
                    output.clone(),
                    Some(serde_json::to_value(&resume)?),
                    None,
                )
                .await
                .map_err(|e| anyhow::anyhow!("Failed to persist wait-for-tools state: {}", e))?;
        }
    }

    Ok(())
}

fn turn_output_with_stop_reason(
    mut output: serde_json::Value,
    stop_reason: everruns_core::turn::TurnStopReason,
) -> serde_json::Value {
    if let Some(object) = output.as_object_mut() {
        object.insert("stop_reason".to_string(), serde_json::json!(stop_reason));
    }
    output
}

/// Iteration boundaries at which queued `USER_MESSAGE` steering signals (task
/// wakes) are drained.
///
/// A wake is delivered as a persisted user message plus a durable
/// `USER_MESSAGE` signal (see `SessionTaskWaker`). The message is picked up by
/// the next reason iteration (which re-reads full history); the signal governs
/// whether the loop runs that next iteration. Draining it:
/// - at the `act`→`reason` boundary delivers a wake that arrived **mid-turn**
///   at the very next reason, so an active parent reacts without ending its
///   turn (EVE-681); and
/// - at a final-answer `reason` boundary decides continue-vs-idle for a wake
///   that arrived as the turn wound down.
///
/// Because `consume_pending_signals` is a destructive read, a wake drained at
/// the act boundary is not seen again by the end-of-turn drain — mid-turn XOR
/// next-turn, never both.
fn drains_wake_signals_after(completed_activity: &str, reason_final_answer: bool) -> bool {
    match completed_activity {
        "act" => true,
        "reason" => reason_final_answer,
        _ => false,
    }
}

/// Consume and count queued `USER_MESSAGE` wakes for `workflow_id` when this
/// activity boundary is a drain point (see [`drains_wake_signals_after`]).
/// Returns 0 without touching the store at non-drain boundaries.
async fn count_drained_wakes<S: TaskStore>(
    store: &Arc<S>,
    workflow_id: Uuid,
    completed_activity: &str,
    reason_final_answer: bool,
) -> Result<usize> {
    if !drains_wake_signals_after(completed_activity, reason_final_answer) {
        return Ok(0);
    }
    Ok(store
        .consume_pending_signals_by_type(workflow_id, everruns_durable::signal_types::USER_MESSAGE)
        .await
        .map_err(|error| anyhow::anyhow!("Failed to consume workflow wake signals: {}", error))?
        .len())
}

async fn enqueue_reason_task<S: TaskStore>(
    store: &Arc<S>,
    workflow_id: Uuid,
    input: &DurableTurnInput,
) -> Result<()> {
    let activity_id = format!("reason_{}", Uuid::now_v7());
    let input_json = serde_json::to_value(input)?;
    store
        .enqueue_task_and_record(workflow_id, activity_id, "reason".to_string(), input_json)
        .await
        .map_err(|e| anyhow::anyhow!("Failed to enqueue reason task: {}", e))?;
    Ok(())
}

async fn enqueue_act_task<S: TaskStore>(
    store: &Arc<S>,
    workflow_id: Uuid,
    plan: &RuntimeActPlan,
) -> Result<()> {
    let mut act_input_json = serde_json::to_value(&plan.input)?;
    if let Some(response_id) = &plan.previous_response_id {
        act_input_json["previous_response_id"] = serde_json::json!(response_id);
    }
    act_input_json["iteration"] = serde_json::json!(plan.iteration);
    if let Some(request_id) = &plan.request_id {
        act_input_json["request_id"] = serde_json::json!(request_id);
    }
    act_input_json["resume_state"] = serde_json::to_value(plan.resume_state.as_ref())?;

    let activity_id = format!("act_{}", Uuid::now_v7());
    store
        .enqueue_task_and_record(workflow_id, activity_id, "act".to_string(), act_input_json)
        .await
        .map_err(|e| anyhow::anyhow!("Failed to enqueue act task: {}", e))?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use everruns_core::error::Result as CoreResult;
    use std::collections::HashMap;
    use std::sync::atomic::AtomicBool;

    // ---- EVE-681: mid-turn task wake drain ----

    fn user_message_signal() -> everruns_durable::WorkflowSignal {
        everruns_durable::WorkflowSignal::new(
            everruns_durable::signal_types::USER_MESSAGE,
            serde_json::json!({}),
        )
    }

    /// `TaskStore` stub returning a fixed set of pending signals and counting
    /// how many times wake-signal drains are called.
    #[derive(Clone)]
    struct RecordingStore {
        signals: Vec<everruns_durable::WorkflowSignal>,
        consume_calls: Arc<AtomicUsize>,
    }

    #[async_trait::async_trait]
    impl TaskStore for RecordingStore {
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
            Ok(vec![])
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
        async fn get_workflow_status(
            &self,
            _workflow_id: Uuid,
        ) -> Result<WorkflowStatus, StoreError> {
            Ok(WorkflowStatus::Running)
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
        ) -> Result<TaskFailureOutcome, StoreError> {
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
        ) -> Result<Vec<everruns_durable::WorkflowSignal>, StoreError> {
            Ok(self.signals.clone())
        }

        async fn consume_pending_signals_by_type(
            &self,
            _workflow_id: Uuid,
            signal_type: &str,
        ) -> Result<Vec<everruns_durable::WorkflowSignal>, StoreError> {
            self.consume_calls.fetch_add(1, Ordering::SeqCst);
            Ok(self
                .signals
                .iter()
                .filter(|signal| signal.signal_type == signal_type)
                .cloned()
                .collect())
        }
    }

    #[test]
    fn wake_signals_drain_at_act_and_final_reason_boundaries() {
        // `act` always precedes another reason → the mid-turn delivery point.
        assert!(drains_wake_signals_after("act", false));
        // A final-answer reason drains to decide continue-vs-idle.
        assert!(drains_wake_signals_after("reason", true));
        // A tool-calling reason does not drain; the following `act` will.
        assert!(!drains_wake_signals_after("reason", false));
        // Turn start and unknown activities never drain.
        assert!(!drains_wake_signals_after("process_input", false));
        assert!(!drains_wake_signals_after("input", false));
    }

    #[test]
    fn durable_turn_output_surfaces_structured_stop_reason() {
        let output = turn_output_with_stop_reason(
            serde_json::json!({ "success": true, "error": null }),
            everruns_core::turn::TurnStopReason::MaxTokens,
        );

        assert_eq!(output["success"], true);
        assert_eq!(output["error"], serde_json::Value::Null);
        assert_eq!(output["stop_reason"], "max_tokens");
    }

    #[tokio::test]
    async fn act_boundary_drains_only_user_message_wakes() {
        // Two wakes plus an unrelated signal accrued during the turn.
        let store = Arc::new(RecordingStore {
            signals: vec![
                user_message_signal(),
                everruns_durable::WorkflowSignal::new(
                    everruns_durable::signal_types::CANCEL,
                    serde_json::json!({}),
                ),
                user_message_signal(),
            ],
            consume_calls: Arc::new(AtomicUsize::new(0)),
        });

        let count = count_drained_wakes(&store, Uuid::now_v7(), "act", false)
            .await
            .expect("act boundary drains");

        // Only USER_MESSAGE wakes are consumed; the cancel signal remains pending
        // for the normal signal dispatcher instead of being dropped.
        assert_eq!(count, 2);
        assert_eq!(store.consume_calls.load(Ordering::SeqCst), 1);
    }

    #[tokio::test]
    async fn tool_calling_reason_does_not_drain_wakes() {
        let store = Arc::new(RecordingStore {
            signals: vec![user_message_signal()],
            consume_calls: Arc::new(AtomicUsize::new(0)),
        });

        // A reason that emitted tool calls (`reason_final_answer = false`) must
        // not consume signals — the following `act` boundary owns that drain,
        // so the wake is delivered exactly once.
        let count = count_drained_wakes(&store, Uuid::now_v7(), "reason", false)
            .await
            .expect("non-drain boundary");

        assert_eq!(count, 0);
        assert_eq!(
            store.consume_calls.load(Ordering::SeqCst),
            0,
            "must not touch the signal store at a non-drain boundary"
        );
    }

    #[test]
    fn parse_resume_state_allows_missing_field() {
        let input = serde_json::json!({});

        let parsed = parse_resume_state(&input).expect("missing resume_state is allowed");

        assert!(parsed.is_none());
    }

    #[test]
    fn parse_resume_state_rejects_malformed_state() {
        let input = serde_json::json!({
            "resume_state": {
                "org_id": "not-an-org-id"
            }
        });

        let error = parse_resume_state(&input).expect_err("malformed resume_state must fail");

        assert!(
            error.to_string().contains("Failed to parse resume_state"),
            "unexpected error: {error}"
        );
    }

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
        impl TaskStore for ConcurrentStore {
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
                        options: ActivityOptions::default(),
                        attempt: 1,
                        max_attempts: 1,
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

            async fn get_workflow_status(
                &self,
                _workflow_id: Uuid,
            ) -> Result<WorkflowStatus, StoreError> {
                Ok(WorkflowStatus::Running)
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
            ) -> Result<Vec<everruns_durable::WorkflowSignal>, StoreError> {
                Ok(vec![])
            }

            async fn consume_pending_signals_by_type(
                &self,
                _workflow_id: Uuid,
                _signal_type: &str,
            ) -> Result<Vec<everruns_durable::WorkflowSignal>, StoreError> {
                Ok(vec![])
            }
        }

        #[derive(Clone)]
        struct NoopAdapters;

        #[async_trait::async_trait]
        impl WorkerAdapters for NoopAdapters {
            async fn get_agent(
                &self,
                _org_id: i64,
                _agent_id: Uuid,
            ) -> CoreResult<Option<everruns_platform::Agent>> {
                unimplemented!()
            }
            async fn get_harness(
                &self,
                _org_id: i64,
                _harness_id: Uuid,
            ) -> CoreResult<Option<everruns_platform::Harness>> {
                unimplemented!()
            }
            async fn get_session(
                &self,
                _org_id: i64,
                _session_id: Uuid,
            ) -> CoreResult<Option<everruns_core::ExecutionSession>> {
                unimplemented!()
            }
            async fn set_session_status(
                &self,
                _org_id: i64,
                _session_id: Uuid,
                _status: &str,
            ) -> CoreResult<()> {
                unimplemented!()
            }
            async fn set_session_title(
                &self,
                _org_id: i64,
                _session_id: Uuid,
                _title: String,
            ) -> CoreResult<everruns_core::ExecutionSession> {
                unimplemented!()
            }
            async fn get_message(
                &self,
                _session_id: Uuid,
                _message_id: Uuid,
            ) -> CoreResult<Option<everruns_core::Message>> {
                unimplemented!()
            }
            async fn load_messages(
                &self,
                _session_id: Uuid,
            ) -> CoreResult<Vec<everruns_core::Message>> {
                unimplemented!()
            }
            async fn emit_event(
                &self,
                _request: everruns_core::events::EventRequest,
            ) -> CoreResult<everruns_core::events::Event> {
                unimplemented!()
            }
            async fn get_resolved_model(
                &self,
                _org_id: i64,
                _model_id: Uuid,
            ) -> CoreResult<Option<everruns_core::traits::ResolvedModel>> {
                unimplemented!()
            }
            async fn get_default_model(
                &self,
                _org_id: i64,
            ) -> CoreResult<Option<everruns_core::traits::ResolvedModel>> {
                unimplemented!()
            }
            async fn get_provider_config(
                &self,
                _org_id: i64,
                _provider: &everruns_core::ProviderKey,
            ) -> CoreResult<Option<everruns_core::ProviderConfig>> {
                unimplemented!()
            }
            async fn resolve_image(
                &self,
                _org_id: i64,
                _image_id: Uuid,
            ) -> CoreResult<Option<everruns_core::traits::ResolvedImage>> {
                unimplemented!()
            }
            async fn resolve_images_batch(
                &self,
                _org_id: i64,
                _image_ids: &[Uuid],
            ) -> CoreResult<HashMap<Uuid, everruns_core::traits::ResolvedImage>> {
                unimplemented!()
            }
            async fn read_file(
                &self,
                _session_id: Uuid,
                _path: &str,
            ) -> CoreResult<Option<everruns_core::session_file::SessionFile>> {
                unimplemented!()
            }
            async fn write_file(
                &self,
                _session_id: Uuid,
                _path: &str,
                _content: &str,
                _encoding: &str,
            ) -> CoreResult<everruns_core::session_file::SessionFile> {
                unimplemented!()
            }
            async fn delete_file(
                &self,
                _session_id: Uuid,
                _path: &str,
                _recursive: bool,
            ) -> CoreResult<bool> {
                unimplemented!()
            }
            async fn list_directory(
                &self,
                _session_id: Uuid,
                _path: &str,
            ) -> CoreResult<Vec<everruns_core::session_file::FileInfo>> {
                unimplemented!()
            }
            async fn stat_file(
                &self,
                _session_id: Uuid,
                _path: &str,
            ) -> CoreResult<Option<everruns_core::session_file::FileStat>> {
                unimplemented!()
            }
            async fn grep_files(
                &self,
                _session_id: Uuid,
                _pattern: &str,
                _path_pattern: Option<&str>,
            ) -> CoreResult<Vec<everruns_core::session_file::GrepMatch>> {
                unimplemented!()
            }
            async fn create_directory(
                &self,
                _session_id: Uuid,
                _path: &str,
            ) -> CoreResult<everruns_core::session_file::FileInfo> {
                unimplemented!()
            }
            async fn get_mcp_server_by_prefix(
                &self,
                _org_id: i64,
                _session_id: Option<Uuid>,
                _server_prefix: &str,
            ) -> CoreResult<crate::mcp_executor::McpServerInfo> {
                unimplemented!()
            }
            async fn load_turn_context(
                &self,
                _org_id: i64,
                _session_id: Uuid,
            ) -> CoreResult<crate::worker_adapters::TurnContext> {
                unimplemented!()
            }
            async fn invoke_scheduled_app_channel(
                &self,
                _org_id: i64,
                _app_id: &str,
                _channel_id: &str,
            ) -> CoreResult<serde_json::Value> {
                unimplemented!()
            }
            async fn invoke_agent_trigger(
                &self,
                _org_id: i64,
                _agent_id: &str,
                _trigger_id: &str,
            ) -> CoreResult<serde_json::Value> {
                unimplemented!()
            }
            async fn claim_due_leased_resources(
                &self,
                _limit: u32,
                _stale_after_seconds: u32,
            ) -> CoreResult<Vec<everruns_core::leased_resource::LeasedResource>> {
                unimplemented!()
            }
            async fn mark_leased_resource_released(
                &self,
                _resource_id: everruns_core::typed_id::LeasedResourceId,
                _expected_cleanup_started_at: chrono::DateTime<chrono::Utc>,
            ) -> CoreResult<bool> {
                unimplemented!()
            }
            async fn mark_leased_resource_cleanup_failed(
                &self,
                _resource_id: everruns_core::typed_id::LeasedResourceId,
                _expected_cleanup_started_at: chrono::DateTime<chrono::Utc>,
                _retry_after_seconds: u32,
                _error: &str,
            ) -> CoreResult<bool> {
                unimplemented!()
            }
            async fn list_orphaned_session_task_ids(
                &self,
                _stale_after: chrono::Duration,
                _limit: i64,
            ) -> CoreResult<Vec<(everruns_core::SessionId, String)>> {
                unimplemented!()
            }
            async fn prune_terminal_session_tasks(
                &self,
                _ttl: chrono::Duration,
                _limit: i64,
            ) -> CoreResult<usize> {
                unimplemented!()
            }

            fn capability_registry(&self) -> everruns_core::capabilities::CapabilityRegistry {
                unimplemented!()
            }
            fn driver_registry(&self) -> everruns_core::DriverRegistry {
                unimplemented!()
            }
            fn sqldb_store(&self) -> everruns_core::traits::SessionSqlDbStoreRef {
                unimplemented!()
            }
            fn storage_store(&self) -> Arc<dyn everruns_core::traits::SessionStorageStore> {
                unimplemented!()
            }
            fn image_artifact_store(
                &self,
                _org_id: i64,
            ) -> Arc<dyn everruns_core::traits::ImageArtifactStore> {
                unimplemented!()
            }
            fn provider_credential_store(
                &self,
                _org_id: i64,
            ) -> Arc<dyn everruns_core::traits::ProviderCredentialStore> {
                unimplemented!()
            }
            fn utility_llm_service(&self) -> Option<Arc<dyn everruns_core::UtilityLlmService>> {
                unimplemented!()
            }
            fn egress_service(&self) -> Option<Arc<dyn everruns_core::EgressService>> {
                unimplemented!()
            }
            fn platform_store(
                &self,
                _org_id: i64,
                _session_id: everruns_core::SessionId,
            ) -> Arc<dyn everruns_platform::PlatformStore> {
                unimplemented!()
            }
            fn connection_resolver(
                &self,
            ) -> Arc<dyn everruns_core::traits::UserConnectionResolver> {
                unimplemented!()
            }
            fn leased_resource_store(&self) -> Arc<dyn everruns_core::traits::LeasedResourceStore> {
                unimplemented!()
            }
            fn schedule_store(
                &self,
                _org_id: i64,
            ) -> Arc<dyn everruns_core::traits::SessionScheduleStore> {
                unimplemented!()
            }
            fn reaper_session_task_registry(
                &self,
            ) -> Arc<dyn everruns_core::session_task::SessionTaskRegistry> {
                unimplemented!()
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
