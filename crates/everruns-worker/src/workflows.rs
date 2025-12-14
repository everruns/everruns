// Temporal workflows for durable agent execution

use anyhow::Result;
use chrono::Utc;
use everruns_contracts::events::AgUiEvent;
use everruns_contracts::resources::RunStatus;
use everruns_storage::models::{CreateMessage, UpdateRun};
use everruns_storage::repositories::Database;
use tracing::{error, info, warn};
use uuid::Uuid;

use crate::activities::{LlmCallActivity, PersistEventActivity, ToolExecutionActivity};
use crate::providers::{openai::OpenAiProvider, ChatMessage, LlmConfig, MessageRole};

/// Agent run workflow orchestrating LLM calls and tool execution
pub struct AgentRunWorkflow {
    run_id: Uuid,
    agent_id: Uuid,
    agent_version: i32,
    thread_id: Uuid,
    db: Database,
    persist_activity: PersistEventActivity,
}

impl AgentRunWorkflow {
    pub async fn new(
        run_id: Uuid,
        agent_id: Uuid,
        agent_version: i32,
        thread_id: Uuid,
        db: Database,
    ) -> Result<Self> {
        let persist_activity = PersistEventActivity::new(db.clone());
        Ok(Self {
            run_id,
            agent_id,
            agent_version,
            thread_id,
            db,
            persist_activity,
        })
    }

    /// Execute the workflow with real LLM calls (M5)
    pub async fn execute(&self) -> Result<()> {
        info!(
            run_id = %self.run_id,
            agent_id = %self.agent_id,
            thread_id = %self.thread_id,
            "Starting agent run workflow"
        );

        // Update run status to running
        self.update_run_status(RunStatus::Running, Some(Utc::now()), None)
            .await?;

        // Emit RUN_STARTED event
        let started_event =
            AgUiEvent::run_started(self.thread_id.to_string(), self.run_id.to_string());
        self.persist_activity
            .persist_event(self.run_id, started_event)
            .await?;

        // Load agent version to get configuration
        let agent_version = self
            .db
            .get_agent_version(self.agent_id, self.agent_version)
            .await?
            .ok_or_else(|| anyhow::anyhow!("Agent version not found"))?;

        // Load agent for default model
        let agent = self
            .db
            .get_agent(self.agent_id)
            .await?
            .ok_or_else(|| anyhow::anyhow!("Agent not found"))?;

        // Parse agent definition for LLM config
        let definition = &agent_version.definition;
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
            tools: Vec::new(), // TODO M6: Parse tools from agent definition
        };

        // Load thread messages
        let message_rows = self.db.list_messages(self.thread_id).await?;

        if message_rows.is_empty() {
            warn!(
                run_id = %self.run_id,
                thread_id = %self.thread_id,
                "No messages in thread, skipping LLM call"
            );
        } else {
            // Convert to ChatMessage format
            let messages: Vec<ChatMessage> = message_rows
                .iter()
                .map(|row| ChatMessage {
                    role: match row.role.as_str() {
                        "system" => MessageRole::System,
                        "user" => MessageRole::User,
                        "assistant" => MessageRole::Assistant,
                        "tool" => MessageRole::Tool,
                        _ => MessageRole::User, // Default to user
                    },
                    content: row.content.clone(),
                    tool_calls: None, // TODO M6: Parse tool calls from message metadata
                    tool_call_id: None, // TODO M6: Parse tool call ID from message metadata
                })
                .collect();

            info!(
                run_id = %self.run_id,
                message_count = messages.len(),
                model = %llm_config.model,
                "Calling LLM"
            );

            // Call LLM (use OpenAI provider for M5)
            let provider = OpenAiProvider::new()?;
            let llm_activity = LlmCallActivity::new(provider, self.db.clone());
            let tool_activity = ToolExecutionActivity::new(self.db.clone());

            // Tool calling loop (M6): Call LLM → Execute tools → Loop back with results
            const MAX_TOOL_ITERATIONS: usize = 5; // Prevent infinite loops
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

                // Call LLM
                let result = llm_activity
                    .call_and_stream(self.run_id, current_messages.clone(), llm_config.clone())
                    .await?;

                // Save assistant response text if any
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
                        "Saved assistant response"
                    );

                    // Add assistant message to conversation
                    current_messages.push(ChatMessage {
                        role: MessageRole::Assistant,
                        content: result.text.clone(),
                        tool_calls: result.tool_calls.clone(),
                        tool_call_id: None,
                    });
                }

                // Check if there are tool calls to execute
                if let Some(tool_calls) = result.tool_calls {
                    info!(
                        run_id = %self.run_id,
                        tool_count = tool_calls.len(),
                        "Executing tool calls"
                    );

                    // Execute tools in parallel
                    let tool_results = tool_activity
                        .execute_tool_calls_parallel(self.run_id, &tool_calls, &llm_config.tools)
                        .await?;

                    // Add tool results to conversation as Tool messages
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

                    // Continue loop to call LLM again with tool results
                    continue;
                }

                // No tool calls, we're done
                break;
            }
        }

        // Emit RUN_FINISHED event
        let finished_event =
            AgUiEvent::run_finished(self.thread_id.to_string(), self.run_id.to_string());
        self.persist_activity
            .persist_event(self.run_id, finished_event)
            .await?;

        // Update run status to completed
        let finished_at = Utc::now();
        self.update_run_status(RunStatus::Completed, None, Some(finished_at))
            .await?;

        info!(
            run_id = %self.run_id,
            "Agent run workflow completed successfully"
        );

        Ok(())
    }

    /// Handle workflow cancellation
    pub async fn cancel(&self) -> Result<()> {
        info!(run_id = %self.run_id, "Cancelling agent run workflow");

        // Update run status to cancelled
        let finished_at = Utc::now();
        self.update_run_status(RunStatus::Cancelled, None, Some(finished_at))
            .await?;

        Ok(())
    }

    /// Handle workflow errors
    pub async fn handle_error(&self, error: &anyhow::Error) -> Result<()> {
        error!(
            run_id = %self.run_id,
            error = %error,
            "Agent run workflow failed"
        );

        // Emit RUN_ERROR event
        let error_event = AgUiEvent::run_error(error.to_string());
        self.persist_activity
            .persist_event(self.run_id, error_event)
            .await?;

        // Update run status to failed
        let finished_at = Utc::now();
        self.update_run_status(RunStatus::Failed, None, Some(finished_at))
            .await?;

        Ok(())
    }

    /// Update the run status in the database
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
