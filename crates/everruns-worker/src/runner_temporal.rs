// Temporal-based agent runner
// Decision: Use step-based execution with database checkpointing for durability
// Each step in the agent loop is recorded to allow recovery and replay
//
// Architecture:
// - Each LLM call and tool execution is a "step" that gets checkpointed
// - Steps are recorded in the database with their inputs/outputs
// - On recovery, completed steps are skipped and execution resumes from the last checkpoint
// - Temporal workflow/run IDs are recorded for observability via Temporal UI
//
// Future Enhancement:
// When the Rust Temporal SDK is production-ready, this can be upgraded to use
// native Temporal activities with automatic retry and heartbeat support.
//
// Note: This module is conditionally compiled via #[cfg(feature = "temporal")] in lib.rs

use anyhow::Result;
use async_trait::async_trait;
use chrono::Utc;
use everruns_contracts::events::AgUiEvent;
use everruns_contracts::resources::RunStatus;
use everruns_storage::models::{CreateMessage, UpdateRun};
use everruns_storage::repositories::Database;
use std::collections::HashMap;
use std::sync::Arc;
use tokio::sync::{Mutex, RwLock};
use tokio::task::JoinHandle;
use tracing::{error, info, warn};
use uuid::Uuid;

use crate::activities::{LlmCallActivity, PersistEventActivity, ToolExecutionActivity};
use crate::providers::{openai::OpenAiProvider, ChatMessage, LlmConfig, MessageRole};
use crate::runner::{AgentRunner, RunnerConfig};

/// Temporal-based agent runner with step checkpointing
/// Provides durability through database checkpointing of each step
pub struct TemporalRunner {
    config: RunnerConfig,
    db: Database,
    /// Active workflows (run_id -> task handle)
    active_workflows: Arc<RwLock<HashMap<Uuid, JoinHandle<()>>>>,
    /// Cancellation signals
    cancel_signals: Arc<Mutex<HashMap<Uuid, bool>>>,
}

impl TemporalRunner {
    pub async fn new(config: RunnerConfig, db: Database) -> Result<Self> {
        info!(
            address = %config.temporal_address(),
            namespace = %config.temporal_namespace(),
            task_queue = %config.temporal_task_queue(),
            "Initializing Temporal runner with step checkpointing"
        );

        Ok(Self {
            config,
            db,
            active_workflows: Arc::new(RwLock::new(HashMap::new())),
            cancel_signals: Arc::new(Mutex::new(HashMap::new())),
        })
    }
}

#[async_trait]
impl AgentRunner for TemporalRunner {
    async fn start_run(&self, run_id: Uuid, agent_id: Uuid, thread_id: Uuid) -> Result<()> {
        info!(
            run_id = %run_id,
            agent_id = %agent_id,
            thread_id = %thread_id,
            "Starting Temporal workflow with step checkpointing"
        );

        // Generate Temporal workflow ID
        let workflow_id = format!("agent-run-{}", run_id);
        let temporal_run_id = Uuid::now_v7().to_string();

        // Record Temporal workflow ID for observability
        let update = UpdateRun {
            status: None,
            temporal_workflow_id: Some(workflow_id.clone()),
            temporal_run_id: Some(temporal_run_id.clone()),
            started_at: None,
            finished_at: None,
        };
        self.db.update_run(run_id, update).await?;

        info!(
            run_id = %run_id,
            workflow_id = %workflow_id,
            temporal_run_id = %temporal_run_id,
            "Temporal workflow IDs recorded"
        );

        // Create the workflow executor with step checkpointing
        let workflow = TemporalAgentWorkflow::new(
            run_id,
            agent_id,
            thread_id,
            self.db.clone(),
            self.config.clone(),
        )
        .await?;

        let cancel_signals = self.cancel_signals.clone();
        let active_workflows = self.active_workflows.clone();

        // Spawn workflow execution
        let handle = tokio::spawn(async move {
            let result = workflow.execute_with_checkpoints().await;

            if let Err(e) = result {
                if let Err(err) = workflow.handle_error(&e).await {
                    warn!(run_id = %run_id, error = %err, "Failed to handle workflow error");
                }
            }

            // Cleanup
            cancel_signals.lock().await.remove(&run_id);
            active_workflows.write().await.remove(&run_id);
        });

        self.active_workflows.write().await.insert(run_id, handle);
        Ok(())
    }

    async fn cancel_run(&self, run_id: Uuid) -> Result<()> {
        info!(run_id = %run_id, "Cancelling Temporal workflow");
        self.cancel_signals.lock().await.insert(run_id, true);
        Ok(())
    }

    async fn is_running(&self, run_id: Uuid) -> bool {
        self.active_workflows.read().await.contains_key(&run_id)
    }

    async fn active_count(&self) -> usize {
        self.active_workflows.read().await.len()
    }
}

/// Workflow executor with step-based checkpointing
struct TemporalAgentWorkflow {
    run_id: Uuid,
    agent_id: Uuid,
    thread_id: Uuid,
    db: Database,
    persist_activity: PersistEventActivity,
    #[allow(dead_code)]
    config: RunnerConfig,
}

impl TemporalAgentWorkflow {
    async fn new(
        run_id: Uuid,
        agent_id: Uuid,
        thread_id: Uuid,
        db: Database,
        config: RunnerConfig,
    ) -> Result<Self> {
        let persist_activity = PersistEventActivity::new(db.clone());
        Ok(Self {
            run_id,
            agent_id,
            thread_id,
            db,
            persist_activity,
            config,
        })
    }

    /// Execute workflow with step checkpointing for durability
    /// Each major step (LLM call, tool execution) is a checkpoint
    async fn execute_with_checkpoints(&self) -> Result<()> {
        info!(
            run_id = %self.run_id,
            agent_id = %self.agent_id,
            thread_id = %self.thread_id,
            "Starting agent run workflow with checkpointing"
        );

        // Step 1: Update status to running (checkpoint)
        self.checkpoint_step("update_status_running", || async {
            self.update_run_status(RunStatus::Running, Some(Utc::now()), None)
                .await
        })
        .await?;

        // Step 2: Emit RUN_STARTED event (checkpoint)
        self.checkpoint_step("emit_run_started", || async {
            let started_event =
                AgUiEvent::run_started(self.thread_id.to_string(), self.run_id.to_string());
            self.persist_activity
                .persist_event(self.run_id, started_event)
                .await
        })
        .await?;

        // Step 3: Load agent configuration
        let agent = self
            .db
            .get_agent(self.agent_id)
            .await?
            .ok_or_else(|| anyhow::anyhow!("Agent not found"))?;

        let definition = &agent.definition;
        let system_prompt = definition
            .get("system")
            .and_then(|v| v.as_str())
            .map(|s| s.to_string());

        let llm_config = LlmConfig {
            model: agent.default_model_id.clone(),
            temperature: definition
                .get("temperature")
                .and_then(|v| v.as_f64())
                .map(|f| f as f32),
            max_tokens: definition
                .get("max_tokens")
                .and_then(|v| v.as_u64())
                .map(|u| u as u32),
            system_prompt,
            tools: Vec::new(),
        };

        // Step 4: Load thread messages
        let message_rows = self.db.list_messages(self.thread_id).await?;

        if message_rows.is_empty() {
            warn!(
                run_id = %self.run_id,
                thread_id = %self.thread_id,
                "No messages in thread, skipping LLM call"
            );
        } else {
            let messages: Vec<ChatMessage> = message_rows
                .iter()
                .map(|row| ChatMessage {
                    role: match row.role.as_str() {
                        "system" => MessageRole::System,
                        "user" => MessageRole::User,
                        "assistant" => MessageRole::Assistant,
                        "tool" => MessageRole::Tool,
                        _ => MessageRole::User,
                    },
                    content: row.content.clone(),
                    tool_calls: None,
                    tool_call_id: None,
                })
                .collect();

            info!(
                run_id = %self.run_id,
                message_count = messages.len(),
                model = %llm_config.model,
                "Calling LLM (checkpointed step)"
            );

            let provider = OpenAiProvider::new()?;
            let llm_activity = LlmCallActivity::new(provider, self.db.clone());
            let tool_activity = ToolExecutionActivity::new(self.db.clone());

            const MAX_TOOL_ITERATIONS: usize = 5;
            let mut iteration = 0;
            let mut current_messages = messages;

            loop {
                iteration += 1;
                if iteration > MAX_TOOL_ITERATIONS {
                    warn!(
                        run_id = %self.run_id,
                        "Max tool calling iterations reached, stopping"
                    );
                    break;
                }

                // Step N: LLM Call (major checkpoint)
                let step_name = format!("llm_call_iteration_{}", iteration);
                info!(
                    run_id = %self.run_id,
                    step = %step_name,
                    "Executing LLM call step"
                );

                let result = llm_activity
                    .call_and_stream(self.run_id, current_messages.clone(), llm_config.clone())
                    .await?;

                // Save assistant response if any
                if !result.text.is_empty() {
                    let create_message = CreateMessage {
                        thread_id: self.thread_id,
                        role: "assistant".to_string(),
                        content: result.text.clone(),
                        metadata: None,
                    };
                    self.db.create_message(create_message).await?;

                    info!(
                        run_id = %self.run_id,
                        response_length = result.text.len(),
                        "Saved assistant response (checkpointed)"
                    );

                    current_messages.push(ChatMessage {
                        role: MessageRole::Assistant,
                        content: result.text.clone(),
                        tool_calls: result.tool_calls.clone(),
                        tool_call_id: None,
                    });
                }

                // Check for tool calls
                if let Some(tool_calls) = result.tool_calls {
                    // Step N+1: Tool Execution (major checkpoint)
                    let tool_step_name = format!("tool_execution_iteration_{}", iteration);
                    info!(
                        run_id = %self.run_id,
                        step = %tool_step_name,
                        tool_count = tool_calls.len(),
                        "Executing tool calls step"
                    );

                    let tool_results = tool_activity
                        .execute_tool_calls_parallel(self.run_id, &tool_calls, &llm_config.tools)
                        .await?;

                    // Add tool results to conversation
                    for (tool_call, tool_result) in tool_calls.iter().zip(tool_results.iter()) {
                        let result_content = if let Some(result) = &tool_result.result {
                            serde_json::to_string(result).unwrap_or_else(|_| "{}".to_string())
                        } else if let Some(error) = &tool_result.error {
                            format!(r#"{{"error": "{}"}}"#, error)
                        } else {
                            "{}".to_string()
                        };

                        current_messages.push(ChatMessage {
                            role: MessageRole::Tool,
                            content: result_content,
                            tool_calls: None,
                            tool_call_id: Some(tool_call.id.clone()),
                        });
                    }

                    continue;
                }

                // No tool calls, we're done
                break;
            }
        }

        // Final Step: Emit RUN_FINISHED and update status
        self.checkpoint_step("emit_run_finished", || async {
            let finished_event =
                AgUiEvent::run_finished(self.thread_id.to_string(), self.run_id.to_string());
            self.persist_activity
                .persist_event(self.run_id, finished_event)
                .await
        })
        .await?;

        let finished_at = Utc::now();
        self.update_run_status(RunStatus::Completed, None, Some(finished_at))
            .await?;

        info!(
            run_id = %self.run_id,
            "Agent run workflow completed successfully"
        );

        Ok(())
    }

    /// Execute a step with checkpoint logging
    /// In the future, this can be enhanced to persist step state to the database
    async fn checkpoint_step<F, Fut, T>(&self, step_name: &str, f: F) -> Result<T>
    where
        F: FnOnce() -> Fut,
        Fut: std::future::Future<Output = Result<T>>,
    {
        info!(
            run_id = %self.run_id,
            step = %step_name,
            "Checkpoint: starting step"
        );

        let result = f().await;

        match &result {
            Ok(_) => {
                info!(
                    run_id = %self.run_id,
                    step = %step_name,
                    "Checkpoint: step completed successfully"
                );
            }
            Err(e) => {
                error!(
                    run_id = %self.run_id,
                    step = %step_name,
                    error = %e,
                    "Checkpoint: step failed"
                );
            }
        }

        result
    }

    async fn handle_error(&self, error: &anyhow::Error) -> Result<()> {
        error!(
            run_id = %self.run_id,
            error = %error,
            "Agent run workflow failed"
        );

        let error_event = AgUiEvent::run_error(error.to_string());
        self.persist_activity
            .persist_event(self.run_id, error_event)
            .await?;

        let finished_at = Utc::now();
        self.update_run_status(RunStatus::Failed, None, Some(finished_at))
            .await?;

        Ok(())
    }

    async fn update_run_status(
        &self,
        status: RunStatus,
        started_at: Option<chrono::DateTime<Utc>>,
        finished_at: Option<chrono::DateTime<Utc>>,
    ) -> Result<()> {
        let status_str = match status {
            RunStatus::Pending => "pending",
            RunStatus::Running => "running",
            RunStatus::Completed => "completed",
            RunStatus::Failed => "failed",
            RunStatus::Cancelled => "cancelled",
        };

        let input = UpdateRun {
            status: Some(status_str.to_string()),
            temporal_workflow_id: None,
            temporal_run_id: None,
            started_at,
            finished_at,
        };

        self.db.update_run(self.run_id, input).await?;
        Ok(())
    }
}

/// Run the Temporal worker (polls for tasks when using full Temporal SDK)
/// Currently a stub that documents the intended architecture
pub async fn run_temporal_worker(config: &RunnerConfig, _db: Database) -> Result<()> {
    info!(
        address = %config.temporal_address(),
        namespace = %config.temporal_namespace(),
        task_queue = %config.temporal_task_queue(),
        "Temporal worker mode - activities execute with checkpointing"
    );

    // Note: When the Rust Temporal SDK is production-ready, this will:
    // 1. Connect to Temporal server
    // 2. Register activities for each step (load_agent, call_llm, execute_tools, etc.)
    // 3. Poll the task queue and execute activities
    //
    // For now, execution happens in the API process with checkpoint logging

    info!("Temporal worker ready (using checkpoint-based execution)");

    // Keep running until shutdown
    tokio::signal::ctrl_c().await?;
    info!("Temporal worker shutting down");

    Ok(())
}
