// In-process worker for DEV_MODE
//
// Decision: Runs task execution in-process with the control-plane for DEV_MODE
// Decision: Uses direct adapters instead of gRPC for all operations
//
// This worker polls the InMemoryWorkflowEventStore for tasks and executes them
// using direct access to the storage backend.

use anyhow::Result;
use everruns_core::ToolRegistry;
use everruns_core::atoms::{ActAtom, Atom, AtomContext, InputAtom, ReasonAtom};
use everruns_core::capabilities::CapabilityRegistry;
use everruns_core::{ActInput, InputAtomInput, ReasonInput, ReasonResult, TokenUsage};
use everruns_durable::{
    ActivityOptions, ClaimedTask, InMemoryWorkflowEventStore, WorkerInfo, WorkflowEvent,
    WorkflowEventStore, WorkflowStatus, append_event, record_activity_completed,
    record_activity_failed, record_activity_started, record_workflow_completed,
};
use everruns_worker::{DurableTurnInput, create_driver_registry};
use std::sync::Arc;
use std::time::Duration;
use tokio::sync::watch;
use tracing::{debug, error, info, warn};
use uuid::Uuid;

use super::direct_adapters::{
    DirectAgentStore, DirectEventEmitter, DirectLlmProviderStore, DirectMessageRetriever,
    DirectSessionFileStore, DirectSessionStore, SessionStatusUpdater,
};
use crate::services::{EventService, LlmResolverService, McpServerService};
use crate::storage::StorageBackend;

/// Configuration for the in-process worker
#[derive(Debug, Clone)]
pub struct InProcessWorkerConfig {
    /// Worker ID
    pub worker_id: String,
    /// Activity types to handle
    pub activity_types: Vec<String>,
    /// Maximum concurrent tasks
    pub max_concurrent_tasks: usize,
    /// Poll interval when no tasks available
    pub poll_interval: Duration,
}

impl Default for InProcessWorkerConfig {
    fn default() -> Self {
        Self {
            worker_id: format!("dev-worker-{}", Uuid::now_v7()),
            activity_types: vec![
                "process_input".to_string(),
                "reason".to_string(),
                "act".to_string(),
            ],
            max_concurrent_tasks: 10,
            poll_interval: Duration::from_millis(100), // Fast polling for dev mode
        }
    }
}

/// In-process worker for DEV_MODE
///
/// This worker runs in the same process as the control-plane and uses direct
/// adapters to access storage and services.
pub struct InProcessWorker {
    config: InProcessWorkerConfig,
    durable_store: Arc<InMemoryWorkflowEventStore>,
    db: Arc<StorageBackend>,
    event_service: Arc<EventService>,
    llm_resolver: Arc<LlmResolverService>,
    mcp_server_service: Arc<McpServerService>,
    shutdown_tx: watch::Sender<bool>,
    shutdown_rx: watch::Receiver<bool>,
}

impl InProcessWorker {
    /// Create a new in-process worker
    pub fn new(
        config: InProcessWorkerConfig,
        durable_store: Arc<InMemoryWorkflowEventStore>,
        db: Arc<StorageBackend>,
        event_service: Arc<EventService>,
        llm_resolver: Arc<LlmResolverService>,
        mcp_server_service: Arc<McpServerService>,
    ) -> Self {
        let (shutdown_tx, shutdown_rx) = watch::channel(false);

        info!(
            worker_id = %config.worker_id,
            "Initialized in-process worker for DEV_MODE"
        );

        Self {
            config,
            durable_store,
            db,
            event_service,
            llm_resolver,
            mcp_server_service,
            shutdown_tx,
            shutdown_rx,
        }
    }

    /// Run the worker (blocking until shutdown)
    pub async fn run(&mut self) -> Result<()> {
        info!(
            worker_id = %self.config.worker_id,
            "Starting in-process worker"
        );

        // Register worker so it appears in the UI
        let worker_info = WorkerInfo {
            id: self.config.worker_id.clone(),
            worker_group: Some("dev".to_string()),
            activity_types: self.config.activity_types.clone(),
            max_concurrency: self.config.max_concurrent_tasks as u32,
            current_load: 0,
            status: "active".to_string(),
            accepting_tasks: true,
            started_at: chrono::Utc::now(),
            last_heartbeat_at: chrono::Utc::now(),
        };
        if let Err(e) = self.durable_store.register_worker(worker_info).await {
            warn!(error = %e, "Failed to register in-process worker (will continue anyway)");
        } else {
            info!(worker_id = %self.config.worker_id, "In-process worker registered");
        }

        // Spawn heartbeat task
        let heartbeat_store = self.durable_store.clone();
        let heartbeat_worker_id = self.config.worker_id.clone();
        let mut heartbeat_shutdown_rx = self.shutdown_rx.clone();
        tokio::spawn(async move {
            loop {
                tokio::select! {
                    _ = tokio::time::sleep(Duration::from_secs(10)) => {
                        if let Err(e) = heartbeat_store.worker_heartbeat(&heartbeat_worker_id, 0, true).await {
                            warn!(error = %e, "Failed to send heartbeat");
                        }
                    }
                    _ = heartbeat_shutdown_rx.changed() => {
                        break;
                    }
                }
            }
        });

        loop {
            // Check for shutdown
            if *self.shutdown_rx.borrow() {
                info!("Shutdown signal received, stopping worker");
                break;
            }

            // Poll for tasks
            match self.poll_and_execute().await {
                Ok(executed) => {
                    if executed == 0 {
                        // No tasks available, wait before next poll
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

        info!("In-process worker stopped");
        Ok(())
    }

    /// Signal the worker to shutdown
    #[allow(dead_code)]
    pub fn shutdown(&self) {
        let _ = self.shutdown_tx.send(true);
    }

    /// Poll for tasks and execute them
    async fn poll_and_execute(&self) -> Result<usize> {
        // Claim tasks from the in-memory store
        let tasks = self
            .durable_store
            .claim_task(
                &self.config.worker_id,
                &self.config.activity_types,
                self.config.max_concurrent_tasks,
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

        // Execute tasks sequentially (could be parallelized if needed)
        for task in &tasks {
            if let Err(e) = self.execute_task(task).await {
                error!(
                    task_id = %task.id,
                    activity_type = %task.activity_type,
                    error = %e,
                    "Task execution failed"
                );

                // Report failure to store
                let _ = self.durable_store.fail_task(task.id, &e.to_string()).await;
            }
        }

        Ok(tasks.len())
    }

    /// Execute a single task
    async fn execute_task(&self, task: &ClaimedTask) -> Result<()> {
        info!(
            task_id = %task.id,
            workflow_id = %task.workflow_id,
            activity_type = %task.activity_type,
            attempt = task.attempt,
            "Executing task"
        );

        // Record ActivityStarted event
        record_activity_started(
            self.durable_store.as_ref(),
            task.workflow_id,
            task.activity_id.clone(),
            task.attempt,
            self.config.worker_id.clone(),
        )
        .await;

        // Execute based on activity type
        let (result, turn_input_opt) = match task.activity_type.as_str() {
            "process_input" | "reason" => {
                // Parse DurableTurnInput
                let turn_input: DurableTurnInput = serde_json::from_value(task.input.clone())
                    .map_err(|e| anyhow::anyhow!("Failed to parse task input: {}", e))?;

                let res = match task.activity_type.as_str() {
                    "process_input" => self.execute_input_activity(&turn_input).await,
                    "reason" => self.execute_reason_activity(&turn_input).await,
                    _ => unreachable!(),
                };
                (res, Some(turn_input))
            }
            "act" => {
                // Parse ActInput
                let act_input: ActInput = serde_json::from_value(task.input.clone())
                    .map_err(|e| anyhow::anyhow!("Failed to parse ActInput: {}", e))?;

                // Create DurableTurnInput from ActInput context
                let turn_input = DurableTurnInput {
                    session_id: act_input.context.session_id,
                    agent_id: act_input.agent_id,
                    input_message_id: act_input.context.input_message_id,
                };

                let res = self.execute_act_activity(&act_input).await;
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
                    self.durable_store.as_ref(),
                    task.workflow_id,
                    task.activity_id.clone(),
                    output.clone(),
                )
                .await;

                // Complete the task - verify we still own it
                let complete_result = self
                    .durable_store
                    .complete_task(task.id, &self.config.worker_id, output.clone())
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
                            self.schedule_next_activity(
                                task.workflow_id,
                                &task.activity_type,
                                &turn_input,
                                &output,
                            )
                            .await?;
                        }
                    }
                    Err(e) => {
                        // Task was reclaimed - don't schedule next activity
                        warn!(
                            task_id = %task.id,
                            activity_type = %task.activity_type,
                            error = %e,
                            "Task completion rejected - skipping next activity"
                        );
                    }
                }
            }
            Err(e) => {
                // Record ActivityFailed event
                let will_retry = task.attempt < 3; // Default max attempts is 3
                record_activity_failed(
                    self.durable_store.as_ref(),
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

    /// Execute input processing activity
    async fn execute_input_activity(&self, input: &DurableTurnInput) -> Result<serde_json::Value> {
        use everruns_core::events::{
            EventContext, EventRequest, SessionActivatedData, TurnStartedData,
        };
        use everruns_core::traits::EventEmitter;

        debug!(
            session_id = %input.session_id,
            "Executing input activity"
        );

        // Create AtomContext
        let context = AtomContext {
            session_id: input.session_id,
            turn_id: Uuid::now_v7(),
            input_message_id: input.input_message_id,
            exec_id: Uuid::now_v7(),
        };

        // Set session status to "active"
        let status_updater = SessionStatusUpdater::new(self.db.clone());
        if let Err(e) = status_updater.set_status(input.session_id, "active").await {
            warn!(error = %e, "Failed to set session status to active");
        }

        // Emit session.activated event
        let event_emitter = DirectEventEmitter::new(self.event_service.clone());
        let activated_event = EventRequest::new(
            input.session_id,
            EventContext::turn(context.turn_id, input.input_message_id),
            SessionActivatedData {
                turn_id: context.turn_id,
                input_message_id: input.input_message_id,
            },
        );
        if let Err(e) = event_emitter.emit(activated_event).await {
            warn!(error = %e, "Failed to emit session.activated event");
        }

        // Emit turn.started event
        let turn_started_event = EventRequest::new(
            input.session_id,
            EventContext::turn(context.turn_id, input.input_message_id),
            TurnStartedData {
                turn_id: context.turn_id,
                input_message_id: input.input_message_id,
            },
        );
        if let Err(e) = event_emitter.emit(turn_started_event).await {
            warn!(error = %e, "Failed to emit turn.started event");
        }

        // Execute InputAtom
        let message_retriever =
            DirectMessageRetriever::new(self.db.clone(), self.event_service.clone());
        let atom = InputAtom::new(message_retriever, event_emitter);

        let atom_input = InputAtomInput { context };
        let result = atom.execute(atom_input).await?;

        Ok(serde_json::to_value(&result)?)
    }

    /// Execute reasoning activity (LLM call)
    async fn execute_reason_activity(&self, input: &DurableTurnInput) -> Result<serde_json::Value> {
        use everruns_core::events::{
            EventContext, EventRequest, SessionIdledData, TurnCompletedData, TurnFailedData,
        };
        use everruns_core::traits::EventEmitter;

        debug!(
            session_id = %input.session_id,
            "Executing reason activity"
        );

        // Create AtomContext
        let context = AtomContext {
            session_id: input.session_id,
            turn_id: Uuid::now_v7(),
            input_message_id: input.input_message_id,
            exec_id: Uuid::now_v7(),
        };

        let session_id = input.session_id;
        let turn_id = context.turn_id;
        let input_message_id = input.input_message_id;

        // Create direct adapters
        let agent_store = DirectAgentStore::new(self.db.clone());
        let session_store = DirectSessionStore::new(self.db.clone());
        let message_retriever =
            DirectMessageRetriever::new(self.db.clone(), self.event_service.clone());
        let provider_store = DirectLlmProviderStore::new(self.llm_resolver.clone());
        let capability_registry = CapabilityRegistry::with_builtins();
        let driver_registry = create_driver_registry();
        let event_emitter = DirectEventEmitter::new(self.event_service.clone());

        // Execute ReasonAtom
        let atom = ReasonAtom::new(
            agent_store,
            session_store,
            message_retriever,
            provider_store,
            capability_registry,
            driver_registry,
            event_emitter,
        );

        // Fetch MCP tool definitions for agent's MCP capabilities
        let mcp_tool_definitions = self
            .build_mcp_tool_definitions(input.agent_id)
            .await
            .unwrap_or_default();

        let reason_input = ReasonInput {
            context,
            agent_id: input.agent_id,
            mcp_tool_definitions,
        };

        let result = atom.execute(reason_input).await?;

        // If turn is complete (no tool calls or failure), set session to idle
        let turn_complete = !result.has_tool_calls || !result.success;
        if turn_complete {
            // Set session status to "idle"
            let status_updater = SessionStatusUpdater::new(self.db.clone());
            if let Err(e) = status_updater.set_status(session_id, "idle").await {
                warn!(error = %e, "Failed to set session status to idle");
            }

            let event_emitter = DirectEventEmitter::new(self.event_service.clone());

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
                if let Err(e) = event_emitter.emit(turn_failed_event).await {
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
                    },
                );
                if let Err(e) = event_emitter.emit(turn_completed_event).await {
                    warn!(error = %e, "Failed to emit turn.completed event");
                }
            }

            // Fetch session to get cumulative usage for session.idled event
            let session_usage = match self.db.get_session(session_id).await {
                Ok(Some(session_row)) => {
                    if session_row.total_input_tokens > 0 || session_row.total_output_tokens > 0 {
                        Some(TokenUsage::with_cache(
                            session_row.total_input_tokens as u32,
                            session_row.total_output_tokens as u32,
                            if session_row.total_cache_read_tokens > 0 {
                                Some(session_row.total_cache_read_tokens as u32)
                            } else {
                                None
                            },
                            if session_row.total_cache_creation_tokens > 0 {
                                Some(session_row.total_cache_creation_tokens as u32)
                            } else {
                                None
                            },
                        ))
                    } else {
                        // Fall back to turn usage if no cumulative usage yet
                        result.usage.clone()
                    }
                }
                _ => result.usage.clone(), // Fall back to turn usage on error
            };

            // Emit session.idled event with cumulative usage
            let idled_event = EventRequest::new(
                session_id,
                EventContext::turn(turn_id, input_message_id),
                SessionIdledData {
                    turn_id,
                    iterations: None,
                    usage: session_usage,
                },
            );
            if let Err(e) = event_emitter.emit(idled_event).await {
                warn!(error = %e, "Failed to emit session.idled event");
            }
        }

        Ok(serde_json::to_value(&result)?)
    }

    /// Execute act activity (tool execution)
    async fn execute_act_activity(&self, input: &ActInput) -> Result<serde_json::Value> {
        debug!(
            session_id = %input.context.session_id,
            tool_count = input.tool_calls.len(),
            "Executing act activity"
        );

        let tool_executor = ToolRegistry::with_defaults();
        let event_emitter = DirectEventEmitter::new(self.event_service.clone());
        let file_store = Arc::new(DirectSessionFileStore::new(self.db.clone()));

        let atom = ActAtom::with_file_store(tool_executor, event_emitter, file_store);

        let result = atom.execute(input.clone()).await?;

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
        use everruns_durable::TaskDefinition;

        let input_json = serde_json::to_value(input)?;

        match completed_activity {
            "process_input" => {
                // After input processing, schedule reason activity
                let activity_id = format!("reason_{}", Uuid::now_v7());
                let task = TaskDefinition {
                    workflow_id,
                    activity_id: activity_id.clone(),
                    activity_type: "reason".to_string(),
                    input: input_json.clone(),
                    options: Default::default(),
                };
                self.durable_store
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
                let _ =
                    append_event(self.durable_store.as_ref(), workflow_id, scheduled_event).await;

                debug!(workflow_id = %workflow_id, "Scheduled reason activity");
            }
            "reason" => {
                // After reasoning, check if there are tool calls
                let reason_result: ReasonResult = serde_json::from_value(output.clone())
                    .map_err(|e| anyhow::anyhow!("Failed to parse ReasonResult: {}", e))?;

                if reason_result.has_tool_calls && reason_result.success {
                    // Get tool count before moving
                    let tool_count = reason_result.tool_calls.len();

                    // Schedule act activity
                    let act_input = ActInput {
                        context: AtomContext {
                            session_id: input.session_id,
                            turn_id: Uuid::now_v7(),
                            input_message_id: input.input_message_id,
                            exec_id: Uuid::now_v7(),
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
                    self.durable_store
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
                    let _ = append_event(self.durable_store.as_ref(), workflow_id, scheduled_event)
                        .await;

                    debug!(
                        workflow_id = %workflow_id,
                        tool_count = tool_count,
                        "Scheduled act activity"
                    );
                } else {
                    // No tool calls or failure - workflow complete
                    // Record WorkflowCompleted event
                    record_workflow_completed(
                        self.durable_store.as_ref(),
                        workflow_id,
                        output.clone(),
                    )
                    .await;

                    self.durable_store
                        .update_workflow_status(workflow_id, WorkflowStatus::Completed, None, None)
                        .await
                        .map_err(|e| anyhow::anyhow!("Failed to update workflow status: {}", e))?;

                    info!(workflow_id = %workflow_id, "Workflow completed");
                }
            }
            "act" => {
                // After action, schedule another reason activity
                let activity_id = format!("reason_{}", Uuid::now_v7());
                let task = TaskDefinition {
                    workflow_id,
                    activity_id: activity_id.clone(),
                    activity_type: "reason".to_string(),
                    input: input_json.clone(),
                    options: Default::default(),
                };
                self.durable_store
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
                let _ =
                    append_event(self.durable_store.as_ref(), workflow_id, scheduled_event).await;

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

    /// Build MCP tool definitions from agent's MCP capabilities
    async fn build_mcp_tool_definitions(
        &self,
        agent_id: Uuid,
    ) -> Result<Vec<everruns_core::ToolDefinition>> {
        use everruns_core::capabilities::mcp::parse_mcp_capability_id;
        use everruns_core::mcp_server::mcp_tool_name;
        use everruns_core::tool_types::{BuiltinTool, ToolDefinition, ToolPolicy};

        // Get the agent's capability IDs
        let capability_rows = self.db.get_agent_capabilities(agent_id).await?;

        let mut mcp_tools = Vec::new();

        for cap_row in &capability_rows {
            let cap_id = &cap_row.capability_id;
            // Parse MCP server UUID from capability ID
            let server_id = match parse_mcp_capability_id(cap_id) {
                Some(id) => id,
                None => continue, // Not an MCP capability
            };

            // Fetch MCP server tools (using cache)
            let tools = match self.mcp_server_service.get_tools(server_id, false).await {
                Ok(t) => t,
                Err(e) => {
                    warn!(
                        server_id = %server_id,
                        error = %e,
                        "Failed to get MCP server tools, skipping"
                    );
                    continue;
                }
            };

            // Get server name for tool prefixing
            let server_name = match self.mcp_server_service.get(server_id).await {
                Ok(Some(s)) => s.name,
                _ => {
                    warn!(server_id = %server_id, "MCP server not found, skipping");
                    continue;
                }
            };

            // Convert each MCP tool to ToolDefinition
            for tool in tools {
                let prefixed_name = mcp_tool_name(&server_name, &tool.name);
                let description = tool
                    .description
                    .unwrap_or_else(|| format!("Tool from MCP server: {}", server_name));

                mcp_tools.push(ToolDefinition::Builtin(BuiltinTool {
                    name: prefixed_name,
                    description,
                    parameters: tool.input_schema,
                    policy: ToolPolicy::Auto,
                }));
            }
        }

        Ok(mcp_tools)
    }
}
