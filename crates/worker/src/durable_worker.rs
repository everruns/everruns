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
use std::sync::Arc;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::time::Duration;
use tokio::sync::{Mutex, watch};
use tracing::{debug, error, info, warn};
use uuid::Uuid;

use crate::activities::{
    ActInput, InputAtomInput, ReasonInput, ReasonResult, act_activity, input_activity,
    reason_activity,
};
use crate::durable_runner::DurableTurnInput;
use crate::grpc_adapters::{GrpcClient, GrpcEventEmitter, load_turn_context};
use crate::grpc_durable_store::{
    ClaimedTask, GrpcDurableStore, TaskNotificationEvent, TaskNotificationStream, WorkflowStatus,
};
use serde::{Deserialize, Serialize};

// =============================================================================
// Act Task Input Wrapper
// =============================================================================

/// Wrapper for act task input that includes org_id
/// This keeps org_id (infrastructure concern) out of core types
#[derive(Debug, Clone, Serialize, Deserialize)]
struct ActTaskInput {
    org_id: i64,
    #[serde(flatten)]
    act_input: ActInput,
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
            ],
            max_concurrent_tasks: 1000, // High default for massive workflow parallelism
            poll_interval: Duration::from_millis(100), // Fallback when push notifications unavailable
            heartbeat_interval: Duration::from_secs(10),
            grpc_address: "127.0.0.1:9001".to_string(),
            connect_timeout: Duration::from_secs(30),
        }
    }
}

impl DurableWorkerConfig {
    /// Create configuration from environment variables
    pub fn from_env() -> Self {
        let worker_id =
            std::env::var("WORKER_ID").unwrap_or_else(|_| format!("worker-{}", Uuid::now_v7()));

        let grpc_address =
            std::env::var("WORKER_GRPC_ADDRESS").unwrap_or_else(|_| "127.0.0.1:9001".to_string());

        let max_concurrent = std::env::var("MAX_CONCURRENT_TASKS")
            .ok()
            .and_then(|s| s.parse().ok())
            .unwrap_or(10);

        let connect_timeout_secs: u64 = std::env::var("WORKER_GRPC_CONNECT_TIMEOUT")
            .ok()
            .and_then(|s| s.parse().ok())
            .unwrap_or(30);

        Self {
            worker_id,
            grpc_address,
            max_concurrent_tasks: max_concurrent,
            connect_timeout: Duration::from_secs(connect_timeout_secs),
            ..Default::default()
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
                    error!(
                        task_id = %task.id,
                        activity_type = %task.activity_type,
                        error = %e,
                        "Task execution failed"
                    );

                    let error_msg = e.to_string();
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
                                    Some(error_msg),
                                )
                                .await;
                        }

                        if matches!(
                            task.activity_type.as_str(),
                            "reason" | "process_input" | "act"
                        ) {
                            drop(store);
                            worker.emit_dlq_error_event(&task).await;
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
                                                Some(error_msg),
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
                                        worker.emit_dlq_error_event(&task).await;
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
                                act_input.org_id,
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
                    "process_input" => self.execute_input_activity(grpc_client, &turn_input).await,
                    "reason" => {
                        self.execute_reason_activity(grpc_client, &turn_input, self.store.clone())
                            .await
                    }
                    _ => unreachable!(),
                };
                (res, Some(turn_input))
            }
            "act" => {
                // Act activity uses ActTaskInput wrapper to include org_id
                let act_task_input: ActTaskInput = serde_json::from_value(task.input.clone())
                    .map_err(|e| anyhow::anyhow!("Failed to parse ActTaskInput: {}", e))?;
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
                let turn_input = DurableTurnInput {
                    org_id: act_task_input.org_id,
                    session_id: act_task_input.act_input.context.session_id,
                    harness_id: act_task_input.act_input.harness_id,
                    agent_id: act_task_input.act_input.agent_id,
                    input_message_id: act_task_input.act_input.context.input_message_id,
                    turn_id: Some(act_task_input.act_input.context.turn_id),
                    previous_response_id,
                    iteration,
                };
                let res = self
                    .execute_act_activity(
                        grpc_client,
                        act_task_input.org_id,
                        act_task_input.act_input,
                    )
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

                        // After act activity: if ActAtom signaled waiting_for_tool_results
                        // (connection setup, client-side tools), check the setup_connection
                        // client hint before pausing. When the hint is true, set the session
                        // status so the client can handle the tool calls. When absent, skip
                        // the pause — the LLM will see the tool errors and inform the user.
                        if task.activity_type == "act"
                            && let Some(ref ti) = turn_input_opt
                        {
                            let waiting = output
                                .get("waiting_for_tool_results")
                                .and_then(|v| v.as_bool())
                                .unwrap_or(false);
                            if waiting {
                                // Check the setup_connection hint before pausing
                                let hint_enabled = {
                                    use everruns_core::traits::SessionStore;
                                    let grpc_client =
                                        GrpcClient::connect(&self.grpc_address).await?;
                                    let session_store = crate::grpc_adapters::GrpcSessionStore::new(
                                        grpc_client.clone(),
                                        ti.org_id,
                                    );
                                    match session_store.get_session(ti.session_id).await {
                                        Ok(Some(session)) => {
                                            let hints = everruns_core::Controls::resolve_hints(
                                                session.hints.as_ref(),
                                                None,
                                            );
                                            hints
                                                .get("setup_connection")
                                                .and_then(|v| v.as_bool())
                                                .unwrap_or(false)
                                        }
                                        _ => false,
                                    }
                                };

                                if hint_enabled {
                                    let grpc_client =
                                        GrpcClient::connect(&self.grpc_address).await?;
                                    if let Err(e) = grpc_client
                                        .set_session_status(
                                            ti.org_id,
                                            ti.session_id,
                                            "waiting_for_tool_results",
                                        )
                                        .await
                                    {
                                        warn!(error = %e, "Failed to set session status to waiting_for_tool_results");
                                    }
                                } else {
                                    info!(
                                        "setup_connection hint absent; skipping wait for tool results"
                                    );
                                }
                            }
                        }

                        // Only schedule next activity if we successfully completed the task
                        // and it has a parent workflow. Standalone tasks have no next activity.
                        if let (Some(turn_input), Some(wf_id)) = (turn_input_opt, task.workflow_id)
                        {
                            self.schedule_next_activity(
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
    async fn emit_dlq_error_event(&self, task: &ClaimedTask) {
        // Try to extract session context from the task input.
        // For "act" tasks the input is wrapped in ActTaskInput; try DurableTurnInput first,
        // then fall back to extracting session_id/turn_id from the JSON directly.
        let turn_input: Option<DurableTurnInput> = serde_json::from_value(task.input.clone()).ok();

        let Some(turn_input) = turn_input else {
            warn!(
                task_id = %task.id,
                "Cannot emit DLQ error event: failed to parse task input"
            );
            return;
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
        let session_id = turn_input.session_id;
        let turn_id = turn_input.turn_id.unwrap_or_default();
        let input_message_id = turn_input.input_message_id;
        let error_message = Message::assistant(
            "I encountered an error while processing your request. Please try again later.",
        );
        let context = EventContext::turn(turn_id, input_message_id);

        if let Err(e) = event_emitter
            .emit(EventRequest::new(
                session_id,
                context,
                OutputMessageCompletedData::new(error_message),
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
            .set_session_status(turn_input.org_id, session_id, "idle")
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
    /// This activity is protected by a circuit breaker to prevent cascading
    /// failures when the LLM provider is unavailable.
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

        // Circuit breaker key for LLM calls
        // TODO: Make this per-provider (openai, anthropic) based on model config
        let circuit_key = "llm";

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

        // Fetch turn context to get MCP tool definitions
        // Note: This fetches agent, session, messages in a single batched call
        // but reason_activity will refetch them via individual stores.
        // The key value here is the mcp_tool_definitions.
        let turn_context = load_turn_context(&grpc_client, input.org_id, input.session_id)
            .await
            .map_err(|e| anyhow::anyhow!("Failed to load turn context: {}", e))?;

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
                Ok(reason_result) => !reason_result.success,
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
        workflow_id: Uuid,
        completed_activity: &str,
        input: &DurableTurnInput,
        output: &serde_json::Value,
    ) -> Result<()> {
        // Clone input so we can update previous_response_id for chaining
        let mut chained_input = input.clone();
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

        match completed_activity {
            "process_input" => {
                // Extract turn_id from output (set by execute_input_activity)
                // TurnId is serialized as prefixed string "turn_abc123"
                let turn_id: Option<TurnId> = output
                    .get("turn_id")
                    .and_then(|v| v.as_str())
                    .and_then(|s| s.parse().ok());

                // Create input with turn_id for subsequent activities
                let input_with_turn = DurableTurnInput {
                    org_id: input.org_id,
                    session_id: input.session_id,
                    harness_id: input.harness_id,
                    agent_id: input.agent_id,
                    input_message_id: input.input_message_id,
                    turn_id,
                    previous_response_id: None,
                    iteration: 1,
                };
                let input_json = serde_json::to_value(&input_with_turn)?;

                // After input processing, schedule reason activity
                store
                    .enqueue_task(
                        workflow_id,
                        format!("reason_{}", Uuid::now_v7()),
                        "reason".to_string(),
                        input_json,
                    )
                    .await
                    .map_err(|e| anyhow::anyhow!("Failed to enqueue reason task: {}", e))?;

                debug!(workflow_id = %workflow_id, turn_id = ?turn_id, "Scheduled reason activity");
            }
            "reason" => {
                // After reasoning, check if there are tool calls
                let reason_result: ReasonResult = serde_json::from_value(output.clone())
                    .map_err(|e| anyhow::anyhow!("Failed to parse ReasonResult: {}", e))?;

                // Carry response_id forward for next reason iteration
                chained_input.previous_response_id = reason_result.response_id.clone();

                if reason_result.has_tool_calls && reason_result.success {
                    // Schedule act activity to execute the tool calls
                    let tool_count = reason_result.tool_calls.len();

                    // Use turn_id from input (propagated from input activity)
                    let turn_id = input.turn_id.unwrap_or_default();

                    let act_task_input = ActTaskInput {
                        org_id: input.org_id,
                        act_input: ActInput {
                            org_id: Some(input.org_id),
                            context: AtomContext {
                                session_id: input.session_id,
                                turn_id,
                                input_message_id: input.input_message_id,
                                exec_id: ExecId::new(),
                            },
                            harness_id: input.harness_id,
                            agent_id: input.agent_id,
                            tool_calls: reason_result.tool_calls,
                            tool_definitions: reason_result.tool_definitions,
                        },
                    };
                    let mut act_input_json = serde_json::to_value(&act_task_input)?;
                    // Carry response_id and iteration through act task for next reason iteration
                    if let Some(rid) = &chained_input.previous_response_id {
                        act_input_json["previous_response_id"] = serde_json::json!(rid);
                    }
                    act_input_json["iteration"] = serde_json::json!(input.iteration);

                    store
                        .enqueue_task(
                            workflow_id,
                            format!("act_{}", Uuid::now_v7()),
                            "act".to_string(),
                            act_input_json,
                        )
                        .await
                        .map_err(|e| anyhow::anyhow!("Failed to enqueue act task: {}", e))?;

                    debug!(
                        workflow_id = %workflow_id,
                        tool_count = tool_count,
                        "Scheduled act activity for tool execution"
                    );
                } else {
                    // No tool calls or failure - workflow complete
                    store
                        .update_workflow_status(workflow_id, WorkflowStatus::Completed, None, None)
                        .await
                        .map_err(|e| anyhow::anyhow!("Failed to update workflow status: {}", e))?;

                    info!(workflow_id = %workflow_id, "Workflow completed");
                }
            }
            "act" => {
                let blocked = output
                    .get("blocked")
                    .and_then(|value| value.as_bool())
                    .unwrap_or(false);
                if blocked {
                    store
                        .update_workflow_status(workflow_id, WorkflowStatus::Completed, None, None)
                        .await
                        .map_err(|e| anyhow::anyhow!("Failed to update workflow status: {}", e))?;

                    info!(workflow_id = %workflow_id, "Workflow completed after dependency block");
                    return Ok(());
                }

                // Check if act needs external input (connection setup, client-side tools).
                // ActAtom sets waiting_for_tool_results via hooks; worker just checks the flag.
                let waiting = output
                    .get("waiting_for_tool_results")
                    .and_then(|v| v.as_bool())
                    .unwrap_or(false);

                if waiting {
                    // Complete workflow and persist the current DurableTurnInput so
                    // the tool-results endpoint can resume with correct turn_id,
                    // iteration, and previous_response_id (instead of creating a
                    // phantom MessageId that InputAtom cannot find).
                    let resumed_input = DurableTurnInput {
                        iteration: chained_input.iteration.saturating_add(1),
                        ..chained_input.clone()
                    };
                    let result_json = serde_json::to_value(&resumed_input)
                        .map_err(|e| {
                            anyhow::anyhow!(
                                "Failed to serialize DurableTurnInput for connection_required resume: {}",
                                e
                            )
                        })?;
                    store
                        .update_workflow_status(
                            workflow_id,
                            WorkflowStatus::Completed,
                            Some(result_json),
                            None,
                        )
                        .await
                        .map_err(|e| anyhow::anyhow!("Failed to update workflow status: {}", e))?;

                    info!(
                        workflow_id = %workflow_id,
                        "Workflow completed — waiting for user to set up connection"
                    );
                    return Ok(());
                }

                // After action, schedule another reason activity (continue the loop)
                // Use chained_input which carries previous_response_id from last reason
                // Increment iteration count for the next reason step
                chained_input.iteration = chained_input.iteration.saturating_add(1);
                let chained_json = serde_json::to_value(&chained_input)?;
                store
                    .enqueue_task(
                        workflow_id,
                        format!("reason_{}", Uuid::now_v7()),
                        "reason".to_string(),
                        chained_json,
                    )
                    .await
                    .map_err(|e| anyhow::anyhow!("Failed to enqueue reason task: {}", e))?;

                debug!(workflow_id = %workflow_id, "Scheduled reason activity after act");
            }
            _ => {
                warn!(
                    activity = completed_activity,
                    "Unknown activity type completed"
                );
            }
        }

        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

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
    fn test_is_non_retryable_task_error_allows_transient_errors() {
        assert!(!is_non_retryable_task_error(
            "Transient LLM error (will retry at task level): server_error"
        ));
        assert!(!is_non_retryable_task_error("Failed to connect to gRPC"));
        assert!(!is_non_retryable_task_error(
            "LLM provider temporarily unavailable (circuit breaker open)"
        ));
    }
}
