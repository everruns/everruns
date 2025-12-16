// Session workflow for agentic loop execution (M2)

use anyhow::Result;
use chrono::Utc;
use everruns_contracts::events::AgUiEvent;
use everruns_storage::models::UpdateSession;
use everruns_storage::repositories::Database;
use tracing::{error, info, warn};
use uuid::Uuid;

use crate::activities::{LlmCallActivity, PersistEventActivity, ToolExecutionActivity};
use crate::providers::{openai::OpenAiProvider, ChatMessage, LlmConfig, MessageRole};

/// Session workflow orchestrating LLM calls and tool execution
/// In M2, this replaces the old AgentRunWorkflow but keeps the same internal logic
pub struct SessionWorkflow {
    session_id: Uuid,
    harness_id: Uuid,
    db: Database,
    persist_activity: PersistEventActivity,
}

impl SessionWorkflow {
    pub async fn new(session_id: Uuid, harness_id: Uuid, db: Database) -> Result<Self> {
        let persist_activity = PersistEventActivity::new(db.clone());
        Ok(Self {
            session_id,
            harness_id,
            db,
            persist_activity,
        })
    }

    /// Execute the workflow with real LLM calls
    pub async fn execute(&self) -> Result<()> {
        info!(
            session_id = %self.session_id,
            harness_id = %self.harness_id,
            "Starting session workflow"
        );

        // Update session started_at
        self.update_session(Some(Utc::now()), None).await?;

        // Emit SESSION_STARTED event
        let started_event = AgUiEvent::session_started(self.session_id.to_string());
        self.persist_activity
            .persist_event(self.session_id, started_event)
            .await?;

        // Load harness to get configuration
        let harness = self
            .db
            .get_harness(self.harness_id)
            .await?
            .ok_or_else(|| anyhow::anyhow!("Harness not found"))?;

        // Build LLM config from harness settings
        let model = harness
            .default_model_id
            .map(|id| id.to_string())
            .unwrap_or_else(|| "gpt-4o".to_string());

        let llm_config = LlmConfig {
            model,
            temperature: harness.temperature,
            max_tokens: harness.max_tokens.map(|t| t as u32),
            system_prompt: Some(harness.system_prompt.clone()),
            tools: Vec::new(), // TODO: Parse tools from harness
        };

        // Load message events from session
        let message_events = self.db.list_message_events(self.session_id).await?;

        if message_events.is_empty() {
            warn!(
                session_id = %self.session_id,
                "No messages in session, skipping LLM call"
            );
        } else {
            // Convert message events to ChatMessage format
            let mut messages: Vec<ChatMessage> = Vec::new();

            // Add system prompt as first message
            if !harness.system_prompt.is_empty() {
                messages.push(ChatMessage {
                    role: MessageRole::System,
                    content: harness.system_prompt.clone(),
                    tool_calls: None,
                    tool_call_id: None,
                });
            }

            // Add messages from events
            for event in &message_events {
                // Parse event data to extract message content
                if let Some(data) = event.data.as_object() {
                    let role = match event.event_type.as_str() {
                        "message.user" => MessageRole::User,
                        "message.assistant" => MessageRole::Assistant,
                        "message.system" => MessageRole::System,
                        _ => continue,
                    };

                    // Try to get content from message.content[0].text or message.content
                    let content = if let Some(message) = data.get("message") {
                        if let Some(content_arr) = message.get("content").and_then(|c| c.as_array())
                        {
                            content_arr
                                .iter()
                                .filter_map(|c| c.get("text").and_then(|t| t.as_str()))
                                .collect::<Vec<_>>()
                                .join("")
                        } else if let Some(content_str) =
                            message.get("content").and_then(|c| c.as_str())
                        {
                            content_str.to_string()
                        } else {
                            continue;
                        }
                    } else if let Some(content) = data.get("content").and_then(|c| c.as_str()) {
                        content.to_string()
                    } else {
                        continue;
                    };

                    messages.push(ChatMessage {
                        role,
                        content,
                        tool_calls: None,
                        tool_call_id: None,
                    });
                }
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

                // Call LLM
                let result = llm_activity
                    .call_and_stream(
                        self.session_id,
                        current_messages.clone(),
                        llm_config.clone(),
                    )
                    .await?;

                // Add assistant response text to conversation if any
                if !result.text.is_empty() {
                    info!(
                        session_id = %self.session_id,
                        response_length = result.text.len(),
                        "Got assistant response"
                    );

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

                    // Execute tools in parallel
                    let tool_results = tool_activity
                        .execute_tool_calls_parallel(
                            self.session_id,
                            &tool_calls,
                            &llm_config.tools,
                        )
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

        // Emit SESSION_FINISHED event
        let finished_event = AgUiEvent::session_finished(self.session_id.to_string());
        self.persist_activity
            .persist_event(self.session_id, finished_event)
            .await?;

        // Update session finished_at
        self.update_session(None, Some(Utc::now())).await?;

        info!(
            session_id = %self.session_id,
            "Session workflow completed successfully"
        );

        Ok(())
    }

    /// Handle workflow cancellation
    pub async fn cancel(&self) -> Result<()> {
        info!(session_id = %self.session_id, "Cancelling session workflow");

        // Update session finished_at
        self.update_session(None, Some(Utc::now())).await?;

        Ok(())
    }

    /// Handle workflow errors
    pub async fn handle_error(&self, error: &anyhow::Error) -> Result<()> {
        error!(
            session_id = %self.session_id,
            error = %error,
            "Session workflow failed"
        );

        // Emit SESSION_ERROR event
        let error_event = AgUiEvent::session_error(error.to_string());
        self.persist_activity
            .persist_event(self.session_id, error_event)
            .await?;

        // Update session finished_at
        self.update_session(None, Some(Utc::now())).await?;

        Ok(())
    }

    /// Update the session timestamps
    async fn update_session(
        &self,
        started_at: Option<chrono::DateTime<Utc>>,
        finished_at: Option<chrono::DateTime<Utc>>,
    ) -> Result<()> {
        let input = UpdateSession {
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
    /// In M2, run_id maps to session_id, agent_id maps to harness_id
    pub async fn legacy_new(
        run_id: Uuid,
        agent_id: Uuid,
        _thread_id: Uuid,
        db: Database,
    ) -> Result<Self> {
        SessionWorkflow::new(run_id, agent_id, db).await
    }
}
