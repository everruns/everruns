// Temporal activity implementations
// Decision: Activities are standalone functions that can be registered with Temporal
//
// These activities handle the actual work of agent execution:
// - Loading data from database
// - Calling LLMs (with streaming and heartbeats)
// - Executing tools
// - Persisting events
//
// All activities must be idempotent and handle their own error scenarios.

use anyhow::{Context, Result};
use everruns_contracts::events::{AgUiEvent, MessageRole};
use everruns_contracts::tools::ToolCall;
use everruns_storage::models::UpdateRun;
use everruns_storage::repositories::Database;
use futures::StreamExt;
use tracing::{error, info};
use uuid::Uuid;

use crate::activities::PersistEventActivity;
use crate::providers::openai::OpenAiProvider;
use crate::providers::{
    ChatMessage, LlmConfig, LlmProvider, LlmStreamEvent, MessageRole as ProviderMessageRole,
};
use crate::temporal_types::*;

/// Activity context for heartbeat reporting
/// In the real Temporal SDK, this would be provided by the runtime
pub struct ActivityContext {
    /// Task token for this activity (used for heartbeats)
    pub task_token: Vec<u8>,
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

    let definition = &agent.definition;

    Ok(LoadAgentOutput {
        agent_id: agent.id,
        name: agent.name,
        model_id: agent.default_model_id,
        system_prompt: definition
            .get("system")
            .and_then(|v| v.as_str())
            .map(|s| s.to_string()),
        temperature: definition
            .get("temperature")
            .and_then(|v| v.as_f64())
            .map(|f| f as f32),
        max_tokens: definition
            .get("max_tokens")
            .and_then(|v| v.as_u64())
            .map(|u| u as u32),
    })
}

/// Load messages from a thread
pub async fn load_messages_activity(
    _ctx: &ActivityContext,
    db: &Database,
    input: LoadMessagesInput,
) -> Result<LoadMessagesOutput> {
    info!(thread_id = %input.thread_id, "Loading thread messages");

    let messages = db
        .list_messages(input.thread_id)
        .await
        .context("Database error loading messages")?;

    let message_data: Vec<MessageData> = messages
        .into_iter()
        .map(|m| MessageData {
            role: m.role,
            content: m.content,
        })
        .collect();

    info!(
        thread_id = %input.thread_id,
        message_count = message_data.len(),
        "Loaded messages"
    );

    Ok(LoadMessagesOutput {
        messages: message_data,
    })
}

/// Update run status in database
pub async fn update_status_activity(
    _ctx: &ActivityContext,
    db: &Database,
    input: UpdateStatusInput,
) -> Result<()> {
    info!(
        run_id = %input.run_id,
        status = %input.status,
        "Updating run status"
    );

    let update = UpdateRun {
        status: Some(input.status.clone()),
        temporal_workflow_id: None,
        temporal_run_id: None,
        started_at: input.started_at,
        finished_at: input.finished_at,
    };

    db.update_run(input.run_id, update)
        .await
        .context("Database error updating run status")?;

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
    persist_activity.persist_event(input.run_id, event).await?;

    Ok(())
}

/// Call LLM and stream response with event persistence
/// This is a long-running activity that uses heartbeats
pub async fn call_llm_activity(
    ctx: &ActivityContext,
    db: &Database,
    input: CallLlmInput,
) -> Result<CallLlmOutput> {
    info!(
        run_id = %input.run_id,
        model = %input.model_id,
        message_count = input.messages.len(),
        "Starting LLM call activity"
    );

    // Convert message data to ChatMessage format
    let messages: Vec<ChatMessage> = input
        .messages
        .iter()
        .map(|m| ChatMessage {
            role: match m.role.as_str() {
                "system" => ProviderMessageRole::System,
                "user" => ProviderMessageRole::User,
                "assistant" => ProviderMessageRole::Assistant,
                "tool" => ProviderMessageRole::Tool,
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
        tools: Vec::new(), // Tools will be added from agent definition
    };

    // Create provider
    let provider = OpenAiProvider::new().context("Failed to create OpenAI provider")?;

    // Start streaming
    let mut stream = provider
        .chat_completion_stream(messages, &config)
        .await
        .context("Failed to start LLM stream")?;

    // Create persist activity
    let persist_activity = PersistEventActivity::new(db.clone());

    // Emit TEXT_MESSAGE_START event
    let message_id = Uuid::now_v7().to_string();
    let start_event = AgUiEvent::text_message_start(&message_id, MessageRole::Assistant);
    persist_activity
        .persist_event(input.run_id, start_event)
        .await?;

    // Accumulate response and emit chunks
    let mut full_response = String::new();
    let mut tool_calls: Option<Vec<ToolCall>> = None;
    let mut chunk_count = 0;

    while let Some(event_result) = stream.next().await {
        match event_result {
            Ok(LlmStreamEvent::TextDelta(delta)) => {
                if !delta.is_empty() {
                    full_response.push_str(&delta);
                    chunk_count += 1;

                    // Emit TEXT_MESSAGE_CONTENT event
                    let content_event = AgUiEvent::text_message_content(&message_id, &delta);
                    persist_activity
                        .persist_event(input.run_id, content_event)
                        .await?;

                    // Heartbeat every 10 chunks to indicate progress
                    if chunk_count % 10 == 0 {
                        ctx.heartbeat(&format!(
                            "Streaming LLM response: {} tokens",
                            full_response.len()
                        ));
                    }
                }
            }
            Ok(LlmStreamEvent::ToolCalls(calls)) => {
                info!(
                    run_id = %input.run_id,
                    tool_count = calls.len(),
                    "Tool calls received from LLM"
                );
                tool_calls = Some(calls);
            }
            Ok(LlmStreamEvent::Done(metadata)) => {
                info!(
                    run_id = %input.run_id,
                    tokens = ?metadata.total_tokens,
                    finish_reason = ?metadata.finish_reason,
                    "LLM call completed"
                );

                // Emit TEXT_MESSAGE_END event
                let end_event = AgUiEvent::text_message_end(&message_id);
                persist_activity
                    .persist_event(input.run_id, end_event)
                    .await?;

                break;
            }
            Ok(LlmStreamEvent::Error(err)) => {
                error!(run_id = %input.run_id, error = %err, "LLM stream error");
                return Err(anyhow::anyhow!("LLM stream error: {}", err));
            }
            Err(e) => {
                error!(run_id = %input.run_id, error = %e, "Stream processing error");
                return Err(e);
            }
        }
    }

    // Convert tool calls to output format
    let output_tool_calls = tool_calls.map(|calls| {
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
        text: full_response,
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
        run_id = %input.run_id,
        tool_count = input.tool_calls.len(),
        "Executing tool calls"
    );

    let persist_activity = PersistEventActivity::new(db.clone());
    let mut results = Vec::new();

    for (i, tool_call_data) in input.tool_calls.iter().enumerate() {
        // Convert to ToolCall
        let tool_call = ToolCall {
            id: tool_call_data.id.clone(),
            name: tool_call_data.name.clone(),
            arguments: serde_json::from_str(&tool_call_data.arguments).unwrap_or_default(),
        };

        // Heartbeat progress
        ctx.heartbeat(&format!(
            "Executing tool {}/{}: {}",
            i + 1,
            input.tool_calls.len(),
            tool_call.name
        ));

        // Emit TOOL_CALL_START event
        let start_event = AgUiEvent::tool_call_start(&tool_call.id, &tool_call.name);
        persist_activity
            .persist_event(input.run_id, start_event)
            .await?;

        // Emit TOOL_CALL_ARGS event
        let args_event = AgUiEvent::tool_call_args(&tool_call.id, tool_call_data.arguments.clone());
        persist_activity
            .persist_event(input.run_id, args_event)
            .await?;

        // Emit TOOL_CALL_END event
        let end_event = AgUiEvent::tool_call_end(&tool_call.id);
        persist_activity
            .persist_event(input.run_id, end_event)
            .await?;

        // For now, we don't have tool definitions available in this context
        // In a real implementation, we'd load them from the agent definition
        // For now, return a placeholder result
        let result = ToolResultData {
            tool_call_id: tool_call.id.clone(),
            result: Some(serde_json::json!({
                "status": "tool_execution_not_implemented",
                "tool_name": tool_call.name
            })),
            error: None,
        };

        // Emit TOOL_CALL_RESULT event
        let result_message_id = Uuid::now_v7().to_string();
        let result_event = AgUiEvent::tool_call_result(
            &result_message_id,
            &tool_call.id,
            result.result.clone().unwrap_or_default(),
        );
        persist_activity
            .persist_event(input.run_id, result_event)
            .await?;

        results.push(result);
    }

    Ok(ExecuteToolsOutput { results })
}

/// Save a message to the thread
pub async fn save_message_activity(
    _ctx: &ActivityContext,
    db: &Database,
    input: SaveMessageInput,
) -> Result<()> {
    info!(
        thread_id = %input.thread_id,
        role = %input.role,
        "Saving message to thread"
    );

    let create_msg = everruns_storage::models::CreateMessage {
        thread_id: input.thread_id,
        role: input.role,
        content: input.content,
        metadata: None,
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
