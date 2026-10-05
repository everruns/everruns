// Task Worker Implementation
//
// Decision: Single worker implementation generic over WorkerAdapters
// Decision: Works with both gRPC (external) and Direct (in-process) adapters
// Decision: Replaces both InProcessWorker and DurableWorker
//
// TaskWorker executes activities (input, reason, act) from the durable task queue.
// It unifies the two worker implementations into one, eliminating code duplication
// while preserving the different deployment models (in-process vs external).

use crate::durable::{ClaimedTask, TaskFailureOutcome, WorkerInfo, WorkflowStatus};
use crate::engine::{ActInput, ActPlan, TurnPlan};
use crate::host::{
    RuntimeSessionLifecycle, advance_host_execution,
    execute_act_activity as runtime_execute_act_activity,
};
use anyhow::Result;
use std::sync::Arc;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::time::Duration;
use tokio::sync::{Notify, watch};
use tokio::task::JoinSet;
use tracing::{debug, error, info, warn};
use uuid::Uuid;

use crate::durable_runner::DurableTurnInput;
use crate::runtime_host::WorkerRuntimeHost;
use crate::task_error::{is_non_retryable_task_error, summarize_task_failure, user_facing_failure};
use crate::task_heartbeat::spawn_task_heartbeat;
use crate::task_wakeup::spawn_wakeup_listener;
use crate::worker_adapters::WorkerAdapters;
use crate::{
    activities::ScheduledAgentTriggerInput, activities::ScheduledChannelInput,
    activities::activity_types, phase_reads::PhaseIds, turn_start,
};

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
            ..defaults
        }
    }
}

// =============================================================================
// Unified Worker
// =============================================================================

pub use everruns_durable_engine::task_store::TaskStore;

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
    /// Cuts the poll backoff short when new work may be claimable.
    wake: Arc<Notify>,
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
            let store = self.store.clone();
            let adapters = self.adapters.clone();
            let worker_id = self.config.worker_id.clone();
            let in_flight_guard = InFlightTaskGuard::increment(self.in_flight.clone());
            let heartbeat_interval = self.config.heartbeat_interval;
            let wake = self.wake.clone();

            task_handles.spawn(async move {
                let _in_flight_guard = in_flight_guard;
                let result =
                    execute_task(&store, &adapters, &worker_id, heartbeat_interval, &task).await;
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
            let _ = store
                .fail_task_and_record(task, "Workflow cancelled", false)
                .await;
            return Ok(());
        }
    }

    store.record_activity_started(task, worker_id).await;

    let (heartbeat_cancel_tx, heartbeat_handle, task_cancellation) = spawn_task_heartbeat(
        store.clone(),
        task.id,
        worker_id.to_string(),
        heartbeat_interval,
    );

    // Execute based on activity type. Keep fallible parsing inside this result so
    // cleanup below runs before malformed tasks are failed.
    let execution = async {
        // A `process_input` task runs the turn's first reason too, and is then
        // completed and scheduled as that `reason` (see `turn_start`).
        let mut activity = task.activity_type.as_str();
        let (result, turn_input_opt) = match activity {
            "process_input" | "reason" => {
                let turn_input: DurableTurnInput = serde_json::from_value(task.input.clone())
                    .map_err(|e| anyhow::anyhow!("Failed to parse task input: {}", e))?;
                let cancel = task_cancellation.clone();
                if activity == "process_input" {
                    activity = "reason";
                    let (res, checkpoint) =
                        turn_start::execute_turn_start(adapters, &turn_input, task.id, cancel)
                            .await;
                    (res, Some(checkpoint))
                } else {
                    let res = turn_start::execute_reason_activity(adapters, &turn_input, cancel);
                    (res.await, Some(turn_input))
                }
            }
            "act" => {
                let act_input: ActInput = serde_json::from_value(task.input.clone())
                    .map_err(|e| anyhow::anyhow!("Failed to parse ActInput: {}", e))?;

                let resume_state = parse_resume_state(&task.input)?;

                // Create DurableTurnInput from ActInput context
                let turn_input = resume_state.unwrap_or(DurableTurnInput {
                    org_id: act_input.org_id.ok_or_else(|| {
                        anyhow::anyhow!("ActInput.org_id must be set for durable turns")
                    })?,
                    session_id: act_input.context.session_id,
                    harness_id: act_input.harness_id,
                    agent_id: act_input.agent_id,
                    input_message_id: act_input.context.input_message_id,
                    turn_id: Some(act_input.context.turn_id),
                    previous_response_id: None,
                    iteration: 1,
                    request_id: None,
                    started_at: None,
                    cumulative_usage: None,
                    tool_call_count: 0,
                    llm_call_count: 0,
                    time_to_first_token_ms: None,
                    final_message_id: None,
                    final_answer_preview: None,
                });
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
            activity_types::INVOKE_SCHEDULED_CHANNEL => {
                let input: ScheduledChannelInput = serde_json::from_value(task.input.clone())
                    .map_err(|e| anyhow::anyhow!("Failed to parse scheduled app input: {}", e))?;
                let res = adapters
                    .invoke_scheduled_channel(input.org_id, &input.app_id, &input.channel_id)
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
        Ok::<_, anyhow::Error>((result, turn_input_opt, activity))
    }
    .await;

    let _ = heartbeat_cancel_tx.send(());
    let _ = heartbeat_handle.await;

    let (result, turn_input_opt, activity) = match execution {
        Ok(execution) => execution,
        Err(e) => {
            fail_activity_task(store, adapters, task, None, &e).await?;

            return Err(e);
        }
    };

    match result {
        Ok(output) => {
            // Complete the task (verifying ownership), draining wake signals
            // in the same call when this boundary is a drain point.
            let schedules = turn_input_opt.is_some() && task.workflow_id.is_some();
            let final_answer = reason_final_answer(activity, &output).unwrap_or(false);
            let drain = (schedules && drains_wake_signals_after(activity, final_answer))
                .then_some(crate::durable_turn::USER_MESSAGE);
            let complete_result = store
                .complete_task_and_drain(task, worker_id, output.clone(), drain)
                .await;

            match complete_result {
                Ok(drained) => {
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
                            activity,
                            &turn_input,
                            &output,
                            drained,
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
            fail_activity_task(store, adapters, task, turn_input_opt.as_ref(), &e).await?;

            return Err(e);
        }
    }

    Ok(())
}

async fn fail_activity_task<S: TaskStore, A: WorkerAdapters + Clone>(
    store: &Arc<S>,
    adapters: &A,
    task: &ClaimedTask,
    turn_input: Option<&DurableTurnInput>,
    error: &anyhow::Error,
) -> Result<TaskFailureOutcome> {
    let failure = summarize_task_failure(
        task.id,
        task.workflow_id,
        &task.activity_type,
        task.attempt,
        Some(task.max_attempts),
        &task.input,
        error,
    );
    let retryable = !is_non_retryable_task_error(error);
    let outcome = store
        .fail_task_and_record(task, &failure.persisted_message, retryable)
        .await
        .map_err(|store_error| anyhow::anyhow!("Failed to persist task failure: {store_error}"))?;

    if matches!(outcome, TaskFailureOutcome::ExhaustedRetries) {
        terminalize_failed_turn(adapters, task, turn_input, &failure.persisted_message).await?;
    }

    Ok(outcome)
}

async fn terminalize_failed_turn<A: WorkerAdapters + Clone>(
    adapters: &A,
    task: &ClaimedTask,
    turn_input: Option<&DurableTurnInput>,
    persisted_error: &str,
) -> Result<()> {
    let parsed_input = if turn_input.is_none() {
        serde_json::from_value::<DurableTurnInput>(task.input.clone()).ok()
    } else {
        None
    };
    let Some(input) = turn_input.or(parsed_input.as_ref()) else {
        return Ok(());
    };
    let Some(turn_id) = input.turn_id else {
        warn!(
            task_id = %task.id,
            workflow_id = ?task.workflow_id,
            "Cannot emit terminal turn events without a turn_id"
        );
        return Ok(());
    };

    let user_error = user_facing_failure(persisted_error);
    let message = user_error.fallback_message();
    let lifecycle = RuntimeSessionLifecycle::new(
        WorkerRuntimeHost::new(adapters.clone()),
        input.org_id,
        input.session_id,
    );
    lifecycle
        .turn_failed(turn_id, input.input_message_id, &message, Some(&user_error))
        .await
        .map_err(anyhow::Error::from)?;
    lifecycle
        .fire_turn_end_hooks(input.harness_id, input.agent_id, turn_id, false)
        .await;
    Ok(())
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

    let host = WorkerRuntimeHost::new(adapters.clone()).prefetching(PhaseIds::act(input));
    let result = runtime_execute_act_activity(&host, input.clone()).await;
    host.flush_events().await;
    let result = result?;

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
    drained: Option<usize>,
) -> Result<()> {
    let reason_final_answer = reason_final_answer(completed_activity, output)?;

    // Drain queued USER_MESSAGE steering signals (task wakes) at the boundaries
    // that precede another reason iteration. The already-persisted wake message
    // is picked up by that reason (it re-reads full history); consuming the
    // signal here is what governs turn continuation and, being destructive,
    // gives exactly-once delivery — see `drains_wake_signals_after`.
    let pending_user_message_count = match drained {
        Some(count) => count,
        None => {
            count_drained_wakes(store, workflow_id, completed_activity, reason_final_answer).await?
        }
    };

    if completed_activity == "act" && pending_user_message_count > 0 {
        debug!(
            %workflow_id,
            pending_user_message_count,
            "delivering mid-turn task wake(s) at the act→reason boundary"
        );
    }

    let mut execution = crate::DurableExecution::new(input.clone());
    let plan = advance_host_execution(
        &WorkerRuntimeHost::new(adapters.clone()),
        &mut execution,
        completed_activity,
        output,
        pending_user_message_count,
    )
    .await?;
    let checkpoint = execution.checkpoint();
    match plan {
        TurnPlan::ScheduleReason(_) => {
            enqueue_reason_task(store, workflow_id, &checkpoint).await?;
        }
        TurnPlan::ScheduleAct(plan) => {
            enqueue_act_task(store, workflow_id, &plan, &checkpoint).await?;
        }
        TurnPlan::Complete { stop_reason, error } => {
            let turn_output = turn_output_with_stop_reason(output.clone(), stop_reason);
            store
                .complete_workflow(
                    workflow_id,
                    turn_output,
                    Some(serde_json::to_value(&checkpoint)?),
                    error.map(crate::durable::WorkflowError::new),
                )
                .await
                .map_err(|e| anyhow::anyhow!("Failed to update workflow status: {}", e))?;
        }
        TurnPlan::WaitForToolResults { .. } => {
            store
                .complete_workflow(
                    workflow_id,
                    output.clone(),
                    Some(serde_json::to_value(&checkpoint)?),
                    None,
                )
                .await
                .map_err(|e| anyhow::anyhow!("Failed to persist wait-for-tools state: {}", e))?;
        }
    }

    Ok(())
}

/// Whether a completed `reason` produced a final answer (no tool calls, no
/// pause), winding the turn down. Always false for other activities.
fn reason_final_answer(completed_activity: &str, output: &serde_json::Value) -> Result<bool> {
    if completed_activity != "reason" {
        return Ok(false);
    }
    let reason_result: ReasonResult = serde_json::from_value(output.clone())
        .map_err(|error| anyhow::anyhow!("Invalid reason output payload: {}", error))?;
    let continues = reason_result.has_tool_calls || reason_result.waiting_for_tool_results;
    Ok(reason_result.success && !continues)
}

fn turn_output_with_stop_reason(
    mut output: serde_json::Value,
    stop_reason: crate::core::turn::TurnStopReason,
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
        .consume_pending_signals_by_type(workflow_id, crate::durable_turn::USER_MESSAGE)
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
    plan: &ActPlan,
    checkpoint: &DurableTurnInput,
) -> Result<()> {
    let act_input_json = act_task_input(plan, checkpoint)?;

    let activity_id = format!("act_{}", Uuid::now_v7());
    store
        .enqueue_task_and_record(workflow_id, activity_id, "act".to_string(), act_input_json)
        .await
        .map_err(|e| anyhow::anyhow!("Failed to enqueue act task: {}", e))?;
    Ok(())
}

fn act_task_input(plan: &ActPlan, checkpoint: &DurableTurnInput) -> Result<serde_json::Value> {
    let mut input = serde_json::to_value(&plan.input)?;
    input["resume_state"] = serde_json::to_value(checkpoint)?;
    Ok(input)
}

#[cfg(test)]
mod tests {
    use crate::unified_worker_test_adapters::NoopAdapters;

    use super::*;
    use crate::durable::{
        ActivityOptions, DurableAdmin, EventLog, HeartbeatResponse, StoreError, TaskDefinition,
        TaskQueue, WorkerRegistry, WorkflowError,
    };
    use everruns_contracts::typed_id::TurnId;
    use std::sync::atomic::AtomicBool;

    // ---- EVE-681: mid-turn task wake drain ----

    fn user_message_signal() -> crate::durable::WorkflowSignal {
        crate::durable::WorkflowSignal::new(
            crate::durable_turn::USER_MESSAGE,
            serde_json::json!({}),
        )
    }

    /// `TaskStore` stub returning a fixed set of pending signals and counting
    /// how many times wake-signal drains are called.
    #[derive(Clone)]
    struct RecordingStore {
        signals: Vec<crate::durable::WorkflowSignal>,
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
            _retryable: bool,
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
        ) -> Result<Vec<crate::durable::WorkflowSignal>, StoreError> {
            Ok(self.signals.clone())
        }

        async fn consume_pending_signals_by_type(
            &self,
            _workflow_id: Uuid,
            signal_type: &str,
        ) -> Result<Vec<crate::durable::WorkflowSignal>, StoreError> {
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

    #[tokio::test]
    async fn completion_drains_only_where_the_store_folds_it_in() {
        // `execute_task` asks the completion to drain at a final-answer reason
        // and after act; a store that cannot fold the drain in returns None so
        // `schedule_next_activity` consumes the wakes itself.
        let reason = |has_tool_calls| {
            serde_json::to_value(ReasonResult {
                success: true,
                has_tool_calls,
                ..ReasonResult::default()
            })
            .unwrap()
        };
        assert!(reason_final_answer("reason", &reason(false)).unwrap());
        assert!(!reason_final_answer("reason", &reason(true)).unwrap());
        assert!(!reason_final_answer("act", &serde_json::json!({})).unwrap());
        assert!(reason_final_answer("reason", &serde_json::json!({})).is_err());

        let store = RecordingStore {
            signals: vec![user_message_signal()],
            consume_calls: Arc::new(AtomicUsize::new(0)),
        };
        let task = ClaimedTask {
            id: Uuid::now_v7(),
            workflow_id: Some(Uuid::now_v7()),
            activity_id: "act-1".into(),
            activity_type: "act".into(),
            input: serde_json::json!({}),
            options: ActivityOptions::default(),
            attempt: 1,
            max_attempts: 1,
        };
        let drained = store
            .complete_task_and_drain(&task, "w", serde_json::json!({}), Some("user_message"))
            .await
            .unwrap();
        assert_eq!(drained, None);
        assert_eq!(store.consume_calls.load(Ordering::SeqCst), 0);
    }

    #[test]
    fn durable_turn_output_surfaces_structured_stop_reason() {
        let output = turn_output_with_stop_reason(
            serde_json::json!({ "success": true, "error": null }),
            crate::core::turn::TurnStopReason::MaxTokens,
        );

        assert_eq!(output["success"], true);
        assert_eq!(output["error"], serde_json::Value::Null);
        assert_eq!(output["stop_reason"], "max_tokens");
    }

    #[test]
    fn act_wire_input_uses_the_engine_checkpoint_as_its_only_resume_state() {
        use crate::engine::{ActSchedulingFacts, TurnState, plan_after_reason};
        use everruns_contracts::typed_id::{HarnessId, MessageId, SessionId};

        let state = TurnState {
            org_id: 7,
            session_id: SessionId::new(),
            harness_id: HarnessId::new(),
            agent_id: None,
            input_message_id: MessageId::new(),
            turn_id: Some(TurnId::new()),
            previous_response_id: Some("before".into()),
            iteration: 2,
            request_id: Some("request-before".into()),
            started_at: None,
            cumulative_usage: None,
            tool_call_count: 0,
            llm_call_count: 0,
            time_to_first_token_ms: None,
            final_message_id: None,
            final_answer_preview: None,
        };
        let reason = ReasonResult {
            native_counts: None,
            success: true,
            text: String::new(),
            tool_calls: vec![everruns_contracts::tool_types::ToolCall {
                id: "call-1".into(),
                name: "noop".into(),
                arguments: serde_json::json!({}),
            }],
            has_tool_calls: true,
            tool_definitions: vec![],
            max_iterations: 8,
            error: None,
            user_facing_error: None,
            error_disclosure: None,
            usage: None,
            output_message_id: None,
            time_to_first_token_ms: None,
            response_id: Some("response-after".into()),
            finish_reason: Some("tool_calls".into()),
            ..ReasonResult::default()
        };
        let (TurnPlan::ScheduleAct(plan), _) = plan_after_reason(
            &state,
            reason,
            0,
            chrono::Utc::now(),
            Some(ActSchedulingFacts::default()),
        ) else {
            panic!("reason with a tool call must schedule act");
        };
        let mut checkpoint = state;
        checkpoint.previous_response_id = Some("checkpoint-response".into());
        checkpoint.iteration = 9;
        checkpoint.request_id = Some("checkpoint-request".into());

        let input = act_task_input(&plan, &checkpoint).expect("serialize act input");

        assert_eq!(input["resume_state"]["iteration"], 9);
        assert_eq!(
            input["resume_state"]["previous_response_id"],
            "checkpoint-response"
        );
        assert_eq!(input["resume_state"]["request_id"], "checkpoint-request");
        assert!(input.get("iteration").is_none());
        assert!(input.get("previous_response_id").is_none());
        assert!(input.get("request_id").is_none());
    }

    #[tokio::test]
    async fn act_boundary_drains_only_user_message_wakes() {
        // Two wakes plus an unrelated signal accrued during the turn.
        let store = Arc::new(RecordingStore {
            signals: vec![
                user_message_signal(),
                crate::durable::WorkflowSignal::new(
                    crate::durable::signal_types::CANCEL,
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

    #[tokio::test]
    async fn direct_store_elects_one_terminal_failure_owner() {
        let store = crate::durable::InMemoryWorkflowEventStore::new();
        let workflow_id = Uuid::now_v7();
        store
            .create_workflow(workflow_id, "turn", serde_json::json!({}), None)
            .await
            .unwrap();
        EventLog::update_workflow_status(&store, workflow_id, WorkflowStatus::Running, None, None)
            .await
            .unwrap();
        store
            .enqueue_task(TaskDefinition {
                workflow_id: Some(workflow_id),
                activity_id: "reason".to_string(),
                activity_type: "reason".to_string(),
                input: serde_json::json!({}),
                options: ActivityOptions::default(),
            })
            .await
            .unwrap();
        let worker = crate::durable::WorkerInfo::new("worker", ["reason"]);
        let registered = WorkerRegistry::register_worker(&store, worker).await;
        registered.unwrap();
        let claimed = TaskQueue::claim_task(&store, "worker", &["reason".into()], 1).await;
        let task = claimed.unwrap().pop().unwrap();

        let outcome = TaskStore::fail_task_and_record(&store, &task, "terminal", false)
            .await
            .unwrap();

        assert!(matches!(outcome, TaskFailureOutcome::ExhaustedRetries));
        assert_eq!(
            EventLog::get_workflow_status(&store, workflow_id)
                .await
                .unwrap(),
            WorkflowStatus::Failed
        );
        let events = store.get_workflow_events(workflow_id).await.unwrap();
        assert_eq!(
            events
                .iter()
                .filter(|event| event.event_type == "workflow_failed")
                .count(),
            1
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
