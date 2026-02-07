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
use everruns_core::ActInput;
use everruns_core::atoms::{ActAtom, Atom, AtomContext, InputAtom, ReasonAtom};
use everruns_core::events::{
    EventContext, EventRequest, SessionActivatedData, SessionIdledData, TurnCompletedData,
    TurnFailedData, TurnStartedData,
};
use everruns_core::typed_id::{ExecId, TurnId};
use everruns_durable::{
    ActivityOptions, ClaimedTask, WorkerInfo, WorkflowEvent, WorkflowEventStore, WorkflowStatus,
    append_event, record_activity_completed, record_activity_failed, record_activity_started,
    record_workflow_completed,
};
use std::sync::Arc;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::time::Duration;
use tokio::sync::watch;
use tracing::{debug, error, info, warn};
use uuid::Uuid;

use crate::durable_runner::DurableTurnInput;
use crate::worker_adapters::{
    AdapterAgentStore, AdapterEventEmitter, AdapterImageResolver, AdapterLlmProviderStore,
    AdapterMessageRetriever, AdapterSessionFileStore, AdapterSessionStore, WorkerAdapters,
};

// Re-export atom types
pub use everruns_core::atoms::{InputAtomInput, ReasonInput, ReasonResult};

// =============================================================================
// Configuration
// =============================================================================

/// Configuration for the task worker
#[derive(Debug, Clone)]
pub struct TaskWorkerConfig {
    /// Worker ID (unique identifier for this worker instance)
    pub worker_id: String,
    /// Activity types this worker handles
    pub activity_types: Vec<String>,
    /// Maximum concurrent tasks
    pub max_concurrent_tasks: usize,
    /// Poll interval when no tasks available
    pub poll_interval: Duration,
    /// Heartbeat interval for worker registration
    pub heartbeat_interval: Duration,
    /// Worker group name (optional, for routing)
    pub worker_group: Option<String>,
}

impl Default for TaskWorkerConfig {
    fn default() -> Self {
        Self {
            worker_id: format!("worker-{}", Uuid::now_v7()),
            activity_types: vec![
                "process_input".to_string(),
                "reason".to_string(),
                "act".to_string(),
            ],
            max_concurrent_tasks: 10,
            poll_interval: Duration::from_millis(100),
            heartbeat_interval: Duration::from_secs(10),
            worker_group: None,
        }
    }
}

impl TaskWorkerConfig {
    /// Create dev mode configuration (faster polling, lower concurrency)
    pub fn dev_mode() -> Self {
        Self {
            worker_id: format!("dev-worker-{}", Uuid::now_v7()),
            worker_group: Some("dev".to_string()),
            max_concurrent_tasks: 10,
            poll_interval: Duration::from_millis(100),
            ..Default::default()
        }
    }

    /// Create production configuration (higher concurrency)
    pub fn production() -> Self {
        Self {
            max_concurrent_tasks: 1000,
            poll_interval: Duration::from_millis(100),
            ..Default::default()
        }
    }

    /// Create configuration from environment variables
    pub fn from_env() -> Self {
        let worker_id =
            std::env::var("WORKER_ID").unwrap_or_else(|_| format!("worker-{}", Uuid::now_v7()));

        let max_concurrent = std::env::var("MAX_CONCURRENT_TASKS")
            .ok()
            .and_then(|s| s.parse().ok())
            .unwrap_or(10);

        let worker_group = std::env::var("WORKER_GROUP").ok();

        Self {
            worker_id,
            max_concurrent_tasks: max_concurrent,
            worker_group,
            ..Default::default()
        }
    }
}

// =============================================================================
// Unified Worker
// =============================================================================

/// Unified worker that executes tasks from the durable task queue
///
/// This worker is generic over:
/// - `S`: WorkflowEventStore implementation (InMemory or Postgres)
/// - `A`: WorkerAdapters implementation (Direct or gRPC)
pub struct TaskWorker<S, A>
where
    S: WorkflowEventStore,
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
    S: WorkflowEventStore,
    A: WorkerAdapters,
{
    /// Create a new unified worker
    pub fn new(config: TaskWorkerConfig, store: Arc<S>, adapters: A) -> Self {
        let (shutdown_tx, shutdown_rx) = watch::channel(false);

        info!(
            worker_id = %config.worker_id,
            max_concurrent = config.max_concurrent_tasks,
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

        tokio::spawn(async move {
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

        // Main poll loop
        loop {
            if *self.shutdown_rx.borrow() {
                info!("Shutdown signal received, stopping worker");
                break;
            }

            match self.poll_and_execute().await {
                Ok(executed) => {
                    if executed == 0 {
                        tokio::select! {
                            _ = tokio::time::sleep(self.config.poll_interval) => {}
                            _ = self.shutdown_rx.changed() => {
                                info!("Shutdown during poll wait");
                                break;
                            }
                        }
                    }
                }
                Err(e) => {
                    error!("Error polling tasks: {}", e);
                    tokio::time::sleep(Duration::from_secs(1)).await;
                }
            }
        }

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

    /// Poll for tasks and execute them
    async fn poll_and_execute(&self) -> Result<usize> {
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

        let tasks = self
            .store
            .claim_task(
                &self.config.worker_id,
                &self.config.activity_types,
                available_slots,
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
            self.in_flight.fetch_add(1, Ordering::SeqCst);

            let store = self.store.clone();
            let adapters = self.adapters.clone();
            let worker_id = self.config.worker_id.clone();
            let in_flight = self.in_flight.clone();

            tokio::spawn(async move {
                let result = execute_task(&store, &adapters, &worker_id, &task).await;

                if let Err(e) = result {
                    error!(
                        task_id = %task.id,
                        activity_type = %task.activity_type,
                        error = %e,
                        "Task execution failed"
                    );

                    let _ = store.fail_task(task.id, &e.to_string()).await;
                }

                in_flight.fetch_sub(1, Ordering::SeqCst);
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

// =============================================================================
// Task Execution
// =============================================================================

/// Execute a single task
async fn execute_task<S, A>(
    store: &Arc<S>,
    adapters: &A,
    worker_id: &str,
    task: &ClaimedTask,
) -> Result<()>
where
    S: WorkflowEventStore,
    A: WorkerAdapters,
{
    info!(
        task_id = %task.id,
        workflow_id = %task.workflow_id,
        activity_type = %task.activity_type,
        attempt = task.attempt,
        "Executing task"
    );

    // Check if workflow is cancelled
    let workflow_status = store.get_workflow_status(task.workflow_id).await;
    if let Ok(status) = workflow_status
        && status == WorkflowStatus::Cancelled
    {
        info!(
            task_id = %task.id,
            workflow_id = %task.workflow_id,
            "Workflow cancelled, skipping task"
        );
        let _ = store.fail_task(task.id, "Workflow cancelled").await;
        return Ok(());
    }

    // Record ActivityStarted event
    record_activity_started(
        store.as_ref(),
        task.workflow_id,
        task.activity_id.clone(),
        task.attempt,
        worker_id.to_string(),
    )
    .await;

    // Execute based on activity type
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

            // Create DurableTurnInput from ActInput context
            let turn_input = DurableTurnInput {
                org_id: everruns_core::DEFAULT_ORG_ID,
                session_id: act_input.context.session_id,
                agent_id: act_input.agent_id,
                input_message_id: act_input.context.input_message_id,
                turn_id: Some(act_input.context.turn_id),
            };

            let res = execute_act_activity(adapters, &act_input).await;
            (res, Some(turn_input))
        }
        _ => (
            Err(anyhow::anyhow!(
                "Unknown activity type: {}",
                task.activity_type
            )),
            None,
        ),
    };

    match result {
        Ok(output) => {
            // Record ActivityCompleted event
            record_activity_completed(
                store.as_ref(),
                task.workflow_id,
                task.activity_id.clone(),
                output.clone(),
            )
            .await;

            // Complete the task - verify ownership
            let complete_result = store
                .complete_task(task.id, worker_id, output.clone())
                .await;

            match complete_result {
                Ok(()) => {
                    info!(
                        task_id = %task.id,
                        activity_type = %task.activity_type,
                        "Task completed successfully"
                    );

                    // Schedule next activity if needed
                    if let Some(turn_input) = turn_input_opt {
                        schedule_next_activity(
                            store,
                            task.workflow_id,
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
            let will_retry = task.attempt < task.max_attempts;
            record_activity_failed(
                store.as_ref(),
                task.workflow_id,
                task.activity_id.clone(),
                e.to_string(),
                will_retry,
            )
            .await;

            return Err(e);
        }
    }

    Ok(())
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
        session_id: input.session_id,
        turn_id: TurnId::new(),
        input_message_id: input.input_message_id,
        exec_id: ExecId::new(),
    };

    // Set session status to "active"
    if let Err(e) = adapters
        .set_session_status(input.org_id, input.session_id.uuid(), "active")
        .await
    {
        warn!(error = %e, "Failed to set session status to active");
    }

    // Emit session.activated event
    let activated_event = EventRequest::new(
        input.session_id,
        EventContext::turn(context.turn_id, input.input_message_id),
        SessionActivatedData {
            turn_id: context.turn_id,
            input_message_id: input.input_message_id,
        },
    );
    if let Err(e) = adapters.emit_event(activated_event).await {
        warn!(error = %e, "Failed to emit session.activated event");
    }

    // Emit turn.started event
    let turn_started_event = EventRequest::new(
        input.session_id,
        EventContext::turn(context.turn_id, input.input_message_id),
        TurnStartedData {
            turn_id: context.turn_id,
            input_message_id: input.input_message_id,
            input_content: None, // Content will be extracted by Braintrust listener
        },
    );
    if let Err(e) = adapters.emit_event(turn_started_event).await {
        warn!(error = %e, "Failed to emit turn.started event");
    }

    // Execute InputAtom
    let message_retriever = AdapterMessageRetriever::new(adapters.clone());
    let atom = InputAtom::new(message_retriever);

    let atom_input = everruns_core::InputAtomInput {
        context: context.clone(),
    };
    let result = atom.execute(atom_input).await?;

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
        session_id: input.session_id,
        turn_id,
        input_message_id: input.input_message_id,
        exec_id: ExecId::new(),
    };

    let session_id = input.session_id;
    let input_message_id = input.input_message_id;

    // Load turn context (batch call for efficiency)
    let turn_context = adapters
        .load_turn_context(input.org_id, session_id.uuid())
        .await?;

    // Create atom dependencies
    let agent_store = AdapterAgentStore::new(adapters.clone(), input.org_id);
    let session_store = AdapterSessionStore::new(adapters.clone(), input.org_id);
    let message_retriever = AdapterMessageRetriever::new(adapters.clone());
    let provider_store = AdapterLlmProviderStore::new(adapters.clone(), input.org_id);
    let capability_registry = adapters.capability_registry();
    let driver_registry = adapters.driver_registry();
    let event_emitter = AdapterEventEmitter::new(adapters.clone());
    let image_resolver = Arc::new(AdapterImageResolver::new(adapters.clone()));

    let atom = ReasonAtom::new(
        agent_store,
        session_store,
        message_retriever,
        provider_store,
        capability_registry,
        driver_registry,
        event_emitter,
    )
    .with_image_resolver(image_resolver);

    let reason_input = everruns_core::ReasonInput {
        context: context.clone(),
        agent_id: input.agent_id,
        org_id: input.org_id,
        mcp_tool_definitions: turn_context.mcp_tool_definitions,
    };

    let result = atom.execute(reason_input).await?;

    // If turn is complete (no tool calls or failure), set session to idle
    let turn_complete = !result.has_tool_calls || !result.success;
    if turn_complete {
        if let Err(e) = adapters
            .set_session_status(input.org_id, session_id.uuid(), "idle")
            .await
        {
            warn!(error = %e, "Failed to set session status to idle");
        }

        // Emit turn.failed or turn.completed
        if !result.success {
            let turn_failed_event = EventRequest::new(
                session_id,
                EventContext::turn(turn_id, input_message_id),
                TurnFailedData {
                    turn_id,
                    error: "An error occurred while processing your request.".to_string(),
                    error_code: Some("llm_error".to_string()),
                },
            );
            if let Err(e) = adapters.emit_event(turn_failed_event).await {
                warn!(error = %e, "Failed to emit turn.failed event");
            }
        } else {
            let turn_completed_event = EventRequest::new(
                session_id,
                EventContext::turn(turn_id, input_message_id),
                TurnCompletedData {
                    turn_id,
                    iterations: 1,
                    duration_ms: None,
                    usage: result.usage.clone(),
                    input_content: None, // Passed through from turn.started by Braintrust
                },
            );
            if let Err(e) = adapters.emit_event(turn_completed_event).await {
                warn!(error = %e, "Failed to emit turn.completed event");
            }
        }

        // Emit session.idled event
        let idled_event = EventRequest::new(
            session_id,
            EventContext::turn(turn_id, input_message_id),
            SessionIdledData {
                turn_id,
                iterations: None,
                usage: result.usage.clone(),
            },
        );
        if let Err(e) = adapters.emit_event(idled_event).await {
            warn!(error = %e, "Failed to emit session.idled event");
        }
    }

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

    // Build tool registry with defaults and capability tools
    let tool_registry = adapters.build_tool_registry(input.agent_id.uuid()).await?;

    let event_emitter = AdapterEventEmitter::new(adapters.clone());
    let file_store = Arc::new(AdapterSessionFileStore::new(adapters.clone()));

    let mut atom = ActAtom::with_file_store(tool_registry, event_emitter, file_store);
    if let Some(sqldb_store) = adapters.sqldb_store() {
        atom = atom.with_sqldb_store(sqldb_store);
    }

    let result = atom.execute(input.clone()).await?;

    Ok(serde_json::to_value(&result)?)
}

// =============================================================================
// Activity Scheduling
// =============================================================================

/// Schedule the next activity based on current activity completion
async fn schedule_next_activity<S: WorkflowEventStore>(
    store: &Arc<S>,
    workflow_id: Uuid,
    completed_activity: &str,
    input: &DurableTurnInput,
    output: &serde_json::Value,
) -> Result<()> {
    use everruns_durable::TaskDefinition;

    match completed_activity {
        "process_input" => {
            // Extract turn_id from output
            let turn_id: Option<TurnId> = output
                .get("turn_id")
                .and_then(|v| v.as_str())
                .and_then(|s| s.parse().ok());

            // Create input with turn_id for subsequent activities
            let input_with_turn = DurableTurnInput {
                org_id: input.org_id,
                session_id: input.session_id,
                agent_id: input.agent_id,
                input_message_id: input.input_message_id,
                turn_id,
            };
            let input_json = serde_json::to_value(&input_with_turn)?;

            // Schedule reason activity
            let activity_id = format!("reason_{}", Uuid::now_v7());
            let task = TaskDefinition {
                workflow_id,
                activity_id: activity_id.clone(),
                activity_type: "reason".to_string(),
                input: input_json.clone(),
                options: Default::default(),
            };
            store
                .enqueue_task(task)
                .await
                .map_err(|e| anyhow::anyhow!("Failed to enqueue reason task: {}", e))?;

            // Record ActivityScheduled event
            let scheduled_event = WorkflowEvent::ActivityScheduled {
                activity_id,
                activity_type: "reason".to_string(),
                input: input_json,
                options: ActivityOptions::default(),
            };
            let _ = append_event(store.as_ref(), workflow_id, scheduled_event).await;

            debug!(workflow_id = %workflow_id, turn_id = ?turn_id, "Scheduled reason activity");
        }
        "reason" => {
            // Check if there are tool calls
            let reason_result: everruns_core::ReasonResult = serde_json::from_value(output.clone())
                .map_err(|e| anyhow::anyhow!("Failed to parse ReasonResult: {}", e))?;

            if reason_result.has_tool_calls && reason_result.success {
                let tool_count = reason_result.tool_calls.len();
                let turn_id = input.turn_id.unwrap_or_default();

                // Schedule act activity
                let act_input = ActInput {
                    context: AtomContext {
                        session_id: input.session_id,
                        turn_id,
                        input_message_id: input.input_message_id,
                        exec_id: ExecId::new(),
                    },
                    agent_id: input.agent_id,
                    tool_calls: reason_result.tool_calls,
                    tool_definitions: reason_result.tool_definitions,
                };
                let act_input_json = serde_json::to_value(&act_input)?;
                let activity_id = format!("act_{}", Uuid::now_v7());

                let task = TaskDefinition {
                    workflow_id,
                    activity_id: activity_id.clone(),
                    activity_type: "act".to_string(),
                    input: act_input_json.clone(),
                    options: Default::default(),
                };
                store
                    .enqueue_task(task)
                    .await
                    .map_err(|e| anyhow::anyhow!("Failed to enqueue act task: {}", e))?;

                // Record ActivityScheduled event
                let scheduled_event = WorkflowEvent::ActivityScheduled {
                    activity_id,
                    activity_type: "act".to_string(),
                    input: act_input_json,
                    options: ActivityOptions::default(),
                };
                let _ = append_event(store.as_ref(), workflow_id, scheduled_event).await;

                debug!(
                    workflow_id = %workflow_id,
                    tool_count = tool_count,
                    "Scheduled act activity"
                );
            } else {
                // Workflow complete
                record_workflow_completed(store.as_ref(), workflow_id, output.clone()).await;

                store
                    .update_workflow_status(workflow_id, WorkflowStatus::Completed, None, None)
                    .await
                    .map_err(|e| anyhow::anyhow!("Failed to update workflow status: {}", e))?;

                info!(workflow_id = %workflow_id, "Workflow completed");
            }
        }
        "act" => {
            let input_json = serde_json::to_value(input)?;

            // Schedule another reason activity
            let activity_id = format!("reason_{}", Uuid::now_v7());
            let task = TaskDefinition {
                workflow_id,
                activity_id: activity_id.clone(),
                activity_type: "reason".to_string(),
                input: input_json.clone(),
                options: Default::default(),
            };
            store
                .enqueue_task(task)
                .await
                .map_err(|e| anyhow::anyhow!("Failed to enqueue reason task: {}", e))?;

            // Record ActivityScheduled event
            let scheduled_event = WorkflowEvent::ActivityScheduled {
                activity_id,
                activity_type: "reason".to_string(),
                input: input_json,
                options: ActivityOptions::default(),
            };
            let _ = append_event(store.as_ref(), workflow_id, scheduled_event).await;

            debug!(workflow_id = %workflow_id, turn_id = ?input.turn_id, "Scheduled reason activity after act");
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_config_default() {
        let config = TaskWorkerConfig::default();
        assert!(config.worker_id.starts_with("worker-"));
        assert_eq!(config.max_concurrent_tasks, 10);
    }

    #[test]
    fn test_config_dev_mode() {
        let config = TaskWorkerConfig::dev_mode();
        assert!(config.worker_id.starts_with("dev-worker-"));
        assert_eq!(config.worker_group, Some("dev".to_string()));
    }

    #[test]
    fn test_config_production() {
        let config = TaskWorkerConfig::production();
        assert_eq!(config.max_concurrent_tasks, 1000);
    }
}
