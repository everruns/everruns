// Temporal activity implementations (M2)
// Decision: Activities are standalone functions that can be registered with Temporal
//
// These activities handle the actual work of session execution:
// - Loading data from database
// - Calling LLMs (with streaming and heartbeats)
// - Executing tools
// - Persisting events
//
// All activities must be idempotent and handle their own error scenarios.

use anyhow::{Context, Result};
use everruns_contracts::events::AgUiEvent;
use everruns_storage::models::UpdateSession;
use everruns_storage::repositories::Database;
use tracing::info;
use uuid::Uuid;

use crate::activities::PersistEventActivity;
use crate::providers::openai::OpenAiProvider;
use crate::providers::{ChatMessage, LlmConfig, LlmProvider, MessageRole as ProviderMessageRole};

use super::types::*;

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

    Ok(LoadAgentOutput {
        agent_id: agent.id,
        name: agent.name,
        model_id: agent
            .default_model_id
            .map(|id| id.to_string())
            .unwrap_or_else(|| "gpt-4o".to_string()),
        system_prompt: Some(agent.system_prompt),
        temperature: None,
        max_tokens: None,
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
        .filter_map(|m| {
            // Extract text content from JSON
            let content = if let Some(text) = m.content.get("text").and_then(|t| t.as_str()) {
                text.to_string()
            } else if let Some(content_str) = m.content.as_str() {
                content_str.to_string()
            } else {
                return None;
            };

            Some(MessageData {
                role: m.role,
                content,
            })
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
        "Starting LLM call activity"
    );

    // Heartbeat to indicate we're starting
    ctx.heartbeat("Starting LLM call");

    // Convert message data to ChatMessage format
    let messages: Vec<ChatMessage> = input
        .messages
        .iter()
        .map(|m| ChatMessage {
            role: match m.role.as_str() {
                "system" => ProviderMessageRole::System,
                "user" => ProviderMessageRole::User,
                "assistant" => ProviderMessageRole::Assistant,
                "tool" | "tool_result" => ProviderMessageRole::Tool,
                _ => ProviderMessageRole::User,
            },
            content: m.content.clone(),
            tool_calls: None,
            tool_call_id: None,
        })
        .collect();

    // Build LLM config
    let config = LlmConfig {
        model: input.model_id.clone(),
        temperature: input.temperature,
        max_tokens: input.max_tokens,
        system_prompt: input.system_prompt.clone(),
        tools: Vec::new(),
    };

    // Create provider
    let provider = OpenAiProvider::new().context("Failed to create OpenAI provider")?;

    // Heartbeat before LLM call
    ctx.heartbeat("Calling LLM...");

    // Non-streaming call
    let result = provider
        .chat_completion(messages, &config)
        .await
        .context("LLM call failed")?;

    info!(
        session_id = %input.session_id,
        tokens = ?result.metadata.total_tokens,
        finish_reason = ?result.metadata.finish_reason,
        "LLM call completed"
    );

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
            })
            .collect()
    });

    Ok(CallLlmOutput {
        text: result.text,
        tool_calls: output_tool_calls,
    })
}

/// Execute tool calls
pub async fn execute_tools_activity(
    ctx: &ActivityContext,
    db: &Database,
    input: ExecuteToolsInput,
) -> Result<ExecuteToolsOutput> {
    info!(
        session_id = %input.session_id,
        tool_count = input.tool_calls.len(),
        "Executing tool calls"
    );

    let persist_activity = PersistEventActivity::new(db.clone());
    let mut results = Vec::new();

    for (i, tool_call_data) in input.tool_calls.iter().enumerate() {
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

        // For now, return a placeholder result
        // In a real implementation, we'd execute the actual tool
        let result = ToolResultData {
            tool_call_id: tool_call_data.id.clone(),
            result: Some(serde_json::json!({
                "status": "tool_execution_not_implemented",
                "tool_name": tool_call_data.name
            })),
            error: None,
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
        "Saving message to session"
    );

    let create_msg = everruns_storage::models::CreateMessage {
        session_id: input.session_id,
        role: input.role,
        content: input.content,
        tool_call_id: None,
    };

    db.create_message(create_msg)
        .await
        .context("Database error saving message")?;

    Ok(())
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
