// Activity implementations for workflow execution
// Decision: Activities are standalone functions that can be registered with Temporal
// Decision: Each LLM call and each tool execution is a separate Temporal activity (node)
//
// These activities handle the actual work of session execution:
// - Loading data from database
// - Calling LLMs (with streaming and heartbeats)
// - Executing tools (each tool = separate activity)
// - Persisting events
//
// All activities must be idempotent and handle their own error scenarios.

use anyhow::{Context, Result};
use everruns_contracts::events::AgUiEvent;
use everruns_contracts::tools::ToolDefinition;
use everruns_core::capabilities::{apply_capabilities, CapabilityId, CapabilityRegistry};
use everruns_core::config::AgentConfig;
use everruns_core::traits::{LlmCallConfig, LlmMessage, LlmMessageRole, LlmProvider, ToolExecutor};
use everruns_openai::OpenAiProvider;
use everruns_storage::models::UpdateSession;
use everruns_storage::repositories::Database;
use tracing::{debug, info, warn};
use uuid::Uuid;

use crate::types::*;
use crate::unified_tool_executor::UnifiedToolExecutor;

// =============================================================================
// Activity Context
// =============================================================================

/// Activity context for heartbeat reporting
/// In the real Temporal SDK, this would be provided by the runtime
pub struct ActivityContext {
    /// Task token for this activity (used for heartbeats)
    #[allow(dead_code)]
    task_token: Vec<u8>,
    /// Function to report heartbeat progress
    heartbeat_fn: Option<Box<dyn Fn(String) + Send + Sync>>,
}

impl ActivityContext {
    pub fn new(task_token: Vec<u8>) -> Self {
        Self {
            task_token,
            heartbeat_fn: None,
        }
    }

    /// Set the heartbeat function
    #[allow(dead_code)]
    pub fn with_heartbeat<F>(mut self, f: F) -> Self
    where
        F: Fn(String) + Send + Sync + 'static,
    {
        self.heartbeat_fn = Some(Box::new(f));
        self
    }

    /// Report progress (heartbeat)
    pub fn heartbeat(&self, details: &str) {
        if let Some(f) = &self.heartbeat_fn {
            f(details.to_string());
        }
    }
}

// =============================================================================
// Event Persistence
// =============================================================================

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

// =============================================================================
// Workflow Activities
// =============================================================================

/// Load agent configuration from database
pub async fn load_agent_activity(
    _ctx: &ActivityContext,
    db: &Database,
    input: LoadAgentInput,
) -> Result<LoadAgentOutput> {
    info!(agent_id = %input.agent_id, "Loading agent configuration");

    let agent = db
        .get_agent(input.agent_id)
        .await
        .context("Database error loading agent")?
        .ok_or_else(|| anyhow::anyhow!("Agent not found: {}", input.agent_id))?;

    // Load capabilities for this agent
    let capabilities = db
        .get_agent_capabilities(input.agent_id)
        .await
        .context("Database error loading agent capabilities")?;

    let capability_ids: Vec<String> = capabilities.into_iter().map(|c| c.capability_id).collect();

    info!(
        agent_id = %input.agent_id,
        capability_count = capability_ids.len(),
        capabilities = ?capability_ids,
        "Loaded agent with capabilities"
    );

    Ok(LoadAgentOutput {
        agent_id: agent.id,
        name: agent.name,
        model_id: agent
            .default_model_id
            .map(|id| id.to_string())
            .unwrap_or_else(|| "gpt-5.2".to_string()),
        system_prompt: Some(agent.system_prompt),
        temperature: None,
        max_tokens: None,
        capability_ids,
    })
}

/// Load messages from a session
pub async fn load_messages_activity(
    _ctx: &ActivityContext,
    db: &Database,
    input: LoadMessagesInput,
) -> Result<LoadMessagesOutput> {
    info!(session_id = %input.session_id, "Loading session messages");

    let messages = db
        .list_messages(input.session_id)
        .await
        .context("Database error loading messages")?;

    let message_data: Vec<MessageData> = messages
        .into_iter()
        .map(|m| {
            // Extract text content from JSON
            let content = if let Some(text) = m.content.get("text").and_then(|t| t.as_str()) {
                text.to_string()
            } else if let Some(content_str) = m.content.as_str() {
                content_str.to_string()
            } else {
                // For tool messages, the content might be in a different format
                // Try to get the raw content as a string
                m.content.to_string()
            };

            // Extract tool_calls from assistant messages (stored in content JSON)
            let tool_calls = if m.role == "assistant" {
                m.content
                    .get("tool_calls")
                    .and_then(|tc| serde_json::from_value::<Vec<ToolCallData>>(tc.clone()).ok())
            } else {
                None
            };

            // Get tool_call_id for tool result messages
            let tool_call_id = m.tool_call_id.clone();

            MessageData {
                role: m.role,
                content,
                tool_calls,
                tool_call_id,
            }
        })
        .collect();

    info!(
        session_id = %input.session_id,
        message_count = message_data.len(),
        "Loaded messages"
    );

    Ok(LoadMessagesOutput {
        messages: message_data,
    })
}

/// Update session status in database
pub async fn update_status_activity(
    _ctx: &ActivityContext,
    db: &Database,
    input: UpdateStatusInput,
) -> Result<()> {
    info!(
        session_id = %input.session_id,
        status = %input.status,
        "Updating session status"
    );

    let update = UpdateSession {
        status: Some(input.status.clone()),
        started_at: input.started_at,
        finished_at: input.finished_at,
        ..Default::default()
    };

    db.update_session(input.session_id, update)
        .await
        .context("Database error updating session status")?;

    Ok(())
}

/// Persist an AG-UI event to the database
pub async fn persist_event_activity(
    _ctx: &ActivityContext,
    db: &Database,
    input: PersistEventInput,
) -> Result<()> {
    let event: AgUiEvent = serde_json::from_value(input.event_data.clone())
        .context("Failed to deserialize event data")?;

    let persist_activity = PersistEventActivity::new(db.clone());
    persist_activity
        .persist_event(input.session_id, event)
        .await?;

    Ok(())
}

/// Call LLM and return response (non-streaming for M2)
/// This is a long-running activity that uses heartbeats
pub async fn call_llm_activity(
    ctx: &ActivityContext,
    db: &Database,
    input: CallLlmInput,
) -> Result<CallLlmOutput> {
    info!(
        session_id = %input.session_id,
        model = %input.model_id,
        message_count = input.messages.len(),
        capability_count = input.capability_ids.len(),
        "Starting LLM call activity"
    );

    // Heartbeat to indicate we're starting
    ctx.heartbeat("Starting LLM call");

    // Apply capabilities to get tools and system prompt modifications
    let registry = CapabilityRegistry::with_builtins();
    let capability_ids: Vec<CapabilityId> = input
        .capability_ids
        .iter()
        .filter_map(|id| {
            let parsed = id.parse::<CapabilityId>();
            if parsed.is_err() {
                warn!(capability_id = %id, "Unknown capability ID, skipping");
            }
            parsed.ok()
        })
        .collect();

    // Build base agent config
    let base_system_prompt = input.system_prompt.clone().unwrap_or_default();
    let base_config = AgentConfig::new(&base_system_prompt, &input.model_id);

    // Apply capabilities to get tools and modified system prompt
    let applied = apply_capabilities(base_config, &capability_ids, &registry);

    info!(
        session_id = %input.session_id,
        applied_capabilities = ?applied.applied_ids,
        tool_count = applied.config.tools.len(),
        "Applied capabilities"
    );

    // Debug log the tools that will be sent to LLM
    for tool in &applied.config.tools {
        debug!(
            session_id = %input.session_id,
            tool_name = %tool_name(tool),
            tool_description = %tool_description(tool),
            "Tool available for LLM"
        );
    }

    // Convert message data to LlmMessage format (core types)
    let mut messages: Vec<LlmMessage> = input
        .messages
        .iter()
        .map(|m| {
            // Convert ToolCallData to ToolCall (different argument types)
            let tool_calls = m.tool_calls.as_ref().map(|calls| {
                calls
                    .iter()
                    .map(|tc| everruns_contracts::tools::ToolCall {
                        id: tc.id.clone(),
                        name: tc.name.clone(),
                        // Parse the JSON string arguments back to Value
                        arguments: serde_json::from_str(&tc.arguments)
                            .unwrap_or(serde_json::Value::Object(serde_json::Map::new())),
                    })
                    .collect()
            });

            LlmMessage {
                role: match m.role.as_str() {
                    "system" => LlmMessageRole::System,
                    "user" => LlmMessageRole::User,
                    "assistant" => LlmMessageRole::Assistant,
                    "tool" | "tool_result" => LlmMessageRole::Tool,
                    _ => LlmMessageRole::User,
                },
                content: m.content.clone(),
                tool_calls,
                tool_call_id: m.tool_call_id.clone(),
            }
        })
        .collect();

    // Prepend system prompt as first message if not already present
    if !messages.iter().any(|m| m.role == LlmMessageRole::System) {
        messages.insert(
            0,
            LlmMessage {
                role: LlmMessageRole::System,
                content: applied.config.system_prompt.clone(),
                tool_calls: None,
                tool_call_id: None,
            },
        );
    }

    // Build LLM config with capability-provided tools (core types)
    // applied.config.tools is already Vec<ToolDefinition> from capabilities
    let config = LlmCallConfig {
        model: input.model_id.clone(),
        temperature: input.temperature,
        max_tokens: input.max_tokens,
        tools: applied.config.tools.clone(),
    };

    // Create provider
    let provider = OpenAiProvider::new().context("Failed to create OpenAI provider")?;

    // Heartbeat before LLM call
    ctx.heartbeat("Calling LLM...");

    // Non-streaming call
    let result = match provider.chat_completion(messages, &config).await {
        Ok(r) => r,
        Err(e) => {
            // Log the error for debugging
            tracing::error!(
                session_id = %input.session_id,
                error = %e,
                model = %input.model_id,
                "LLM call failed"
            );
            // Return a detailed error message that will be visible in Temporal and potentially UI
            return Err(anyhow::anyhow!("LLM call failed: {}", e));
        }
    };

    info!(
        session_id = %input.session_id,
        tokens = ?result.metadata.total_tokens,
        finish_reason = ?result.metadata.finish_reason,
        "LLM call completed"
    );

    // Debug log the response text
    debug!(
        session_id = %input.session_id,
        text_len = result.text.len(),
        has_tool_calls = result.tool_calls.is_some(),
        "LLM response details"
    );

    // Debug log tool calls if present
    if let Some(ref tool_calls) = result.tool_calls {
        for tc in tool_calls {
            debug!(
                session_id = %input.session_id,
                tool_call_id = %tc.id,
                tool_name = %tc.name,
                arguments = %tc.arguments,
                "LLM requested tool call"
            );
        }
    }

    // Emit step events
    let persist_activity = PersistEventActivity::new(db.clone());
    let step_event = AgUiEvent::step_finished("llm_call".to_string());
    persist_activity
        .persist_event(input.session_id, step_event)
        .await?;

    // Convert tool calls to output format
    let output_tool_calls = result.tool_calls.map(|calls| {
        calls
            .into_iter()
            .map(|tc| ToolCallData {
                id: tc.id,
                name: tc.name,
                arguments: serde_json::to_string(&tc.arguments).unwrap_or_default(),
                tool_definition_json: None,
            })
            .collect()
    });

    Ok(CallLlmOutput {
        text: result.text,
        tool_calls: output_tool_calls,
    })
}

/// Execute tool calls using the UnifiedToolExecutor
///
/// This activity executes tool calls using the UnifiedToolExecutor
/// which uses ToolRegistry for consistent tool execution.
pub async fn execute_tools_activity(
    ctx: &ActivityContext,
    db: &Database,
    input: ExecuteToolsInput,
) -> Result<ExecuteToolsOutput> {
    info!(
        session_id = %input.session_id,
        tool_count = input.tool_calls.len(),
        "Executing tool calls via UnifiedToolExecutor"
    );

    // Debug log each tool call to be executed
    for tc in &input.tool_calls {
        debug!(
            session_id = %input.session_id,
            tool_call_id = %tc.id,
            tool_name = %tc.name,
            arguments = %tc.arguments,
            "Tool call to execute"
        );
    }

    let persist_activity = PersistEventActivity::new(db.clone());
    let tool_executor = UnifiedToolExecutor::with_default_tools();
    let mut results = Vec::new();

    for (i, tool_call_data) in input.tool_calls.iter().enumerate() {
        debug!(
            session_id = %input.session_id,
            tool_index = i + 1,
            total_tools = input.tool_calls.len(),
            tool_name = %tool_call_data.name,
            "Starting tool execution"
        );

        // Heartbeat progress
        ctx.heartbeat(&format!(
            "Executing tool {}/{}: {}",
            i + 1,
            input.tool_calls.len(),
            tool_call_data.name
        ));

        // Emit TOOL_CALL_START event
        let start_event = AgUiEvent::tool_call_start(&tool_call_data.id, &tool_call_data.name);
        persist_activity
            .persist_event(input.session_id, start_event)
            .await?;

        // Emit TOOL_CALL_ARGS event
        let args_event =
            AgUiEvent::tool_call_args(&tool_call_data.id, tool_call_data.arguments.clone());
        persist_activity
            .persist_event(input.session_id, args_event)
            .await?;

        // Emit TOOL_CALL_END event
        let end_event = AgUiEvent::tool_call_end(&tool_call_data.id);
        persist_activity
            .persist_event(input.session_id, end_event)
            .await?;

        // Parse arguments from JSON string
        let arguments: serde_json::Value =
            serde_json::from_str(&tool_call_data.arguments).unwrap_or(serde_json::json!({}));

        // Create ToolCall for execution
        let tool_call = everruns_contracts::tools::ToolCall {
            id: tool_call_data.id.clone(),
            name: tool_call_data.name.clone(),
            arguments,
        };

        // Parse tool definition if provided, otherwise create a placeholder
        let tool_def: ToolDefinition = if let Some(ref json) = tool_call_data.tool_definition_json {
            serde_json::from_str(json).unwrap_or_else(|_| {
                ToolDefinition::Builtin(everruns_contracts::tools::BuiltinTool {
                    name: tool_call_data.name.clone(),
                    description: "Tool execution".to_string(),
                    kind: everruns_contracts::tools::BuiltinToolKind::HttpGet,
                    policy: everruns_contracts::tools::ToolPolicy::Auto,
                    parameters: serde_json::json!({}),
                })
            })
        } else {
            // Default to builtin tool - UnifiedToolExecutor will look it up in registry
            ToolDefinition::Builtin(everruns_contracts::tools::BuiltinTool {
                name: tool_call_data.name.clone(),
                description: "Tool execution".to_string(),
                kind: everruns_contracts::tools::BuiltinToolKind::HttpGet,
                policy: everruns_contracts::tools::ToolPolicy::Auto,
                parameters: serde_json::json!({}),
            })
        };

        // Execute the tool using the ToolExecutor trait
        let exec_result = tool_executor
            .execute(&tool_call, &tool_def)
            .await
            .map_err(|e| anyhow::anyhow!("Tool execution failed: {}", e))?;

        // Debug log the tool execution result
        debug!(
            session_id = %input.session_id,
            tool_call_id = %exec_result.tool_call_id,
            has_result = exec_result.result.is_some(),
            has_error = exec_result.error.is_some(),
            result = ?exec_result.result,
            error = ?exec_result.error,
            "Tool execution completed"
        );

        let result = ToolResultData {
            tool_call_id: exec_result.tool_call_id,
            result: exec_result.result,
            error: exec_result.error,
        };

        // Emit TOOL_CALL_RESULT event
        let result_message_id = Uuid::now_v7().to_string();
        let result_event = AgUiEvent::tool_call_result(
            &result_message_id,
            &tool_call_data.id,
            result.result.clone().unwrap_or_default(),
        );
        persist_activity
            .persist_event(input.session_id, result_event)
            .await?;

        results.push(result);
    }

    Ok(ExecuteToolsOutput { results })
}

/// Save a message to the session
pub async fn save_message_activity(
    _ctx: &ActivityContext,
    db: &Database,
    input: SaveMessageInput,
) -> Result<()> {
    info!(
        session_id = %input.session_id,
        role = %input.role,
        tool_call_id = ?input.tool_call_id,
        "Saving message to session"
    );

    let create_msg = everruns_storage::models::CreateMessage {
        session_id: input.session_id,
        role: input.role,
        content: input.content,
        tool_call_id: input.tool_call_id,
    };

    db.create_message(create_msg)
        .await
        .context("Database error saving message")?;

    Ok(())
}

// =============================================================================
// Helper functions for logging
// =============================================================================

/// Extract tool name from ToolDefinition enum
fn tool_name(tool: &ToolDefinition) -> &str {
    match tool {
        ToolDefinition::Builtin(b) => &b.name,
    }
}

/// Extract tool description from ToolDefinition enum
fn tool_description(tool: &ToolDefinition) -> &str {
    match tool {
        ToolDefinition::Builtin(b) => &b.description,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_activity_context_heartbeat() {
        let ctx = ActivityContext::new(vec![1, 2, 3]);
        // Should not panic even without heartbeat function
        ctx.heartbeat("test");
    }

    #[test]
    fn test_activity_context_with_heartbeat() {
        use std::sync::atomic::{AtomicUsize, Ordering};
        use std::sync::Arc;

        let counter = Arc::new(AtomicUsize::new(0));
        let counter_clone = counter.clone();

        let ctx = ActivityContext::new(vec![1, 2, 3]).with_heartbeat(move |_| {
            counter_clone.fetch_add(1, Ordering::SeqCst);
        });

        ctx.heartbeat("test1");
        ctx.heartbeat("test2");

        assert_eq!(counter.load(Ordering::SeqCst), 2);
    }
}
