// In-process worker for DEV_MODE
//
// Decision: Runs task execution in-process with the control-plane for DEV_MODE
// Decision: Uses direct adapters instead of gRPC for all operations
//
// This worker polls the InMemoryWorkflowEventStore for tasks and executes them
// using direct access to the storage backend.

use anyhow::Result;
use everruns_core::atoms::{ActAtom, Atom, AtomContext, InputAtom, ReasonAtom};
use everruns_core::capabilities::CapabilityRegistry;
use everruns_core::ToolRegistry;
use everruns_core::{ActInput, InputAtomInput, ReasonInput, ReasonResult};
use everruns_durable::{
    ClaimedTask, InMemoryWorkflowEventStore, WorkerInfo, WorkflowEventStore, WorkflowStatus,
};
use everruns_worker::{create_driver_registry, DurableTurnInput};
use std::sync::Arc;
use std::time::Duration;
use tokio::sync::watch;
use tracing::{debug, error, info, warn};
use uuid::Uuid;

use super::direct_adapters::{
    DirectAgentStore, DirectEventEmitter, DirectLlmProviderStore, DirectMessageStore,
    DirectSessionFileStore, DirectSessionStore, SessionStatusUpdater,
};
use crate::services::{EventService, LlmResolverService};
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
                // Complete the task
                self.durable_store
                    .complete_task(task.id, output.clone())
                    .await
                    .map_err(|e| anyhow::anyhow!("Failed to complete task: {}", e))?;

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
        let message_store = DirectMessageStore::new(self.db.clone(), self.event_service.clone());
        let atom = InputAtom::new(message_store, event_emitter);

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
        let message_store = DirectMessageStore::new(self.db.clone(), self.event_service.clone());
        let provider_store = DirectLlmProviderStore::new(self.llm_resolver.clone());
        let capability_registry = CapabilityRegistry::with_builtins();
        let driver_registry = create_driver_registry();
        let event_emitter = DirectEventEmitter::new(self.event_service.clone());

        // Execute ReasonAtom
        let atom = ReasonAtom::new(
            agent_store,
            session_store,
            message_store,
            provider_store,
            capability_registry,
            driver_registry,
            event_emitter,
        );

        let reason_input = ReasonInput {
            context,
            agent_id: input.agent_id,
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
                    },
                );
                if let Err(e) = event_emitter.emit(turn_completed_event).await {
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
                let task = TaskDefinition {
                    workflow_id,
                    activity_id: format!("reason_{}", Uuid::now_v7()),
                    activity_type: "reason".to_string(),
                    input: input_json,
                    options: Default::default(),
                };
                self.durable_store
                    .enqueue_task(task)
                    .await
                    .map_err(|e| anyhow::anyhow!("Failed to enqueue reason task: {}", e))?;

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

                    let task = TaskDefinition {
                        workflow_id,
                        activity_id: format!("act_{}", Uuid::now_v7()),
                        activity_type: "act".to_string(),
                        input: act_input_json,
                        options: Default::default(),
                    };
                    self.durable_store
                        .enqueue_task(task)
                        .await
                        .map_err(|e| anyhow::anyhow!("Failed to enqueue act task: {}", e))?;

                    debug!(
                        workflow_id = %workflow_id,
                        tool_count = tool_count,
                        "Scheduled act activity"
                    );
                } else {
                    // No tool calls or failure - workflow complete
                    self.durable_store
                        .update_workflow_status(workflow_id, WorkflowStatus::Completed, None, None)
                        .await
                        .map_err(|e| anyhow::anyhow!("Failed to update workflow status: {}", e))?;

                    info!(workflow_id = %workflow_id, "Workflow completed");
                }
            }
            "act" => {
                // After action, schedule another reason activity
                let task = TaskDefinition {
                    workflow_id,
                    activity_id: format!("reason_{}", Uuid::now_v7()),
                    activity_type: "reason".to_string(),
                    input: input_json,
                    options: Default::default(),
                };
                self.durable_store
                    .enqueue_task(task)
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
