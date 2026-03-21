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
use crate::session_lifecycle::SessionLifecycle;
use crate::worker_adapters::{
    AdapterAgentStore, AdapterEventEmitter, AdapterHarnessStore, AdapterImageResolver,
    AdapterLlmProviderStore, AdapterMessageRetriever, AdapterSessionFileStore,
    AdapterSessionMutator, AdapterSessionStore, WorkerAdapters,
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
                "leased_resource_cleanup".to_string(),
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
            .unwrap_or(1000);

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

/// Helper: detect dependency blockers using shared core logic.
async fn detect_dependency_blocker<A: WorkerAdapters>(
    adapters: &A,
    org_id: i64,
    harness_id: everruns_core::HarnessId,
    agent_id: Option<everruns_core::AgentId>,
) -> Result<Option<everruns_core::DependencyBlocker>> {
    let harness_store = AdapterHarnessStore::new(adapters.clone(), org_id);
    let agent_store = AdapterAgentStore::new(adapters.clone(), org_id);
    Ok(
        everruns_core::detect_dependency_blocker(
            &harness_store,
            &agent_store,
            harness_id,
            agent_id,
        )
        .await?,
    )
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
            let _ = store.fail_task(task.id, "Workflow cancelled").await;
            return Ok(());
        }
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

            // Create DurableTurnInput from ActInput context
            let turn_input = DurableTurnInput {
                org_id: act_input
                    .org_id
                    .expect("ActInput.org_id must be set for durable turns"),
                session_id: act_input.context.session_id,
                harness_id: act_input.harness_id,
                agent_id: act_input.agent_id,
                input_message_id: act_input.context.input_message_id,
                turn_id: Some(act_input.context.turn_id),
                previous_response_id,
                iteration,
            };

            let res = execute_act_activity(adapters, &act_input).await;
            (res, Some(turn_input))
        }
        "leased_resource_cleanup" => {
            let cleanup_input: crate::leased_resource_cleanup::LeasedResourceCleanupInput =
                serde_json::from_value(task.input.clone())
                    .map_err(|e| anyhow::anyhow!("Failed to parse cleanup input: {}", e))?;
            let res =
                crate::leased_resource_cleanup::execute_cleanup_activity(adapters, &cleanup_input)
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

                    // After act activity: if ActAtom signaled waiting_for_tool_results
                    // (connection setup, client-side tools), check the setup_connection
                    // client hint before pausing. When the hint is true, set the session
                    // status so the client can handle the tool calls. When absent, skip
                    // the pause — the LLM will see the tool errors and inform the user.
                    if task.activity_type == "act"
                        && let Some(ref ti) = turn_input_opt
                    {
                        let act_result: Option<everruns_core::ActResult> =
                            serde_json::from_value(output.clone()).ok();
                        let waiting = act_result
                            .as_ref()
                            .is_some_and(|r| r.waiting_for_tool_results);
                        if waiting {
                            // Check the setup_connection hint before pausing
                            let hint_enabled =
                                match adapters.get_session(ti.org_id, ti.session_id.uuid()).await {
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
                                };

                            if hint_enabled {
                                let lifecycle = SessionLifecycle::new(
                                    adapters.clone(),
                                    ti.org_id,
                                    ti.session_id,
                                );
                                lifecycle.waiting_for_tool_results().await;
                            } else {
                                info!(
                                    "setup_connection hint absent; skipping wait for tool results"
                                );
                            }
                        }
                    }

                    // Schedule next activity if needed (only for workflow-bound tasks)
                    if let (Some(turn_input), Some(wf_id)) = (turn_input_opt, task.workflow_id) {
                        schedule_next_activity(
                            store,
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

    // Turn starting: set active, emit session.activated + turn.started
    let lifecycle = SessionLifecycle::new(adapters.clone(), input.org_id, input.session_id);
    lifecycle
        .turn_started(context.turn_id, input.input_message_id, None)
        .await;

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

    let lifecycle = SessionLifecycle::new(adapters.clone(), input.org_id, session_id);

    if let Some(blocker) =
        detect_dependency_blocker(adapters, input.org_id, input.harness_id, input.agent_id).await?
    {
        lifecycle
            .dependency_blocked(turn_id, input_message_id, blocker)
            .await;
        return Ok(serde_json::to_value(everruns_core::ReasonResult {
            success: false,
            text: blocker.message().to_string(),
            tool_calls: vec![],
            has_tool_calls: false,
            tool_definitions: vec![],
            max_iterations: 100,
            error: Some("dependency_unavailable".to_string()),
            usage: None,
            response_id: None,
            locale: None,
        })?);
    }

    // Load turn context (batch call for efficiency)
    let turn_context = adapters
        .load_turn_context(input.org_id, session_id.uuid())
        .await?;

    // Create atom dependencies
    let harness_store = AdapterHarnessStore::new(adapters.clone(), input.org_id);
    let agent_store = AdapterAgentStore::new(adapters.clone(), input.org_id);
    let session_store = AdapterSessionStore::new(adapters.clone(), input.org_id);
    let message_retriever = AdapterMessageRetriever::new(adapters.clone());
    let provider_store = AdapterLlmProviderStore::new(adapters.clone(), input.org_id);
    let capability_registry = adapters.capability_registry();
    let driver_registry = adapters.driver_registry();
    let event_emitter = AdapterEventEmitter::new(adapters.clone());
    let image_resolver = Arc::new(AdapterImageResolver::new(adapters.clone(), input.org_id));

    let atom = ReasonAtom::new(
        harness_store,
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
        harness_id: input.harness_id,
        agent_id: input.agent_id,
        org_id: input.org_id,
        mcp_tool_definitions: turn_context.mcp_tool_definitions,
        previous_response_id: input.previous_response_id.clone(),
        iteration: input.iteration,
    };

    let result = atom.execute(reason_input).await?;

    // If turn is complete (no tool calls or failure), emit lifecycle events.
    // Client-side tool handling is now done by ActAtom's hooks.
    let turn_complete = !result.has_tool_calls || !result.success;
    if turn_complete {
        if !result.success {
            lifecycle
                .turn_failed(
                    turn_id,
                    input_message_id,
                    "An error occurred while processing your request.",
                    Some("llm_error"),
                )
                .await;
        } else {
            lifecycle
                .turn_completed(
                    turn_id,
                    input_message_id,
                    input.iteration,
                    result.usage.clone(),
                    None,
                )
                .await;
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

    // Extract org_id early — must be set by callers for proper tenant isolation.
    let org_id = input
        .org_id
        .expect("ActInput.org_id must be set for act activities");

    if let Some(blocker) =
        detect_dependency_blocker(adapters, org_id, input.harness_id, input.agent_id).await?
    {
        let lifecycle = SessionLifecycle::new(adapters.clone(), org_id, input.context.session_id);
        lifecycle
            .dependency_blocked(
                input.context.turn_id,
                input.context.input_message_id,
                blocker,
            )
            .await;
        let output = serde_json::to_value(everruns_core::ActResult {
            results: vec![],
            completed: true,
            success_count: 0,
            error_count: 1,
            waiting_for_tool_results: false,
            blocked: true,
            client_tool_calls: vec![],
            client_tool_definitions: vec![],
        })?;
        return Ok(output);
    }

    // Build tool registry with defaults and capability tools.
    // When agent_id is present, use agent capabilities.
    // When agent_id is absent, fall back to harness capabilities so that
    // harness-provided tools (e.g. bash) are still registered.
    let tool_registry = if let Some(agent_id) = input.agent_id {
        adapters
            .build_tool_registry(org_id, agent_id.uuid())
            .await?
    } else {
        adapters
            .build_tool_registry_for_harness(org_id, input.harness_id.uuid())
            .await?
    };

    let event_emitter = AdapterEventEmitter::new(adapters.clone());
    let file_store = Arc::new(AdapterSessionFileStore::new(adapters.clone()));
    let session_store = Arc::new(AdapterSessionStore::new(adapters.clone(), org_id));
    let session_mutator = Arc::new(AdapterSessionMutator::new(adapters.clone(), org_id));
    let agent_store = Arc::new(AdapterAgentStore::new(adapters.clone(), org_id));

    let mut atom = ActAtom::with_file_store(tool_registry, event_emitter, file_store)
        .with_session_store(session_store)
        .with_session_mutator(session_mutator)
        .with_agent_store(agent_store);
    atom = atom
        .with_sqldb_store(adapters.sqldb_store())
        .with_storage_store(adapters.storage_store())
        .with_connection_resolver(adapters.connection_resolver())
        .with_leased_resource_store(adapters.leased_resource_store())
        .with_schedule_store(adapters.schedule_store(org_id))
        .with_platform_store(adapters.platform_store(org_id));

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
                harness_id: input.harness_id,
                agent_id: input.agent_id,
                input_message_id: input.input_message_id,
                turn_id,
                previous_response_id: None,
                iteration: 1,
            };
            let input_json = serde_json::to_value(&input_with_turn)?;

            // Schedule reason activity
            let activity_id = format!("reason_{}", Uuid::now_v7());
            let task = TaskDefinition {
                workflow_id: Some(workflow_id),
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
            let reason_result: everruns_core::ReasonResult = serde_json::from_value(output.clone())
                .map_err(|e| anyhow::anyhow!("Failed to parse ReasonResult: {}", e))?;

            let response_id = reason_result.response_id.clone();

            if reason_result.has_tool_calls && reason_result.success {
                let turn_id = input.turn_id.unwrap_or_default();

                // Send ALL tool calls to ActAtom — it handles client/server partitioning internally
                let act_input = ActInput {
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
                    locale: reason_result.locale,
                };
                let mut act_input_json = serde_json::to_value(&act_input)?;
                if let Some(rid) = &response_id {
                    act_input_json["previous_response_id"] = serde_json::json!(rid);
                }
                act_input_json["iteration"] = serde_json::json!(input.iteration);
                let activity_id = format!("act_{}", Uuid::now_v7());

                let task = TaskDefinition {
                    workflow_id: Some(workflow_id),
                    activity_id: activity_id.clone(),
                    activity_type: "act".to_string(),
                    input: act_input_json.clone(),
                    options: Default::default(),
                };
                store
                    .enqueue_task(task)
                    .await
                    .map_err(|e| anyhow::anyhow!("Failed to enqueue act task: {}", e))?;

                let scheduled_event = WorkflowEvent::ActivityScheduled {
                    activity_id,
                    activity_type: "act".to_string(),
                    input: act_input_json,
                    options: ActivityOptions::default(),
                };
                let _ = append_event(store.as_ref(), workflow_id, scheduled_event).await;

                debug!(workflow_id = %workflow_id, "Scheduled act activity");
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
            let blocked = output
                .get("blocked")
                .and_then(|value| value.as_bool())
                .unwrap_or(false);
            if blocked {
                record_workflow_completed(store.as_ref(), workflow_id, output.clone()).await;
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
                // Complete workflow — it will be resumed by tool-results endpoint
                record_workflow_completed(store.as_ref(), workflow_id, output.clone()).await;
                store
                    .update_workflow_status(workflow_id, WorkflowStatus::Completed, None, None)
                    .await
                    .map_err(|e| anyhow::anyhow!("Failed to update workflow status: {}", e))?;

                info!(
                    workflow_id = %workflow_id,
                    "Workflow completed — waiting for tool results"
                );
            } else {
                // Normal flow: schedule another reason activity
                // Increment iteration count for the next reason step
                let mut next_input = input.clone();
                next_input.iteration = next_input.iteration.saturating_add(1);
                let input_json = serde_json::to_value(&next_input)?;
                let activity_id = format!("reason_{}", Uuid::now_v7());
                let task = TaskDefinition {
                    workflow_id: Some(workflow_id),
                    activity_id: activity_id.clone(),
                    activity_type: "reason".to_string(),
                    input: input_json.clone(),
                    options: Default::default(),
                };
                store
                    .enqueue_task(task)
                    .await
                    .map_err(|e| anyhow::anyhow!("Failed to enqueue reason task: {}", e))?;

                let scheduled_event = WorkflowEvent::ActivityScheduled {
                    activity_id,
                    activity_type: "reason".to_string(),
                    input: input_json,
                    options: ActivityOptions::default(),
                };
                let _ = append_event(store.as_ref(), workflow_id, scheduled_event).await;

                debug!(workflow_id = %workflow_id, turn_id = ?input.turn_id, "Scheduled reason activity after act");
            }
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
