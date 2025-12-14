// Temporal activities for agent execution

use anyhow::Result;
use everruns_contracts::events::{AgUiEvent, MessageRole};
use everruns_contracts::tools::{ToolCall, ToolDefinition, ToolResult};
use everruns_storage::repositories::Database;
use futures::StreamExt;
use reqwest::Client;
use tracing::{error, info, warn};
use uuid::Uuid;

use crate::providers::{ChatMessage, LlmConfig, LlmProvider, LlmStreamEvent};
use crate::tools::{execute_tool, requires_approval};

/// Result from LLM call including text and optional tool calls
#[derive(Debug, Clone)]
pub struct LlmCallResult {
    /// Assistant's text response (may be empty if only tool calls)
    pub text: String,
    /// Tool calls requested by the LLM
    pub tool_calls: Option<Vec<ToolCall>>,
}

/// Activity to persist AG-UI events to the database
pub struct PersistEventActivity {
    db: Database,
}

impl PersistEventActivity {
    pub fn new(db: Database) -> Self {
        Self { db }
    }

    /// Persist an event to the run_events table
    pub async fn persist_event(&self, run_id: Uuid, event: AgUiEvent) -> Result<()> {
        let event_type = match &event {
            AgUiEvent::RunStarted(_) => "RunStarted",
            AgUiEvent::RunFinished(_) => "RunFinished",
            AgUiEvent::RunError(_) => "RunError",
            AgUiEvent::StepStarted(_) => "StepStarted",
            AgUiEvent::StepFinished(_) => "StepFinished",
            AgUiEvent::TextMessageStart(_) => "TextMessageStart",
            AgUiEvent::TextMessageContent(_) => "TextMessageContent",
            AgUiEvent::TextMessageEnd(_) => "TextMessageEnd",
            AgUiEvent::ToolCallStart(_) => "ToolCallStart",
            AgUiEvent::ToolCallArgs(_) => "ToolCallArgs",
            AgUiEvent::ToolCallEnd(_) => "ToolCallEnd",
            AgUiEvent::ToolCallResult(_) => "ToolCallResult",
            AgUiEvent::StateSnapshot(_) => "StateSnapshot",
            AgUiEvent::StateDelta(_) => "StateDelta",
            AgUiEvent::MessagesSnapshot(_) => "MessagesSnapshot",
            AgUiEvent::Custom(_) => "Custom",
        };

        let event_data = serde_json::to_value(&event)?;

        // Insert into run_events table
        sqlx::query(
            r#"
            INSERT INTO run_events (run_id, event_type, event_data)
            VALUES ($1, $2, $3)
            "#,
        )
        .bind(run_id)
        .bind(event_type)
        .bind(event_data)
        .execute(self.db.pool())
        .await?;

        info!(
            run_id = %run_id,
            event_type = %event_type,
            "Persisted event"
        );

        Ok(())
    }
}

/// Activity to call LLM and stream response with AG-UI events
pub struct LlmCallActivity<P: LlmProvider> {
    provider: P,
    persist_activity: PersistEventActivity,
}

impl<P: LlmProvider> LlmCallActivity<P> {
    pub fn new(provider: P, db: Database) -> Self {
        let persist_activity = PersistEventActivity::new(db);
        Self {
            provider,
            persist_activity,
        }
    }

    /// Call LLM with messages and stream response as AG-UI events
    /// Returns the assistant response text and any tool calls
    pub async fn call_and_stream(
        &self,
        run_id: Uuid,
        messages: Vec<ChatMessage>,
        config: LlmConfig,
    ) -> Result<LlmCallResult> {
        info!(
            run_id = %run_id,
            model = %config.model,
            message_count = messages.len(),
            "Starting LLM call"
        );

        // Start streaming from provider
        let mut stream = self
            .provider
            .chat_completion_stream(messages, &config)
            .await?;

        // Emit TEXT_MESSAGE_START event
        let message_id = Uuid::now_v7().to_string();
        let start_event = AgUiEvent::text_message_start(&message_id, MessageRole::Assistant);
        self.persist_activity
            .persist_event(run_id, start_event)
            .await?;

        // Accumulate response and emit chunks
        let mut full_response = String::new();
        let mut tool_calls: Option<Vec<ToolCall>> = None;

        while let Some(event_result) = stream.next().await {
            match event_result {
                Ok(LlmStreamEvent::TextDelta(delta)) => {
                    if !delta.is_empty() {
                        full_response.push_str(&delta);

                        // Emit TEXT_MESSAGE_CONTENT event (AG-UI uses delta, not chunk)
                        let content_event = AgUiEvent::text_message_content(&message_id, &delta);
                        self.persist_activity
                            .persist_event(run_id, content_event)
                            .await?;
                    }
                }
                Ok(LlmStreamEvent::ToolCalls(calls)) => {
                    info!(
                        run_id = %run_id,
                        tool_count = calls.len(),
                        "Tool calls received from LLM"
                    );
                    tool_calls = Some(calls);
                }
                Ok(LlmStreamEvent::Done(metadata)) => {
                    info!(
                        run_id = %run_id,
                        tokens = ?metadata.total_tokens,
                        finish_reason = ?metadata.finish_reason,
                        "LLM call completed"
                    );

                    // Emit TEXT_MESSAGE_END event
                    let end_event = AgUiEvent::text_message_end(&message_id);
                    self.persist_activity
                        .persist_event(run_id, end_event)
                        .await?;

                    break;
                }
                Ok(LlmStreamEvent::Error(err)) => {
                    error!(run_id = %run_id, error = %err, "LLM stream error");
                    anyhow::bail!("LLM stream error: {}", err);
                }
                Err(e) => {
                    error!(run_id = %run_id, error = %e, "Stream processing error");
                    return Err(e);
                }
            }
        }

        Ok(LlmCallResult {
            text: full_response,
            tool_calls,
        })
    }
}

/// Activity to execute tool calls with AG-UI event emission
pub struct ToolExecutionActivity {
    client: Client,
    persist_activity: PersistEventActivity,
}

impl ToolExecutionActivity {
    pub fn new(db: Database) -> Self {
        let persist_activity = PersistEventActivity::new(db);
        let client = Client::new();
        Self {
            client,
            persist_activity,
        }
    }

    /// Execute a single tool call and emit events
    pub async fn execute_tool_call(
        &self,
        run_id: Uuid,
        tool_call: &ToolCall,
        tool_def: &ToolDefinition,
    ) -> Result<ToolResult> {
        // Check if tool requires approval
        if requires_approval(tool_def) {
            warn!(
                run_id = %run_id,
                tool_name = %tool_call.name,
                "Tool requires approval (HITL not implemented yet)"
            );
            // TODO M6+: Implement HITL approval flow
            // For now, reject tools that require approval
            return Ok(ToolResult {
                tool_call_id: tool_call.id.clone(),
                result: None,
                error: Some("Tool requires approval (not implemented)".to_string()),
            });
        }

        // Emit TOOL_CALL_START event
        let start_event = AgUiEvent::tool_call_start(&tool_call.id, &tool_call.name);
        self.persist_activity
            .persist_event(run_id, start_event)
            .await?;

        // Emit TOOL_CALL_ARGS event with the arguments
        let args_json = serde_json::to_string(&tool_call.arguments).unwrap_or_default();
        let args_event = AgUiEvent::tool_call_args(&tool_call.id, args_json);
        self.persist_activity
            .persist_event(run_id, args_event)
            .await?;

        // Emit TOOL_CALL_END event
        let end_event = AgUiEvent::tool_call_end(&tool_call.id);
        self.persist_activity
            .persist_event(run_id, end_event)
            .await?;

        // Execute the tool
        info!(
            run_id = %run_id,
            tool_call_id = %tool_call.id,
            tool_name = %tool_call.name,
            "Executing tool call"
        );

        let result = execute_tool(tool_call, tool_def, &self.client).await;

        // Emit TOOL_CALL_RESULT event
        let result_message_id = Uuid::now_v7().to_string();
        let content = result.result.clone().unwrap_or(serde_json::Value::Null);
        let result_event = AgUiEvent::tool_call_result(&result_message_id, &tool_call.id, content);
        self.persist_activity
            .persist_event(run_id, result_event)
            .await?;

        info!(
            run_id = %run_id,
            tool_call_id = %tool_call.id,
            success = result.error.is_none(),
            "Tool call completed"
        );

        Ok(result)
    }

    /// Execute multiple tool calls in parallel
    pub async fn execute_tool_calls_parallel(
        &self,
        run_id: Uuid,
        tool_calls: &[ToolCall],
        tool_defs: &[ToolDefinition],
    ) -> Result<Vec<ToolResult>> {
        info!(
            run_id = %run_id,
            tool_count = tool_calls.len(),
            "Executing tool calls in parallel"
        );

        // Create a map of tool names to definitions for quick lookup
        let tool_map: std::collections::HashMap<String, &ToolDefinition> = tool_defs
            .iter()
            .map(|def| {
                let name = match def {
                    ToolDefinition::Webhook(w) => &w.name,
                    ToolDefinition::Builtin(b) => &b.name,
                };
                (name.clone(), def)
            })
            .collect();

        // Execute all tool calls in parallel using tokio::join_all
        let futures: Vec<_> = tool_calls
            .iter()
            .map(|tool_call| async {
                let tool_def = tool_map.get(&tool_call.name).ok_or_else(|| {
                    anyhow::anyhow!("Tool definition not found for: {}", tool_call.name)
                })?;

                self.execute_tool_call(run_id, tool_call, tool_def).await
            })
            .collect();

        let results = futures::future::join_all(futures).await;

        // Convert Vec<Result<ToolResult>> to Result<Vec<ToolResult>>
        // Collect all results, including errors
        let tool_results: Vec<ToolResult> = results
            .into_iter()
            .map(|r| match r {
                Ok(result) => result,
                Err(e) => ToolResult {
                    tool_call_id: String::new(), // Will be set by caller if needed
                    result: None,
                    error: Some(e.to_string()),
                },
            })
            .collect();

        Ok(tool_results)
    }
}
