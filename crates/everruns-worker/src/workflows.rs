// Session workflow for agentic loop execution (M2)
// Uses Agent/Session/Messages model with Events as SSE notifications

use anyhow::Result;
use chrono::Utc;
use everruns_contracts::events::AgUiEvent;
use everruns_storage::models::{CreateMessage, UpdateSession};
use everruns_storage::repositories::Database;
use tracing::{error, info, warn};
use uuid::Uuid;

use crate::activities::{LlmCallActivity, PersistEventActivity, ToolExecutionActivity};
use crate::providers::{openai::OpenAiProvider, ChatMessage, LlmConfig, MessageRole};

/// Session workflow orchestrating LLM calls and tool execution
pub struct SessionWorkflow {
    session_id: Uuid,
    agent_id: Uuid,
    db: Database,
    persist_activity: PersistEventActivity,
}

impl SessionWorkflow {
    pub async fn new(session_id: Uuid, agent_id: Uuid, db: Database) -> Result<Self> {
        let persist_activity = PersistEventActivity::new(db.clone());
        Ok(Self {
            session_id,
            agent_id,
            db,
            persist_activity,
        })
    }

    /// Execute the workflow with real LLM calls
    pub async fn execute(&self) -> Result<()> {
        info!(
            session_id = %self.session_id,
            agent_id = %self.agent_id,
            "Starting session workflow"
        );

        // Update session status to running and set started_at
        self.update_session_status("running", Some(Utc::now()), None)
            .await?;

        // Emit SESSION_STARTED event (SSE notification)
        let started_event = AgUiEvent::session_started(self.session_id.to_string());
        self.persist_activity
            .persist_event(self.session_id, started_event)
            .await?;

        // Load agent to get configuration
        let agent = self
            .db
            .get_agent(self.agent_id)
            .await?
            .ok_or_else(|| anyhow::anyhow!("Agent not found"))?;

        // Build LLM config from agent settings
        let model = agent
            .default_model_id
            .map(|id| id.to_string())
            .unwrap_or_else(|| "gpt-4o".to_string());

        let llm_config = LlmConfig {
            model,
            temperature: None, // TODO: Add to session or use default
            max_tokens: None,  // TODO: Add to session or use default
            system_prompt: Some(agent.system_prompt.clone()),
            tools: Vec::new(), // TODO: Parse tools from session
        };

        // Load messages from session (PRIMARY data)
        let messages_rows = self.db.list_messages(self.session_id).await?;

        if messages_rows.is_empty() {
            warn!(
                session_id = %self.session_id,
                "No messages in session, skipping LLM call"
            );
        } else {
            // Convert messages to ChatMessage format
            let mut messages: Vec<ChatMessage> = Vec::new();

            // Add system prompt as first message
            if !agent.system_prompt.is_empty() {
                messages.push(ChatMessage {
                    role: MessageRole::System,
                    content: agent.system_prompt.clone(),
                    tool_calls: None,
                    tool_call_id: None,
                });
            }

            // Add messages from database
            for msg in &messages_rows {
                let role = match msg.role.as_str() {
                    "user" => MessageRole::User,
                    "assistant" => MessageRole::Assistant,
                    "system" => MessageRole::System,
                    "tool_call" => continue, // Tool calls are handled separately
                    "tool_result" => MessageRole::Tool,
                    _ => continue,
                };

                // Extract content from JSON
                let content = if let Some(text) = msg.content.get("text").and_then(|t| t.as_str()) {
                    text.to_string()
                } else if let Some(content_str) = msg.content.as_str() {
                    content_str.to_string()
                } else if let Some(result) = msg.content.get("result") {
                    serde_json::to_string(result).unwrap_or_else(|_| "{}".to_string())
                } else {
                    continue;
                };

                messages.push(ChatMessage {
                    role,
                    content,
                    tool_calls: None,
                    tool_call_id: msg.tool_call_id.clone(),
                });
            }

            info!(
                session_id = %self.session_id,
                message_count = messages.len(),
                model = %llm_config.model,
                "Calling LLM"
            );

            // Call LLM (use OpenAI provider)
            let provider = OpenAiProvider::new()?;
            let llm_activity = LlmCallActivity::new(provider, self.db.clone());
            let tool_activity = ToolExecutionActivity::new(self.db.clone());

            // Tool calling loop: Call LLM → Execute tools → Loop back with results
            const MAX_TOOL_ITERATIONS: usize = 5;
            let mut iteration = 0;
            let mut current_messages = messages;

            loop {
                iteration += 1;
                if iteration > MAX_TOOL_ITERATIONS {
                    warn!(
                        session_id = %self.session_id,
                        "Max tool calling iterations reached, stopping"
                    );
                    break;
                }

                // Call LLM (non-streaming)
                let result = llm_activity
                    .call(
                        self.session_id,
                        current_messages.clone(),
                        llm_config.clone(),
                    )
                    .await?;

                // If we got a response, persist it as an assistant Message
                if !result.text.is_empty() {
                    info!(
                        session_id = %self.session_id,
                        response_length = result.text.len(),
                        "Got assistant response"
                    );

                    // Store assistant response as Message (PRIMARY data)
                    let create_msg = CreateMessage {
                        session_id: self.session_id,
                        role: "assistant".to_string(),
                        content: serde_json::json!({ "text": result.text.clone() }),
                        tool_call_id: None,
                    };
                    self.db.create_message(create_msg).await?;

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
                        session_id = %self.session_id,
                        tool_count = tool_calls.len(),
                        "Executing tool calls"
                    );

                    // Store tool calls as Messages (PRIMARY data)
                    for tool_call in &tool_calls {
                        let create_msg = CreateMessage {
                            session_id: self.session_id,
                            role: "tool_call".to_string(),
                            content: serde_json::json!({
                                "id": tool_call.id,
                                "name": tool_call.name,
                                "arguments": tool_call.arguments
                            }),
                            tool_call_id: Some(tool_call.id.clone()),
                        };
                        self.db.create_message(create_msg).await?;
                    }

                    // Execute tools in parallel
                    let tool_results = tool_activity
                        .execute_tool_calls_parallel(
                            self.session_id,
                            &tool_calls,
                            &llm_config.tools,
                        )
                        .await?;

                    // Store tool results as Messages (PRIMARY data) and add to conversation
                    for (tool_call, tool_result) in tool_calls.iter().zip(tool_results.iter()) {
                        let result_content = if let Some(result) = &tool_result.result {
                            serde_json::to_string(result).unwrap_or_else(|_| "{}".to_string())
                        } else if let Some(error) = &tool_result.error {
                            format!(r#"{{"error": "{}"}}"#, error)
                        } else {
                            "{}".to_string()
                        };

                        // Store tool result as Message
                        let create_msg = CreateMessage {
                            session_id: self.session_id,
                            role: "tool_result".to_string(),
                            content: serde_json::json!({
                                "result": tool_result.result,
                                "error": tool_result.error
                            }),
                            tool_call_id: Some(tool_call.id.clone()),
                        };
                        self.db.create_message(create_msg).await?;

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

        // Emit SESSION_FINISHED event (SSE notification)
        let finished_event = AgUiEvent::session_finished(self.session_id.to_string());
        self.persist_activity
            .persist_event(self.session_id, finished_event)
            .await?;

        // Update session status to completed and set finished_at
        self.update_session_status("completed", None, Some(Utc::now()))
            .await?;

        info!(
            session_id = %self.session_id,
            "Session workflow completed successfully"
        );

        Ok(())
    }

    /// Handle workflow cancellation
    pub async fn cancel(&self) -> Result<()> {
        info!(session_id = %self.session_id, "Cancelling session workflow");

        // Update session status to failed and set finished_at
        self.update_session_status("failed", None, Some(Utc::now()))
            .await?;

        Ok(())
    }

    /// Handle workflow errors
    pub async fn handle_error(&self, error: &anyhow::Error) -> Result<()> {
        error!(
            session_id = %self.session_id,
            error = %error,
            "Session workflow failed"
        );

        // Emit SESSION_ERROR event (SSE notification)
        let error_event = AgUiEvent::session_error(error.to_string());
        self.persist_activity
            .persist_event(self.session_id, error_event)
            .await?;

        // Update session status to failed and set finished_at
        self.update_session_status("failed", None, Some(Utc::now()))
            .await?;

        Ok(())
    }

    /// Update the session timestamps and status
    async fn update_session_status(
        &self,
        status: &str,
        started_at: Option<chrono::DateTime<Utc>>,
        finished_at: Option<chrono::DateTime<Utc>>,
    ) -> Result<()> {
        let input = UpdateSession {
            status: Some(status.to_string()),
            started_at,
            finished_at,
            ..Default::default()
        };

        self.db.update_session(self.session_id, input).await?;
        Ok(())
    }
}

// Keep the old name as an alias for backwards compatibility during migration
pub type AgentRunWorkflow = SessionWorkflow;

impl AgentRunWorkflow {
    /// Legacy compatibility constructor
    /// In M2, run_id maps to session_id, agent_id remains agent_id
    pub async fn legacy_new(
        run_id: Uuid,
        agent_id: Uuid,
        _thread_id: Uuid,
        db: Database,
    ) -> Result<Self> {
        SessionWorkflow::new(run_id, agent_id, db).await
    }
}
