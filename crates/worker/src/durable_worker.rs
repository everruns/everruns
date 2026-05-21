// Durable execution engine worker
// Decision: Push-based task notifications via gRPC streaming with polling fallback
// Decision: Uses gRPC adapters for control-plane communication (no direct DB access)

use anyhow::Result;
use everruns_core::atoms::AtomContext;
use everruns_core::events::{
    EventContext, EventRequest, OutputMessageCompletedData, SessionIdledData,
};
use everruns_core::traits::EventEmitter;
use everruns_core::typed_id::{ExecId, MessageId, SessionId, TurnId};
use everruns_core::{Message, PlatformDefinition};
use everruns_runtime::{RuntimeActPlan, RuntimeTurnPlan, plan_next_host_turn};
use std::sync::Arc;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::time::Duration;
use tokio::sync::{Mutex, watch};
use tracing::{debug, error, info, warn};
use uuid::Uuid;

use crate::activities::{
    ActInput, InputAtomInput, ReasonInput, ScheduledAppChannelInput, act_activity, input_activity,
    reason_activity,
};
use crate::durable_runner::DurableTurnInput;
use crate::grpc_adapters::{GrpcClient, GrpcEventEmitter, load_turn_context};
use crate::grpc_durable_store::{
    ClaimedTask, GrpcDurableStore, TaskNotificationEvent, TaskNotificationStream, WorkflowStatus,
};
use crate::runtime_host::WorkerRuntimeHost;
use crate::task_error::{summarize_task_failure, user_facing_failure};
use crate::worker_adapters::WorkerAdapters;
use serde::{Deserialize, Serialize};

// =============================================================================
// Act Task Input Wrapper
// =============================================================================

/// Wrapper for act task input.
///
/// `org_id` is carried on the flattened `ActInput` itself. Duplicating it as
/// an explicit outer field collides with the flattened key during
/// deserialization and drops the inner value (EVE-325), so the wrapper only
/// adds fields not already present on `ActInput`.
#[derive(Debug, Clone, Serialize, Deserialize)]
struct ActTaskInput {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    request_id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    resume_state: Option<DurableTurnInput>,
    #[serde(flatten)]
    act_input: ActInput,
}

impl ActTaskInput {
    /// Returns the org_id carried by the inner `ActInput`, erroring if it is
    /// missing. Used by paths that require a present org_id to execute (the
    /// main `execute_task` act branch); some failure-emission paths
    /// intentionally tolerate a missing value and fall back to 0 so they can
    /// still surface an error event to the session.
    fn require_org_id(&self) -> Result<i64> {
        self.act_input
            .org_id
            .ok_or_else(|| anyhow::anyhow!(MISSING_ACT_ORG_ID_ERROR))
    }
}

/// Error substring marking a missing `org_id` on an act task. Persisted into
/// the task failure message so `is_non_retryable_task_error` can short-circuit
/// the retry loop and DLQ the task immediately instead of burning attempts on
/// a deterministic configuration failure.
const MISSING_ACT_ORG_ID_ERROR: &str = "ActTaskInput.act_input.org_id must be set";

// =============================================================================
// Task Session Context Extraction
// =============================================================================

/// Session context extracted from a task input.
///
/// Used by failure handling such as DLQ emission and other task-input
/// error paths that need to surface user-facing events regardless of which
/// phase (input/reason/act) failed.
#[derive(Debug)]
struct TaskSessionContext {
    org_id: i64,
    session_id: SessionId,
    turn_id: TurnId,
    input_message_id: MessageId,
}

/// Extract session context from a task input by activity type.
///
/// Different activities serialize different wrapper types into `task.input`
/// (`DurableTurnInput` for input/reason, `ActTaskInput` for act). Without
/// handling both shapes, act-task failure paths silently skip emitting
/// `session.idled` and leave the session stuck.
///
/// Returns `Err` with the underlying parse failure (or "unknown activity
/// type") so callers can log enough context to diagnose unexpected
/// `task.input` shapes in production.
fn extract_task_session_context(
    activity_type: &str,
    input: &serde_json::Value,
) -> Result<TaskSessionContext> {
    use anyhow::Context;
    match activity_type {
        "process_input" | "reason" => {
            let turn_input: DurableTurnInput =
                serde_json::from_value(input.clone()).with_context(|| {
                    format!("parse DurableTurnInput for activity '{activity_type}'")
                })?;
            Ok(TaskSessionContext {
                org_id: turn_input.org_id,
                session_id: turn_input.session_id,
                turn_id: turn_input.turn_id.unwrap_or_default(),
                input_message_id: turn_input.input_message_id,
            })
        }
        "act" => {
            let act_task_input: ActTaskInput = serde_json::from_value(input.clone())
                .with_context(|| format!("parse ActTaskInput for activity '{activity_type}'"))?;
            Ok(TaskSessionContext {
                org_id: act_task_input.act_input.org_id.unwrap_or(0),
                session_id: act_task_input.act_input.context.session_id,
                turn_id: act_task_input.act_input.context.turn_id,
                input_message_id: act_task_input.act_input.context.input_message_id,
            })
        }
        other => Err(anyhow::anyhow!(
            "unknown activity type '{other}' has no recognised task input shape"
        )),
    }
}

// =============================================================================
// Cancellation Helper
// =============================================================================

/// Emit cancellation events when workflow is cancelled
/// This emits the agent message and session.idled event
async fn emit_cancellation_events(
    grpc_address: &str,
    org_id: i64,
    session_id: SessionId,
    turn_id: TurnId,
    input_message_id: MessageId,
) {
    // Connect to gRPC for event emission
    let grpc_client = match GrpcClient::connect(grpc_address).await {
        Ok(client) => client,
        Err(e) => {
            warn!(error = %e, "Failed to connect to gRPC for cancellation events");
            return;
        }
    };

    let event_emitter = GrpcEventEmitter::new(grpc_client.clone());

    // Emit agent message indicating cancellation completed
    let cancel_message = Message::assistant("Work was cancelled by user.");
    let message_event = EventRequest::new(
        session_id,
        EventContext::turn(turn_id, input_message_id),
        OutputMessageCompletedData::new(cancel_message),
    );
    if let Err(e) = event_emitter.emit(message_event).await {
        warn!(session_id = %session_id, error = %e, "Failed to emit cancellation message");
    }

    // Emit session.idled event
    let idled_event = EventRequest::new(
        session_id,
        EventContext::turn(turn_id, input_message_id),
        SessionIdledData {
            turn_id,
            iterations: None,
            usage: None,
        },
    );
    if let Err(e) = event_emitter.emit(idled_event).await {
        warn!(session_id = %session_id, error = %e, "Failed to emit session.idled event");
    }

    // Set session status to idle
    if let Err(e) = grpc_client
        .set_session_status(org_id, session_id, "idle")
        .await
    {
        warn!(session_id = %session_id, error = %e, "Failed to set session status to idle");
    }

    info!(session_id = %session_id, "Cancellation events emitted");
}

// =============================================================================
// Configuration
// =============================================================================

/// Configuration for the durable worker
#[derive(Debug, Clone)]
pub struct DurableWorkerConfig {
    /// Worker ID (unique identifier for this worker instance)
    pub worker_id: String,
    /// Activity types this worker handles
    pub activity_types: Vec<String>,
    /// Maximum concurrent tasks
    pub max_concurrent_tasks: usize,
    /// Poll interval when push notifications unavailable (fallback)
    pub poll_interval: Duration,
    /// Heartbeat interval for claimed tasks
    pub heartbeat_interval: Duration,
    /// gRPC address for control-plane communication
    pub grpc_address: String,
    /// Timeout for initial connection to control-plane gRPC
    pub connect_timeout: Duration,
}

impl Default for DurableWorkerConfig {
    fn default() -> Self {
        Self {
            worker_id: format!("worker-{}", Uuid::now_v7()),
            activity_types: vec![
                "process_input".to_string(),
                "reason".to_string(),
                "act".to_string(),
                "leased_resource_cleanup".to_string(),
                "invoke_scheduled_app_channel".to_string(),
            ],
            max_concurrent_tasks: 1000, // High default for massive workflow parallelism
            poll_interval: Duration::from_millis(10), // Fallback when push notifications unavailable
            heartbeat_interval: Duration::from_secs(10),
            grpc_address: "127.0.0.1:9001".to_string(),
            connect_timeout: Duration::from_secs(30),
        }
    }
}

impl DurableWorkerConfig {
    /// Create configuration from environment variables
    pub fn from_env() -> Self {
        use everruns_config::{env_duration_secs, env_or, env_string_any};

        let defaults = Self::default();
        Self {
            worker_id: std::env::var("WORKER_ID")
                .unwrap_or_else(|_| format!("worker-{}", Uuid::now_v7())),
            grpc_address: env_string_any(
                &["SERVER_GRPC_ADDRESS", "WORKER_GRPC_ADDRESS"],
                "127.0.0.1:9001",
            ),
            max_concurrent_tasks: env_or("MAX_CONCURRENT_TASKS", 1000),
            connect_timeout: env_duration_secs(
                "WORKER_GRPC_CONNECT_TIMEOUT",
                defaults.connect_timeout,
            ),
            ..defaults
        }
    }
}

// =============================================================================
// DurableWorker
// =============================================================================

/// Worker that executes tasks from the durable task queue via gRPC.
/// Uses push-based notifications for low-latency task pickup (<10ms P99),
/// with polling fallback when notifications are unavailable.
pub struct DurableWorker {
    config: DurableWorkerConfig,
    store: Arc<Mutex<GrpcDurableStore>>,
    grpc_address: String,
    platform_definition: Arc<PlatformDefinition>,
    shutdown_tx: watch::Sender<bool>,
    shutdown_rx: watch::Receiver<bool>,
    /// Count of tasks currently being executed
    in_flight: Arc<AtomicUsize>,
}

impl DurableWorker {
    /// Create a new durable worker
    pub async fn new(config: DurableWorkerConfig) -> Result<Self> {
        Self::with_platform_definition(config, crate::platform::default_platform_definition()).await
    }

    /// Create a new durable worker with an explicit platform definition.
    pub async fn with_platform_definition(
        config: DurableWorkerConfig,
        platform_definition: PlatformDefinition,
    ) -> Result<Self> {
        info!(
            worker_id = %config.worker_id,
            grpc_address = %config.grpc_address,
            max_concurrent = config.max_concurrent_tasks,
            "Initializing durable worker (gRPC mode)"
        );

        let store =
            GrpcDurableStore::connect_with_timeout(&config.grpc_address, config.connect_timeout)
                .await?;
        let grpc_address = config.grpc_address.clone();

        let (shutdown_tx, shutdown_rx) = watch::channel(false);

        info!("Durable worker initialized");

        Ok(Self {
            config,
            store: Arc::new(Mutex::new(store)),
            grpc_address,
            platform_definition: Arc::new(platform_definition),
            shutdown_tx,
            shutdown_rx,
            in_flight: Arc::new(AtomicUsize::new(0)),
        })
    }

    /// Create from environment variables
    pub async fn from_env() -> Result<Self> {
        let config = DurableWorkerConfig::from_env();
        Self::new(config).await
    }

    /// Run the worker (blocking until shutdown)
    pub async fn run(self) -> Result<()> {
        // Wrap self in Arc for concurrent task execution
        let worker = Arc::new(self);
        worker.run_inner().await
    }

    /// Internal run implementation using Arc<Self>
    async fn run_inner(self: &Arc<Self>) -> Result<()> {
        info!(
            worker_id = %self.config.worker_id,
            "Starting durable worker"
        );

        // Clone shutdown receiver for use in the loop (changed() requires &mut)
        let mut shutdown_rx = self.shutdown_rx.clone();

        // Register with control-plane
        {
            let mut store = self.store.lock().await;
            match store
                .register_worker(
                    &self.config.worker_id,
                    None, // worker_group
                    self.config.activity_types.clone(),
                    self.config.max_concurrent_tasks as u32,
                )
                .await
            {
                Ok(()) => info!(worker_id = %self.config.worker_id, "Worker registered"),
                Err(e) => {
                    warn!(worker_id = %self.config.worker_id, error = %e, "Failed to register worker (will continue anyway)")
                }
            }
        }

        // Track time since last heartbeat
        let mut last_heartbeat = std::time::Instant::now();
        let heartbeat_interval = self.config.heartbeat_interval;

        // Fallback poll interval (longer since notifications handle most cases)
        let fallback_poll_interval = Duration::from_secs(10);

        // Try to establish notification stream (optional - polling is fallback)
        let mut notification_stream: Option<TaskNotificationStream> = None;
        {
            let mut store = self.store.lock().await;
            match store
                .subscribe_task_notifications(
                    &self.config.worker_id,
                    self.config.activity_types.clone(),
                )
                .await
            {
                Ok(stream) => {
                    info!(
                        worker_id = %self.config.worker_id,
                        "Connected to push-based task notification stream"
                    );
                    notification_stream = Some(stream);
                }
                Err(e) => {
                    debug!(
                        worker_id = %self.config.worker_id,
                        error = %e,
                        "Push notifications unavailable, using polling fallback"
                    );
                }
            }
        }

        // Track stream reconnection attempts
        let mut reconnect_backoff = Duration::from_secs(1);
        let max_reconnect_backoff = Duration::from_secs(60);

        // Main event loop with push notifications and polling fallback
        loop {
            // Check for shutdown
            if *shutdown_rx.borrow() {
                info!("Shutdown signal received, stopping worker");
                break;
            }

            // Send heartbeat if interval has passed
            if last_heartbeat.elapsed() >= heartbeat_interval {
                let mut store = self.store.lock().await;
                let current_load = self.in_flight.load(Ordering::SeqCst) as u32;
                if let Err(e) = store
                    .heartbeat_worker(&self.config.worker_id, current_load, true)
                    .await
                {
                    debug!("Failed to send worker heartbeat: {}", e);
                }
                last_heartbeat = std::time::Instant::now();
            }

            // Wait for notification, fallback poll timeout, or shutdown
            let should_poll = match &mut notification_stream {
                Some(stream) => {
                    tokio::select! {
                        // Push notification received
                        notification = stream.recv() => {
                            match notification {
                                Some(TaskNotificationEvent::TaskAvailable { activity_type, pending_count }) => {
                                    debug!(
                                        activity_type = ?activity_type,
                                        pending_count = ?pending_count,
                                        "Received task available notification"
                                    );
                                    true // Poll immediately
                                }
                                Some(TaskNotificationEvent::Heartbeat) => {
                                    // Stream is alive, continue waiting
                                    false
                                }
                                None => {
                                    // Stream ended - need to reconnect
                                    warn!(
                                        worker_id = %self.config.worker_id,
                                        "Task notification stream disconnected, switching to polling"
                                    );
                                    notification_stream = None;
                                    true // Poll to catch any missed tasks
                                }
                            }
                        }

                        // Fallback poll timeout (long interval since we have notifications)
                        _ = tokio::time::sleep(fallback_poll_interval) => {
                            debug!("Fallback poll timeout, checking for tasks");
                            true
                        }

                        // Shutdown signal
                        _ = shutdown_rx.changed() => {
                            info!("Shutdown during notification wait");
                            break;
                        }
                    }
                }
                None => {
                    // No notification stream - use regular polling with reconnect attempts
                    tokio::select! {
                        // Short poll interval when no notifications
                        _ = tokio::time::sleep(self.config.poll_interval) => {
                            // Try to reconnect periodically
                            let mut store = self.store.lock().await;
                            match store
                                .subscribe_task_notifications(
                                    &self.config.worker_id,
                                    self.config.activity_types.clone(),
                                )
                                .await
                            {
                                Ok(stream) => {
                                    info!(
                                        worker_id = %self.config.worker_id,
                                        "Reconnected to push-based task notification stream"
                                    );
                                    notification_stream = Some(stream);
                                    reconnect_backoff = Duration::from_secs(1);
                                }
                                Err(_) => {
                                    // Increase backoff, but still poll
                                    reconnect_backoff = std::cmp::min(
                                        reconnect_backoff * 2,
                                        max_reconnect_backoff,
                                    );
                                }
                            }
                            drop(store);
                            true
                        }

                        // Shutdown signal
                        _ = shutdown_rx.changed() => {
                            info!("Shutdown during poll wait");
                            break;
                        }
                    }
                }
            };

            if !should_poll {
                continue;
            }

            // Poll for tasks
            match self.poll_and_execute().await {
                Ok(executed) => {
                    if executed > 0 {
                        debug!(tasks_executed = executed, "Executed tasks");
                    }
                }
                Err(e) => {
                    error!("Error polling tasks: {}", e);
                    tokio::time::sleep(Duration::from_secs(5)).await;
                }
            }
        }

        // Deregister on shutdown - this also reclaims any tasks we had claimed
        // allowing them to be immediately picked up by other workers
        {
            let mut store = self.store.lock().await;
            match store.deregister_worker(&self.config.worker_id).await {
                Ok(tasks_reclaimed) => {
                    if tasks_reclaimed > 0 {
                        info!(
                            worker_id = %self.config.worker_id,
                            tasks_reclaimed,
                            "Worker deregistered and tasks reclaimed"
                        );
                    } else {
                        info!(worker_id = %self.config.worker_id, "Worker deregistered");
                    }
                }
                Err(e) => {
                    // Log but don't fail - the stale task reclamation will handle cleanup
                    warn!(
                        worker_id = %self.config.worker_id,
                        error = %e,
                        "Failed to deregister worker (tasks will be reclaimed after heartbeat timeout)"
                    );
                }
            }
        }

        info!("Durable worker stopped");
        Ok(())
    }

    /// Get a handle that can be used to trigger shutdown
    /// This must be called before `run()` since run consumes self
    pub fn shutdown_handle(&self) -> ShutdownHandle {
        ShutdownHandle {
            tx: self.shutdown_tx.clone(),
        }
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

/// Check if a task error is deterministic and should never be retried.
///
/// Deterministic errors reference data that is permanently gone (e.g. a deleted
/// message). Retrying will never succeed and only burns attempts while keeping
/// the workflow in `Pending` status, blocking the entire session.
fn is_non_retryable_task_error(error: &str) -> bool {
    let lower = error.to_ascii_lowercase();
    lower.contains("user message not found")
        || (lower.contains("inputatom execution failed") && lower.contains("not found"))
        || lower.contains(&MISSING_ACT_ORG_ID_ERROR.to_ascii_lowercase())
}

impl DurableWorker {
    /// Poll for tasks and execute them concurrently
    async fn poll_and_execute(self: &Arc<Self>) -> Result<usize> {
        // Calculate available slots
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

        // Claim only as many tasks as we have slots for
        let tasks = {
            let mut store = self.store.lock().await;
            store
                .claim_tasks(
                    &self.config.worker_id,
                    &self.config.activity_types,
                    available_slots,
                )
                .await
                .map_err(|e| anyhow::anyhow!("Failed to claim tasks: {}", e))?
        };

        if tasks.is_empty() {
            return Ok(0);
        }

        debug!(
            worker_id = %self.config.worker_id,
            task_count = tasks.len(),
            available_slots = available_slots,
            "Claimed tasks"
        );

        // Spawn tasks concurrently (don't await)
        for task in tasks {
            // Reserve slot before spawning
            self.in_flight.fetch_add(1, Ordering::SeqCst);

            // Clone Arc<Self> for the spawned task
            let worker = Arc::clone(self);

            tokio::spawn(async move {
                let result = worker.execute_task(&task).await;

                if let Err(e) = result {
                    let failure = summarize_task_failure(
                        task.id,
                        task.workflow_id,
                        &task.activity_type,
                        task.attempt,
                        None,
                        &task.input,
                        &e,
                    );
                    error!(
                        task_id = %task.id,
                        workflow_id = ?task.workflow_id,
                        activity_type = %task.activity_type,
                        attempt = task.attempt,
                        session_id = ?failure.session_id,
                        tool_identifiers = ?failure.tool_identifiers,
                        error_chain = %failure.error_chain,
                        "Task execution failed"
                    );

                    let error_msg = failure.persisted_message;
                    let force_dlq = is_non_retryable_task_error(&error_msg);

                    if force_dlq {
                        // Deterministic error — retrying will never succeed.
                        // Exhaust all retry attempts immediately so the task
                        // reaches DLQ, then complete the workflow.
                        warn!(
                            task_id = %task.id,
                            activity_type = %task.activity_type,
                            error = %error_msg,
                            "Non-retryable error detected, exhausting retries"
                        );

                        let mut store = worker.store.lock().await;

                        // Loop fail_task until the store says no more retries
                        // (task is in DLQ). This ensures we don't complete the
                        // workflow while the task is still retryable.
                        loop {
                            match store.fail_task(task.id, &error_msg).await {
                                Ok(will_retry) => {
                                    if !will_retry {
                                        break;
                                    }
                                    // Consume another attempt immediately.
                                }
                                Err(fail_err) => {
                                    error!(
                                        task_id = %task.id,
                                        error = %fail_err,
                                        "Failed to force task into DLQ via fail_task"
                                    );
                                    break;
                                }
                            }
                        }

                        // Task is now in DLQ — safe to complete the workflow.
                        if let Some(wf_id) = task.workflow_id {
                            let _ = store
                                .update_workflow_status(
                                    wf_id,
                                    WorkflowStatus::Completed,
                                    None,
                                    Some(error_msg.clone()),
                                )
                                .await;
                        }

                        if matches!(
                            task.activity_type.as_str(),
                            "reason" | "process_input" | "act"
                        ) {
                            drop(store);
                            worker.emit_dlq_error_event(&task, &error_msg).await;
                        }
                    } else {
                        // Report failure to store (may retry)
                        let mut store = worker.store.lock().await;
                        match store.fail_task(task.id, &error_msg).await {
                            Ok(will_retry) => {
                                if !will_retry {
                                    // Task exhausted all retries (DLQ). Mark workflow as
                                    // completed so the session doesn't get stuck.
                                    if let Some(wf_id) = task.workflow_id {
                                        warn!(
                                            task_id = %task.id,
                                            workflow_id = %wf_id,
                                            "Task moved to DLQ, completing workflow with failure"
                                        );
                                        let _ = store
                                            .update_workflow_status(
                                                wf_id,
                                                WorkflowStatus::Completed,
                                                None,
                                                Some(error_msg.clone()),
                                            )
                                            .await;
                                    }

                                    // Emit a single user-facing error event now that all
                                    // retries are exhausted. Transient errors skip event
                                    // emission during each attempt to avoid duplicates;
                                    // this is the one place we surface the final failure.
                                    if matches!(
                                        task.activity_type.as_str(),
                                        "reason" | "process_input" | "act"
                                    ) {
                                        drop(store);
                                        worker.emit_dlq_error_event(&task, &error_msg).await;
                                    }
                                }
                            }
                            Err(fail_err) => {
                                error!(
                                    task_id = %task.id,
                                    error = %fail_err,
                                    "Failed to report task failure"
                                );
                            }
                        }
                    }
                }

                // Release slot when done (success or failure)
                worker.in_flight.fetch_sub(1, Ordering::SeqCst);
            });
        }

        Ok(0) // Return 0 since we don't wait for completion
    }

    /// Execute a single task
    async fn execute_task(&self, task: &ClaimedTask) -> Result<()> {
        info!(
            task_id = %task.id,
            workflow_id = ?task.workflow_id,
            activity_type = %task.activity_type,
            attempt = task.attempt,
            "Executing task"
        );

        // Check if workflow is cancelled before executing (only for workflow-bound tasks)
        if let Some(wf_id) = task.workflow_id {
            let mut store = self.store.lock().await;
            if let Ok((status, _, _)) = store.get_workflow_status(wf_id).await
                && status == WorkflowStatus::Cancelled
            {
                info!(
                    task_id = %task.id,
                    workflow_id = %wf_id,
                    "Workflow cancelled, skipping task execution"
                );

                // Parse task input to get session context for cancellation events
                let (org_id, session_id, input_message_id) = match task.activity_type.as_str() {
                    "process_input" | "reason" => {
                        if let Ok(turn_input) =
                            serde_json::from_value::<DurableTurnInput>(task.input.clone())
                        {
                            (
                                turn_input.org_id,
                                turn_input.session_id,
                                turn_input.input_message_id,
                            )
                        } else {
                            (
                                0,
                                SessionId::from_uuid(task.workflow_id.unwrap_or_else(Uuid::nil)),
                                MessageId::from_uuid(Uuid::nil()),
                            )
                        }
                    }
                    "act" => {
                        if let Ok(act_input) =
                            serde_json::from_value::<ActTaskInput>(task.input.clone())
                        {
                            (
                                act_input.act_input.org_id.unwrap_or(0),
                                act_input.act_input.context.session_id,
                                act_input.act_input.context.input_message_id,
                            )
                        } else {
                            (
                                0,
                                SessionId::from_uuid(task.workflow_id.unwrap_or_else(Uuid::nil)),
                                MessageId::from_uuid(Uuid::nil()),
                            )
                        }
                    }
                    _ => (
                        0,
                        SessionId::from_uuid(task.workflow_id.unwrap_or_else(Uuid::nil)),
                        MessageId::from_uuid(Uuid::nil()),
                    ),
                };

                // Emit cancellation events (agent message + session.idled)
                let turn_id = TurnId::new(); // Generate turn_id for cancellation context
                drop(store); // Release lock before async call
                emit_cancellation_events(
                    &self.grpc_address,
                    org_id,
                    session_id,
                    turn_id,
                    input_message_id,
                )
                .await;

                // Mark task as failed due to cancellation
                let mut store = self.store.lock().await;
                let _ = store.fail_task(task.id, "Workflow cancelled").await;
                return Ok(());
            }
        }

        // Create a new gRPC client for this task execution
        let grpc_client = GrpcClient::connect(&self.grpc_address).await?;

        // Spawn heartbeat background task
        let task_id = task.id;
        let worker_id = self.config.worker_id.clone();
        let heartbeat_interval = self.config.heartbeat_interval;
        let store_for_heartbeat = self.store.clone();
        let (cancel_tx, mut cancel_rx) = tokio::sync::oneshot::channel::<()>();

        let heartbeat_handle = tokio::spawn(async move {
            let mut interval = tokio::time::interval(heartbeat_interval);
            loop {
                tokio::select! {
                    _ = interval.tick() => {
                        let mut store = store_for_heartbeat.lock().await;
                        match store.heartbeat_task(task_id, &worker_id, None).await {
                            Ok(response) => {
                                if response.should_cancel {
                                    warn!(task_id = %task_id, "Task cancellation requested via heartbeat");
                                    break;
                                }
                                debug!(task_id = %task_id, "Heartbeat sent");
                            }
                            Err(e) => {
                                warn!(task_id = %task_id, error = %e, "Failed to send heartbeat");
                            }
                        }
                    }
                    _ = &mut cancel_rx => {
                        debug!(task_id = %task_id, "Heartbeat loop cancelled");
                        break;
                    }
                }
            }
        });

        // Execute based on activity type - different activities have different input formats
        let (result, turn_input_opt) = match task.activity_type.as_str() {
            "process_input" | "reason" => {
                // These activities use DurableTurnInput
                let turn_input: DurableTurnInput = serde_json::from_value(task.input.clone())
                    .map_err(|e| anyhow::anyhow!("Failed to parse task input: {}", e))?;
                let res = match task.activity_type.as_str() {
                    "process_input" => {
                        self.execute_input_activity(grpc_client.clone(), &turn_input)
                            .await
                    }
                    "reason" => {
                        self.execute_reason_activity(
                            grpc_client.clone(),
                            &turn_input,
                            self.store.clone(),
                        )
                        .await
                    }
                    _ => unreachable!(),
                };
                (res, Some(turn_input))
            }
            "act" => {
                // Act activity uses ActTaskInput; org_id is read from the
                // flattened ActInput payload via require_org_id().
                let act_task_input: ActTaskInput = serde_json::from_value(task.input.clone())
                    .map_err(|e| anyhow::anyhow!("Failed to parse ActTaskInput: {}", e))?;
                let org_id = act_task_input.require_org_id()?;
                // Create DurableTurnInput from ActInput context for scheduling next activity
                // Include turn_id from act_input context for trace correlation
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
                let mut turn_input = act_task_input.resume_state.unwrap_or(DurableTurnInput {
                    org_id,
                    session_id: act_task_input.act_input.context.session_id,
                    harness_id: act_task_input.act_input.harness_id,
                    agent_id: act_task_input.act_input.agent_id,
                    input_message_id: act_task_input.act_input.context.input_message_id,
                    turn_id: Some(act_task_input.act_input.context.turn_id),
                    previous_response_id: previous_response_id.clone(),
                    iteration,
                    request_id: act_task_input.request_id.clone(),
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
                turn_input.request_id = act_task_input.request_id.clone();
                let res = self
                    .execute_act_activity(grpc_client.clone(), org_id, act_task_input.act_input)
                    .await;
                (res, Some(turn_input))
            }
            "leased_resource_cleanup" => {
                let cleanup_input: crate::leased_resource_cleanup::LeasedResourceCleanupInput =
                    serde_json::from_value(task.input.clone())
                        .map_err(|e| anyhow::anyhow!("Failed to parse cleanup input: {}", e))?;
                let adapters = crate::grpc_worker_adapters::GrpcWorkerAdapters::from_client_with_platform_definition(
                    grpc_client.clone(),
                    self.platform_definition.as_ref().clone(),
                );
                let res = crate::leased_resource_cleanup::execute_cleanup_activity(
                    &adapters,
                    &cleanup_input,
                )
                .await;
                (res, None)
            }
            "invoke_scheduled_app_channel" => {
                let input: ScheduledAppChannelInput = serde_json::from_value(task.input.clone())
                    .map_err(|e| anyhow::anyhow!("Failed to parse scheduled app input: {}", e))?;
                let adapters = crate::grpc_worker_adapters::GrpcWorkerAdapters::from_client_with_platform_definition(
                    grpc_client.clone(),
                    self.platform_definition.as_ref().clone(),
                );
                let res = adapters
                    .invoke_scheduled_app_channel(input.org_id, &input.app_id, &input.channel_id)
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

        // Stop heartbeat loop
        let _ = cancel_tx.send(());
        let _ = heartbeat_handle.await;

        match result {
            Ok(output) => {
                // Complete the task - this verifies we still own it
                // If task was reclaimed while we were executing, we must NOT schedule next activity
                let complete_result = {
                    let mut store = self.store.lock().await;
                    store
                        .complete_task(task.id, &self.config.worker_id, output.clone())
                        .await
                };

                match complete_result {
                    Ok(()) => {
                        info!(
                            task_id = %task.id,
                            activity_type = %task.activity_type,
                            "Task completed successfully"
                        );

                        // Only schedule next activity if we successfully completed the task
                        // and it has a parent workflow. Standalone tasks have no next activity.
                        if let (Some(turn_input), Some(wf_id)) = (turn_input_opt, task.workflow_id)
                        {
                            self.schedule_next_activity(
                                grpc_client,
                                wf_id,
                                &task.activity_type,
                                &turn_input,
                                &output,
                            )
                            .await?;
                        }
                    }
                    Err(e) => {
                        // Check if this is a "task not owned" error (expected during reclaim)
                        // vs other errors (network, database) that should be propagated
                        let error_str = e.to_string();
                        if error_str.contains("not owned") || error_str.contains("reclaimed") {
                            // Task was reclaimed by another worker or already completed
                            // This is expected during heartbeat timeout recovery
                            // Do NOT schedule next activity - the other worker will do it
                            warn!(
                                task_id = %task.id,
                                activity_type = %task.activity_type,
                                error = %e,
                                "Task completion rejected (reclaimed or already completed) - skipping next activity scheduling"
                            );
                        } else {
                            // Other errors (network, database) should be propagated
                            // so the failure handler can run and retry the task
                            return Err(e);
                        }
                    }
                }
            }
            Err(e) => {
                return Err(e);
            }
        }

        Ok(())
    }

    /// Emit a single user-facing error event when a task exhausts all retries (DLQ).
    ///
    /// Transient LLM errors skip event emission during each retry attempt to
    /// prevent duplicate "I encountered an error" messages in the UI. This method
    /// emits one final error event so the user sees exactly one error message,
    /// then emits session.idled + sets session status to idle so the UI unblocks.
    async fn emit_dlq_error_event(&self, task: &ClaimedTask, persisted_error: &str) {
        // Extract session context from the task input. `act` task input is
        // wrapped in `ActTaskInput`, while `reason`/`process_input` use
        // `DurableTurnInput`; both shapes must be handled or act-task DLQs
        // skip emitting session.idled and leave the session stuck.
        let ctx = match extract_task_session_context(&task.activity_type, &task.input) {
            Ok(ctx) => ctx,
            Err(err) => {
                warn!(
                    task_id = %task.id,
                    activity_type = %task.activity_type,
                    error = format!("{err:#}"),
                    "Cannot emit DLQ error event: failed to parse task input"
                );
                return;
            }
        };

        let grpc_client = match GrpcClient::connect(&self.grpc_address).await {
            Ok(client) => client,
            Err(e) => {
                warn!(
                    task_id = %task.id,
                    error = %e,
                    "Cannot emit DLQ error event: failed to connect to gRPC"
                );
                return;
            }
        };

        let event_emitter = GrpcEventEmitter::new(grpc_client.clone());
        let TaskSessionContext {
            org_id,
            session_id,
            turn_id,
            input_message_id,
        } = ctx;
        let user_error = user_facing_failure(persisted_error);
        let mut error_message = Message::assistant(user_error.fallback_message());
        let mut metadata = std::collections::HashMap::new();
        user_error.apply_to_message_metadata(&mut metadata);
        error_message.metadata = Some(metadata);
        let context = EventContext::turn(turn_id, input_message_id);

        if let Err(e) = event_emitter
            .emit(EventRequest::new(
                session_id,
                context,
                OutputMessageCompletedData::new(error_message).with_user_facing_error(&user_error),
            ))
            .await
        {
            warn!(
                task_id = %task.id,
                session_id = %session_id,
                error = %e,
                "Failed to emit DLQ error event"
            );
        } else {
            info!(
                task_id = %task.id,
                session_id = %session_id,
                "Emitted DLQ error event for user"
            );
        }

        // Emit session.idled so the UI unblocks
        let idled_event = EventRequest::new(
            session_id,
            EventContext::turn(turn_id, input_message_id),
            SessionIdledData {
                turn_id,
                iterations: None,
                usage: None,
            },
        );
        if let Err(e) = event_emitter.emit(idled_event).await {
            warn!(session_id = %session_id, error = %e, "Failed to emit session.idled after DLQ");
        }

        // Set session status to idle
        if let Err(e) = grpc_client
            .set_session_status(org_id, session_id, "idle")
            .await
        {
            warn!(session_id = %session_id, error = %e, "Failed to set session idle after DLQ");
        }
    }

    /// Execute input processing activity
    async fn execute_input_activity(
        &self,
        grpc_client: GrpcClient,
        input: &DurableTurnInput,
    ) -> Result<serde_json::Value> {
        debug!(
            session_id = %input.session_id,
            "Executing input activity"
        );

        // Create AtomContext for this execution
        let context = AtomContext {
            session_id: input.session_id,
            turn_id: TurnId::new(),
            input_message_id: input.input_message_id,
            exec_id: ExecId::new(),
        };

        let atom_input = InputAtomInput {
            context: context.clone(),
        };

        // Use the existing input_activity function with gRPC adapters
        let result = input_activity(grpc_client, input.org_id, atom_input).await?;

        // Include turn_id in output for propagation to subsequent activities
        // TurnId.to_string() returns prefixed format "turn_abc123"
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
    ///
    /// This activity is protected by a per-provider circuit breaker to prevent
    /// cascading failures when an LLM provider is unavailable. Each provider
    /// (openai, anthropic, gemini, etc.) has its own breaker so an outage in
    /// one does not block the others.
    async fn execute_reason_activity(
        &self,
        grpc_client: GrpcClient,
        input: &DurableTurnInput,
        store: Arc<Mutex<GrpcDurableStore>>,
    ) -> Result<serde_json::Value> {
        debug!(
            session_id = %input.session_id,
            turn_id = ?input.turn_id,
            "Executing reason activity"
        );

        // Fetch turn context to get MCP tool definitions and model/provider info.
        // We need the provider type before the circuit breaker check so breaker
        // keys are per-provider (e.g. "llm:openai", "llm:anthropic").
        let turn_context = load_turn_context(&grpc_client, input.org_id, input.session_id)
            .await
            .map_err(|e| anyhow::anyhow!("Failed to load turn context: {}", e))?;

        // Per-provider circuit breaker key (e.g. "llm:openai", "llm:anthropic").
        // Falls back to non-colliding "llm:unknown" when model/provider info is unavailable.
        let circuit_key_owned = match &turn_context.model {
            Some(m) => format!("llm:{}", m.provider_type),
            None => "llm:unknown".to_string(),
        };
        let circuit_key = circuit_key_owned.as_str();

        // Check circuit breaker before making LLM call
        {
            let mut store_guard = store.lock().await;
            let check = store_guard.check_circuit_breaker(circuit_key).await;
            match check {
                Ok(result) => {
                    if !result.allowed {
                        warn!(
                            circuit_key = circuit_key,
                            state = ?result.state,
                            "Circuit breaker is open - rejecting LLM call"
                        );
                        return Err(anyhow::anyhow!(
                            "LLM provider temporarily unavailable (circuit breaker open)"
                        ));
                    }
                }
                Err(e) => {
                    // Log but continue - don't block on circuit breaker check failure
                    warn!(error = %e, "Failed to check circuit breaker, proceeding anyway");
                }
            }
        }

        // Create AtomContext - use turn_id from input if available (from input activity)
        let turn_id = input.turn_id.unwrap_or_default();
        let context = AtomContext {
            session_id: input.session_id,
            turn_id,
            input_message_id: input.input_message_id,
            exec_id: ExecId::new(),
        };

        let reason_input = ReasonInput {
            context,
            harness_id: input.harness_id,
            agent_id: input.agent_id,
            org_id: input.org_id,
            mcp_tool_definitions: turn_context.mcp_tool_definitions,
            previous_response_id: input.previous_response_id.clone(),
            iteration: input.iteration,
        };

        // Use the existing reason_activity function with gRPC adapters
        let result = reason_activity(
            grpc_client,
            input.org_id,
            reason_input,
            self.platform_definition.as_ref(),
        )
        .await;

        // Record circuit breaker outcome
        // LLM failures are wrapped as Ok(ReasonResult { success: false }) by the atom,
        // so we must check result.success to detect them for the circuit breaker.
        {
            let mut store_guard = store.lock().await;
            let is_llm_failure = match &result {
                Ok(reason_result) => {
                    // Dependency blockers (archived/deleted harness/agent) return
                    // success=false before any LLM call. Do not count them as
                    // provider outages for the circuit breaker.
                    !reason_result.success
                        && reason_result.error.as_deref() != Some("dependency_unavailable")
                }
                Err(_) => true,
            };

            if is_llm_failure {
                match store_guard
                    .record_circuit_breaker_failure(circuit_key)
                    .await
                {
                    Ok(failure_result) => {
                        if failure_result.circuit_opened {
                            warn!(
                                circuit_key = circuit_key,
                                "Circuit breaker opened due to LLM failures"
                            );
                        }
                    }
                    Err(e) => {
                        warn!(error = %e, "Failed to record circuit breaker failure");
                    }
                }
            } else if let Err(e) = store_guard
                .record_circuit_breaker_success(circuit_key)
                .await
            {
                warn!(error = %e, "Failed to record circuit breaker success");
            }
        }

        let result = result?;

        // Transient LLM failures (server errors, rate limits, timeouts) should be
        // retried at the durable task level, not just swallowed as "normal" failures.
        // The atom's internal retries handle short-lived blips; the durable engine's
        // task retries (with longer exponential backoff) handle sustained outages.
        if !result.success
            && let Some(ref error_msg) = result.error
            && everruns_core::llm_retry::is_transient_error_message(error_msg)
        {
            return Err(anyhow::anyhow!(
                "Transient LLM error (will retry at task level): {}",
                error_msg
            ));
        }

        Ok(serde_json::to_value(&result)?)
    }

    /// Execute act activity (tool execution)
    async fn execute_act_activity(
        &self,
        grpc_client: GrpcClient,
        org_id: i64,
        act_input: ActInput,
    ) -> Result<serde_json::Value> {
        debug!(
            org_id = org_id,
            session_id = %act_input.context.session_id,
            tool_count = act_input.tool_calls.len(),
            "Executing act activity"
        );

        // Use the existing act_activity function with gRPC adapters
        let result = act_activity(
            grpc_client,
            org_id,
            act_input,
            self.platform_definition.as_ref(),
        )
        .await?;

        Ok(serde_json::to_value(&result)?)
    }

    /// Schedule the next activity based on current activity completion
    async fn schedule_next_activity(
        &self,
        grpc_client: GrpcClient,
        workflow_id: Uuid,
        completed_activity: &str,
        input: &DurableTurnInput,
        output: &serde_json::Value,
    ) -> Result<()> {
        let mut store = self.store.lock().await;

        // Check if workflow is cancelled before scheduling next activity
        if let Ok((status, _, _)) = store.get_workflow_status(workflow_id).await
            && status == WorkflowStatus::Cancelled
        {
            info!(
                workflow_id = %workflow_id,
                completed_activity = %completed_activity,
                "Workflow cancelled, not scheduling next activity"
            );

            // Emit cancellation events (agent message + session.idled)
            let turn_id = TurnId::new(); // Generate turn_id for cancellation context
            drop(store); // Release lock before async call
            emit_cancellation_events(
                &self.grpc_address,
                input.org_id,
                input.session_id,
                turn_id,
                input.input_message_id,
            )
            .await;

            return Ok(());
        }

        let adapters =
            crate::grpc_worker_adapters::GrpcWorkerAdapters::from_client_with_platform_definition(
                grpc_client,
                self.platform_definition.as_ref().clone(),
            );
        let host = WorkerRuntimeHost::new(adapters);
        let pending_user_message_count = if completed_activity == "reason" {
            let reason_result: everruns_core::ReasonResult = serde_json::from_value(output.clone())
                .map_err(|error| anyhow::anyhow!("Invalid reason output payload: {}", error))?;
            if reason_result.success && !reason_result.has_tool_calls {
                store
                    .get_and_consume_signals(workflow_id)
                    .await
                    .map_err(|error| {
                        anyhow::anyhow!("Failed to consume workflow signals: {}", error)
                    })?
                    .into_iter()
                    .filter(|signal| {
                        signal.signal_type == everruns_durable::signal_types::USER_MESSAGE
                    })
                    .count()
            } else {
                0
            }
        } else {
            0
        };

        match plan_next_host_turn(
            &host,
            completed_activity,
            input,
            output,
            pending_user_message_count,
        )
        .await?
        {
            RuntimeTurnPlan::ScheduleReason(next) => {
                store
                    .enqueue_task(
                        workflow_id,
                        format!("reason_{}", Uuid::now_v7()),
                        "reason".to_string(),
                        serde_json::to_value(&next)?,
                    )
                    .await
                    .map_err(|e| anyhow::anyhow!("Failed to enqueue reason task: {}", e))?;
            }
            RuntimeTurnPlan::ScheduleAct(plan) => {
                let input_json = serialize_act_plan_for_durable_worker(&plan)?;
                store
                    .enqueue_task(
                        workflow_id,
                        format!("act_{}", Uuid::now_v7()),
                        "act".to_string(),
                        input_json,
                    )
                    .await
                    .map_err(|e| anyhow::anyhow!("Failed to enqueue act task: {}", e))?;
            }
            RuntimeTurnPlan::Complete { error } => {
                store
                    .update_workflow_status(workflow_id, WorkflowStatus::Completed, None, error)
                    .await
                    .map_err(|e| anyhow::anyhow!("Failed to update workflow status: {}", e))?;
            }
            RuntimeTurnPlan::WaitForToolResults { resume } => {
                store
                    .update_workflow_status(
                        workflow_id,
                        WorkflowStatus::Completed,
                        Some(serde_json::to_value(&resume)?),
                        None,
                    )
                    .await
                    .map_err(|e| {
                        anyhow::anyhow!("Failed to persist wait-for-tools state: {}", e)
                    })?;
            }
        }

        Ok(())
    }
}

fn serialize_act_plan_for_durable_worker(plan: &RuntimeActPlan) -> Result<serde_json::Value> {
    if plan.input.org_id.is_none() {
        return Err(anyhow::anyhow!(
            "ActInput.org_id must be set for durable worker scheduling"
        ));
    }
    let mut input_json = serde_json::to_value(ActTaskInput {
        request_id: plan.request_id.clone(),
        resume_state: Some(plan.resume_state.as_ref().clone()),
        act_input: plan.input.clone(),
    })?;
    if let Some(response_id) = &plan.previous_response_id {
        input_json["previous_response_id"] = serde_json::json!(response_id);
    }
    input_json["iteration"] = serde_json::json!(plan.iteration);
    Ok(input_json)
}

#[cfg(test)]
mod tests {
    use super::*;
    use everruns_core::typed_id::HarnessId;
    use serde_json::json;

    #[test]
    fn test_config_default() {
        let config = DurableWorkerConfig::default();
        assert!(config.worker_id.starts_with("worker-"));
        assert_eq!(config.max_concurrent_tasks, 1000);
        assert_eq!(config.grpc_address, "127.0.0.1:9001");
    }

    #[test]
    fn test_is_non_retryable_task_error_detects_missing_message() {
        assert!(is_non_retryable_task_error(
            "Message store error: User message not found: msg_abc123"
        ));
        assert!(is_non_retryable_task_error(
            "InputAtom execution failed: Message store error: User message not found: msg_xyz"
        ));
    }

    #[test]
    fn test_is_non_retryable_task_error_detects_missing_act_org_id() {
        assert!(is_non_retryable_task_error(
            "ActAtom execution failed: ActTaskInput.act_input.org_id must be set"
        ));
    }

    #[test]
    fn test_is_non_retryable_task_error_allows_transient_errors() {
        assert!(!is_non_retryable_task_error(
            "Transient LLM error (will retry at task level): server_error"
        ));
        assert!(!is_non_retryable_task_error("Failed to connect to gRPC"));
        assert!(!is_non_retryable_task_error(
            "LLM provider temporarily unavailable (circuit breaker open)"
        ));
    }

    #[test]
    fn test_is_non_retryable_task_error_with_formatted_task_failure() {
        let task_id = Uuid::parse_str("019d97fa-c961-7272-a2f8-fef2220f0ec1").unwrap();
        let workflow_id = Some(Uuid::parse_str("019d9861-b17a-71d3-b929-eb0e7ae2dc55").unwrap());
        let input = json!({
            "act_input": {
                "context": {
                    "session_id": "session_019d97fa3d8f736195d605c1ace8b83c"
                }
            }
        });
        let error = anyhow::anyhow!("Message store error: User message not found: msg_abc123")
            .context("ActAtom execution failed");

        let failure =
            summarize_task_failure(task_id, workflow_id, "act", 1, Some(3), &input, &error);

        assert!(is_non_retryable_task_error(&failure.persisted_message));
    }

    #[test]
    fn test_extract_task_session_context_reason() {
        let session_id = SessionId::new();
        let input_message_id = MessageId::new();
        let turn_id = TurnId::new();
        let turn_input = DurableTurnInput {
            org_id: 42,
            session_id,
            harness_id: HarnessId::new(),
            agent_id: None,
            input_message_id,
            turn_id: Some(turn_id),
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
        };
        let value = serde_json::to_value(&turn_input).unwrap();

        let ctx = extract_task_session_context("reason", &value).expect("reason input parses");

        assert_eq!(ctx.org_id, 42);
        assert_eq!(ctx.session_id, session_id);
        assert_eq!(ctx.turn_id, turn_id);
        assert_eq!(ctx.input_message_id, input_message_id);
    }

    #[test]
    fn test_extract_task_session_context_act() {
        use everruns_core::atoms::{ActInput, AtomContext};
        use everruns_core::typed_id::ExecId;

        let session_id = SessionId::new();
        let input_message_id = MessageId::new();
        let turn_id = TurnId::new();
        let act_task_input = ActTaskInput {
            request_id: Some("req_123".to_string()),
            resume_state: None,
            act_input: ActInput {
                org_id: Some(7),
                context: AtomContext {
                    session_id,
                    turn_id,
                    input_message_id,
                    exec_id: ExecId::new(),
                },
                harness_id: HarnessId::new(),
                agent_id: None,
                tool_calls: vec![],
                tool_definitions: vec![],
                locale: None,
                blueprint_id: None,
                network_access: None,
            },
        };
        let value = serde_json::to_value(&act_task_input).unwrap();

        let ctx = extract_task_session_context("act", &value)
            .expect("act input parses — regression for EVE-306");

        assert_eq!(ctx.org_id, 7);
        assert_eq!(ctx.session_id, session_id);
        assert_eq!(ctx.turn_id, turn_id);
        assert_eq!(ctx.input_message_id, input_message_id);
    }

    #[test]
    fn test_extract_task_session_context_unknown_activity() {
        let value = json!({ "irrelevant": true });
        let err = extract_task_session_context("heartbeat", &value).expect_err("unknown errs");
        assert!(format!("{err:#}").contains("heartbeat"));
    }

    #[test]
    fn test_extract_task_session_context_act_malformed_includes_parse_error() {
        let value = json!({ "org_id": 1, "context": { "session_id": "not-a-session-id" } });
        let err = extract_task_session_context("act", &value).expect_err("malformed errs");
        let chain = format!("{err:#}");
        assert!(
            chain.contains("ActTaskInput"),
            "expected wrapping context in error chain: {chain}"
        );
    }

    /// Regression for EVE-325: ensure the inner `ActInput.org_id` survives a
    /// serde round-trip through `ActTaskInput`. Previously the outer
    /// `ActTaskInput.org_id` and flattened `ActInput.org_id` collided on the
    /// same JSON key and deserialization dropped the inner value, causing
    /// every tool call in the durable worker path to fail with
    /// "ActInput.org_id must be set for runtime host execution".
    #[test]
    fn act_task_input_roundtrip_preserves_inner_org_id() {
        use everruns_core::atoms::{ActInput, AtomContext};
        use everruns_core::typed_id::ExecId;

        let input = ActTaskInput {
            request_id: None,
            resume_state: None,
            act_input: ActInput {
                org_id: Some(7),
                context: AtomContext {
                    session_id: SessionId::new(),
                    turn_id: TurnId::new(),
                    input_message_id: MessageId::new(),
                    exec_id: ExecId::new(),
                },
                harness_id: HarnessId::new(),
                agent_id: None,
                tool_calls: vec![],
                tool_definitions: vec![],
                locale: None,
                blueprint_id: None,
                network_access: None,
            },
        };
        let json = serde_json::to_value(&input).unwrap();
        let decoded: ActTaskInput = serde_json::from_value(json).unwrap();
        assert_eq!(decoded.act_input.org_id, Some(7));
    }

    /// Regression guard: `ActTaskInput` carries a `request_id` alongside the
    /// flattened `ActInput`. A serde round-trip must preserve both without
    /// collisions so durable act-task replay observes the same correlation ID.
    #[test]
    fn act_task_input_roundtrip_preserves_request_id_and_inner_context() {
        use everruns_core::atoms::{ActInput, AtomContext};
        use everruns_core::typed_id::ExecId;

        let session_id = SessionId::new();
        let turn_id = TurnId::new();
        let input_message_id = MessageId::new();
        let exec_id = ExecId::new();

        let input = ActTaskInput {
            request_id: Some("req_abc".to_string()),
            resume_state: None,
            act_input: ActInput {
                org_id: Some(42),
                context: AtomContext {
                    session_id,
                    turn_id,
                    input_message_id,
                    exec_id,
                },
                harness_id: HarnessId::new(),
                agent_id: None,
                tool_calls: vec![],
                tool_definitions: vec![],
                locale: None,
                blueprint_id: None,
                network_access: None,
            },
        };
        let json = serde_json::to_value(&input).unwrap();
        let decoded: ActTaskInput = serde_json::from_value(json).unwrap();
        assert_eq!(decoded.request_id.as_deref(), Some("req_abc"));
        assert_eq!(decoded.act_input.org_id, Some(42));
        assert_eq!(decoded.act_input.context.session_id, session_id);
        assert_eq!(decoded.act_input.context.turn_id, turn_id);
        assert_eq!(decoded.act_input.context.input_message_id, input_message_id);
        assert_eq!(decoded.act_input.context.exec_id, exec_id);
    }

    /// When an act task reaches DLQ with its inner `org_id` missing, the
    /// extractor still returns a usable session context so the worker can
    /// emit `session.idled` and unblock the UI. The wrapper falls back to 0
    /// on purpose — `require_org_id` handles the refusal on the execution
    /// path, but DLQ emission must tolerate the missing value.
    #[test]
    fn test_extract_task_session_context_act_tolerates_missing_inner_org_id() {
        use everruns_core::atoms::{ActInput, AtomContext};
        use everruns_core::typed_id::ExecId;

        let session_id = SessionId::new();
        let turn_id = TurnId::new();
        let input_message_id = MessageId::new();
        let act_task_input = ActTaskInput {
            request_id: None,
            resume_state: None,
            act_input: ActInput {
                org_id: None,
                context: AtomContext {
                    session_id,
                    turn_id,
                    input_message_id,
                    exec_id: ExecId::new(),
                },
                harness_id: HarnessId::new(),
                agent_id: None,
                tool_calls: vec![],
                tool_definitions: vec![],
                locale: None,
                blueprint_id: None,
                network_access: None,
            },
        };
        let value = serde_json::to_value(&act_task_input).unwrap();

        let ctx = extract_task_session_context("act", &value)
            .expect("act input without inner org_id must still parse for DLQ path");

        assert_eq!(ctx.org_id, 0);
        assert_eq!(ctx.session_id, session_id);
        assert_eq!(ctx.turn_id, turn_id);
        assert_eq!(ctx.input_message_id, input_message_id);
    }

    /// `process_input`/`reason` tasks may land without a `turn_id` (very
    /// early lifecycle). Extraction must return a default rather than
    /// erroring so the DLQ path can still surface user-facing events.
    #[test]
    fn test_extract_task_session_context_reason_defaults_missing_turn_id() {
        let session_id = SessionId::new();
        let input_message_id = MessageId::new();
        let turn_input = DurableTurnInput {
            org_id: 3,
            session_id,
            harness_id: HarnessId::new(),
            agent_id: None,
            input_message_id,
            turn_id: None,
            previous_response_id: None,
            iteration: 0,
            request_id: None,
            started_at: None,
            cumulative_usage: None,
            tool_call_count: 0,
            llm_call_count: 0,
            time_to_first_token_ms: None,
            final_message_id: None,
            final_answer_preview: None,
        };
        let value = serde_json::to_value(&turn_input).unwrap();

        let ctx =
            extract_task_session_context("reason", &value).expect("turn_id=None still parses");

        assert_eq!(ctx.org_id, 3);
        assert_eq!(ctx.session_id, session_id);
        assert_eq!(ctx.input_message_id, input_message_id);
        // A synthesised TurnId is used so downstream EventContext::turn
        // still accepts the value and DLQ emission is not short-circuited.
        // We only assert the context was constructed successfully above.
        let _ = ctx.turn_id;
    }

    /// Guard against classifier drift: transient LLM failures that the
    /// worker retries must not be marked non-retryable. Missing-user-message
    /// and missing-act-org-id remain the only non-retryable patterns.
    #[test]
    fn test_is_non_retryable_task_error_does_not_match_generic_not_found() {
        // A 404 from the LLM provider is not the same as a missing user
        // message — the retry loop must keep handling those.
        assert!(!is_non_retryable_task_error(
            "OpenAI API error (404): model not found"
        ));
        assert!(!is_non_retryable_task_error(
            "ReasonAtom execution failed: OpenAI API error (404): model not found"
        ));
        // Budget exhausted surfaces as a task failure, but the worker still
        // funnels it through the single DLQ emission path rather than
        // short-circuiting retries here.
        assert!(!is_non_retryable_task_error(
            "ReasonAtom execution failed: Budget exhausted. 1.00 tokens spent reached the 1.00 tokens limit."
        ));
    }

    /// Non-retryable detection is case-insensitive so the classifier keeps
    /// matching even if upstream error wrapping changes casing.
    #[test]
    fn test_is_non_retryable_task_error_is_case_insensitive() {
        assert!(is_non_retryable_task_error(
            "Message store error: USER MESSAGE NOT FOUND: msg_abc"
        ));
        assert!(is_non_retryable_task_error(
            "ACTTASKINPUT.ACT_INPUT.ORG_ID MUST BE SET"
        ));
    }
}
