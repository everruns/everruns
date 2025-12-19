// Session activities for workflow execution (M2)

use anyhow::Result;
use everruns_contracts::events::AgUiEvent;
use everruns_contracts::tools::{ToolCall, ToolDefinition, ToolResult};
use everruns_core::traits::{LlmCallConfig, LlmMessage, LlmProvider};
use everruns_storage::repositories::Database;
use reqwest::Client;
use tracing::{info, warn};
use uuid::Uuid;

use crate::tools::{execute_tool, requires_approval};

/// Result from LLM call including text and optional tool calls
#[derive(Debug, Clone)]
pub struct LlmCallResult {
    /// Assistant's text response (may be empty if only tool calls)
    pub text: String,
    /// Tool calls requested by the LLM
    pub tool_calls: Option<Vec<ToolCall>>,
}

/// Activity to persist AG-UI events to the session_events table
pub struct PersistEventActivity {
    db: Database,
}

impl PersistEventActivity {
    pub fn new(db: Database) -> Self {
        Self { db }
    }

    /// Persist an event to the session_events table
    pub async fn persist_event(&self, session_id: Uuid, event: AgUiEvent) -> Result<()> {
        let event_type = match &event {
            AgUiEvent::RunStarted(_) => "session.started",
            AgUiEvent::RunFinished(_) => "session.finished",
            AgUiEvent::RunError(_) => "session.error",
            AgUiEvent::StepStarted(_) => "step.started",
            AgUiEvent::StepFinished(_) => "step.finished",
            AgUiEvent::TextMessageStart(_) => "text.start",
            AgUiEvent::TextMessageContent(_) => "text.delta",
            AgUiEvent::TextMessageEnd(_) => "text.end",
            AgUiEvent::ToolCallStart(_) => "tool.call.start",
            AgUiEvent::ToolCallArgs(_) => "tool.call.args",
            AgUiEvent::ToolCallEnd(_) => "tool.call.end",
            AgUiEvent::ToolCallResult(_) => "tool.result",
            AgUiEvent::StateSnapshot(_) => "state.snapshot",
            AgUiEvent::StateDelta(_) => "state.delta",
            AgUiEvent::MessagesSnapshot(_) => "messages.snapshot",
            AgUiEvent::Custom(_) => "custom",
        };

        let event_data = serde_json::to_value(&event)?;

        // Insert into events table with auto-incrementing sequence
        sqlx::query(
            r#"
            INSERT INTO events (session_id, sequence, event_type, data)
            VALUES ($1, COALESCE((SELECT MAX(sequence) + 1 FROM events WHERE session_id = $1), 1), $2, $3)
            "#,
        )
        .bind(session_id)
        .bind(event_type)
        .bind(event_data)
        .execute(self.db.pool())
        .await?;

        info!(
            session_id = %session_id,
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

    /// Call LLM with messages (non-streaming)
    /// Returns the assistant response text and any tool calls
    pub async fn call(
        &self,
        session_id: Uuid,
        messages: Vec<LlmMessage>,
        config: LlmCallConfig,
    ) -> Result<LlmCallResult> {
        info!(
            session_id = %session_id,
            model = %config.model,
            message_count = messages.len(),
            "Starting LLM call"
        );

        // Emit step started event
        let step_event = AgUiEvent::step_started("llm_call".to_string());
        self.persist_activity
            .persist_event(session_id, step_event)
            .await?;

        // Call LLM (non-streaming)
        let result = self.provider.chat_completion(messages, &config).await?;

        info!(
            session_id = %session_id,
            tokens = ?result.metadata.total_tokens,
            finish_reason = ?result.metadata.finish_reason,
            response_len = result.text.len(),
            tool_calls = result.tool_calls.as_ref().map(|c| c.len()).unwrap_or(0),
            "LLM call completed"
        );

        // Emit step finished event
        let step_finished_event = AgUiEvent::step_finished("llm_call".to_string());
        self.persist_activity
            .persist_event(session_id, step_finished_event)
            .await?;

        Ok(LlmCallResult {
            text: result.text,
            tool_calls: result.tool_calls,
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
        session_id: Uuid,
        tool_call: &ToolCall,
        tool_def: &ToolDefinition,
    ) -> Result<ToolResult> {
        // Check if tool requires approval
        if requires_approval(tool_def) {
            warn!(
                session_id = %session_id,
                tool_name = %tool_call.name,
                "Tool requires approval (HITL not implemented yet)"
            );
            // TODO: Implement HITL approval flow
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
            .persist_event(session_id, start_event)
            .await?;

        // Emit TOOL_CALL_ARGS event with the arguments
        let args_json = serde_json::to_string(&tool_call.arguments).unwrap_or_default();
        let args_event = AgUiEvent::tool_call_args(&tool_call.id, args_json);
        self.persist_activity
            .persist_event(session_id, args_event)
            .await?;

        // Emit TOOL_CALL_END event
        let end_event = AgUiEvent::tool_call_end(&tool_call.id);
        self.persist_activity
            .persist_event(session_id, end_event)
            .await?;

        // Execute the tool
        info!(
            session_id = %session_id,
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
            .persist_event(session_id, result_event)
            .await?;

        info!(
            session_id = %session_id,
            tool_call_id = %tool_call.id,
            success = result.error.is_none(),
            "Tool call completed"
        );

        Ok(result)
    }

    /// Execute multiple tool calls in parallel
    pub async fn execute_tool_calls_parallel(
        &self,
        session_id: Uuid,
        tool_calls: &[ToolCall],
        tool_defs: &[ToolDefinition],
    ) -> Result<Vec<ToolResult>> {
        info!(
            session_id = %session_id,
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

                self.execute_tool_call(session_id, tool_call, tool_def)
                    .await
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
