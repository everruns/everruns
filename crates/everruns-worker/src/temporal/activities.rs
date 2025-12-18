// Temporal activity implementations (M2)
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
use everruns_agent_loop::config::AgentConfig;
use everruns_agent_loop::memory::NoOpEventEmitter;
use everruns_agent_loop::step::{LoopStep, StepInput, StepResult};
use everruns_agent_loop::AgentLoop;
use everruns_contracts::events::AgUiEvent;
use everruns_contracts::tools::ToolDefinition;
use everruns_storage::models::UpdateSession;
use everruns_storage::repositories::Database;
use tracing::info;
use uuid::Uuid;

use crate::activities::PersistEventActivity;
use crate::adapters::{DbMessageStore, OpenAiLlmAdapter};
use crate::providers::openai::OpenAiProvider;
use crate::providers::{ChatMessage, LlmConfig, LlmProvider, MessageRole as ProviderMessageRole};
use crate::unified_tool_executor::UnifiedToolExecutor;
use everruns_agent_loop::traits::{MessageStore, ToolExecutor};

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
            .unwrap_or_else(|| "gpt-5.2".to_string()),
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

/// Execute tool calls using the UnifiedToolExecutor
///
/// This activity executes tool calls using the same ToolExecutor trait
/// that the in-process mode uses, ensuring consistent behavior.
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

    let persist_activity = PersistEventActivity::new(db.clone());
    let tool_executor = UnifiedToolExecutor::with_default_tools();
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

// =============================================================================
// Step-based activities (using step.rs abstractions)
// Each activity is a separate Temporal node for better observability and retry
// =============================================================================

/// Setup step activity - loads agent config and messages
/// This is the first activity in a session workflow
#[allow(dead_code)]
pub async fn setup_step_activity(
    ctx: &ActivityContext,
    db: &Database,
    input: SetupStepInput,
) -> Result<SetupStepOutput> {
    info!(
        session_id = %input.session_id,
        agent_id = %input.agent_id,
        "Setting up agent loop"
    );

    ctx.heartbeat("Loading agent configuration");

    // Load agent
    let agent = db
        .get_agent(input.agent_id)
        .await
        .context("Database error loading agent")?
        .ok_or_else(|| anyhow::anyhow!("Agent not found: {}", input.agent_id))?;

    let agent_config = LoadAgentOutput {
        agent_id: agent.id,
        name: agent.name,
        model_id: agent
            .default_model_id
            .map(|id| id.to_string())
            .unwrap_or_else(|| "gpt-5.2".to_string()),
        system_prompt: Some(agent.system_prompt),
        temperature: None,
        max_tokens: None,
    };

    ctx.heartbeat("Loading messages");

    // Load messages using DbMessageStore
    let message_store = DbMessageStore::new(db.clone());
    let messages = message_store
        .load(input.session_id)
        .await
        .context("Failed to load messages")?;

    info!(
        session_id = %input.session_id,
        message_count = messages.len(),
        "Setup complete"
    );

    // Create setup step record
    let step = LoopStep::setup(input.session_id).complete(StepResult::SetupComplete {
        message_count: messages.len(),
    });

    Ok(SetupStepOutput {
        agent_config,
        messages,
        step,
    })
}

/// Execute LLM step activity - calls LLM with current messages
/// Returns whether there are tool calls to execute
#[allow(dead_code)]
pub async fn execute_llm_step_activity(
    ctx: &ActivityContext,
    db: &Database,
    input: ExecuteLlmStepInput,
) -> Result<ExecuteLlmStepOutput> {
    info!(
        session_id = %input.session_id,
        iteration = input.iteration,
        message_count = input.messages.len(),
        "Executing LLM step"
    );

    ctx.heartbeat("Starting LLM call");

    // Build agent config
    let config = AgentConfig::new(
        input.agent_config.system_prompt.as_deref().unwrap_or(""),
        &input.agent_config.model_id,
    )
    .with_max_iterations(1); // Single iteration for this step

    // Create agent loop with NoOp event emitter (we'll emit events separately for Temporal)
    // and DbMessageStore for message storage, using UnifiedToolExecutor for consistency
    let event_emitter = NoOpEventEmitter;
    let message_store = DbMessageStore::new(db.clone());
    let llm_provider = OpenAiLlmAdapter::new().context("Failed to create LLM adapter")?;
    let tool_executor = UnifiedToolExecutor::with_default_tools();

    let agent_loop = AgentLoop::new(
        config,
        event_emitter,
        message_store,
        llm_provider,
        tool_executor,
    );

    // Create step input
    let step_input = StepInput {
        session_id: input.session_id,
        iteration: input.iteration,
        messages: input.messages,
        pending_tool_calls: Vec::new(),
    };

    ctx.heartbeat("Calling LLM...");

    // Execute the step
    let step_output = agent_loop
        .execute_step(step_input)
        .await
        .context("Failed to execute LLM step")?;

    let has_tool_calls = !step_output.pending_tool_calls.is_empty();
    let pending_tool_calls = step_output.pending_tool_calls.clone();

    info!(
        session_id = %input.session_id,
        iteration = input.iteration,
        has_tool_calls = has_tool_calls,
        tool_count = pending_tool_calls.len(),
        "LLM step complete"
    );

    // Persist step event
    let persist_activity = PersistEventActivity::new(db.clone());
    let step_event = AgUiEvent::step_finished(format!("llm_call_{}", input.iteration));
    persist_activity
        .persist_event(input.session_id, step_event)
        .await?;

    Ok(ExecuteLlmStepOutput {
        step_output,
        has_tool_calls,
        pending_tool_calls,
    })
}

/// Execute a single tool activity - each tool call is a separate Temporal node
/// This provides maximum observability and allows individual tool retries
///
/// Uses the UnifiedToolExecutor to ensure consistent behavior with in-process mode.
#[allow(dead_code)]
pub async fn execute_single_tool_activity(
    ctx: &ActivityContext,
    db: &Database,
    input: ExecuteSingleToolInput,
) -> Result<ExecuteSingleToolOutput> {
    info!(
        session_id = %input.session_id,
        tool_id = %input.tool_call.id,
        tool_name = %input.tool_call.name,
        "Executing single tool via UnifiedToolExecutor"
    );

    ctx.heartbeat(&format!("Executing tool: {}", input.tool_call.name));

    // Persist tool start events
    let persist_activity = PersistEventActivity::new(db.clone());

    let start_event = AgUiEvent::tool_call_start(&input.tool_call.id, &input.tool_call.name);
    persist_activity
        .persist_event(input.session_id, start_event)
        .await?;

    let args_json = serde_json::to_string(&input.tool_call.arguments).unwrap_or_default();
    let args_event = AgUiEvent::tool_call_args(&input.tool_call.id, args_json);
    persist_activity
        .persist_event(input.session_id, args_event)
        .await?;

    let end_event = AgUiEvent::tool_call_end(&input.tool_call.id);
    persist_activity
        .persist_event(input.session_id, end_event)
        .await?;

    // Create the unified tool executor
    let tool_executor = UnifiedToolExecutor::with_default_tools();

    // Parse tool definition if provided
    let tool_def: ToolDefinition = if let Some(ref json) = input.tool_definition_json {
        serde_json::from_str(json).context("Failed to parse tool definition")?
    } else {
        // Default to builtin - UnifiedToolExecutor will look it up in registry
        ToolDefinition::Builtin(everruns_contracts::tools::BuiltinTool {
            name: input.tool_call.name.clone(),
            description: "Tool execution".to_string(),
            kind: everruns_contracts::tools::BuiltinToolKind::HttpGet,
            policy: everruns_contracts::tools::ToolPolicy::Auto,
            parameters: serde_json::json!({}),
        })
    };

    // Execute the tool using the ToolExecutor trait
    let result = tool_executor
        .execute(&input.tool_call, &tool_def)
        .await
        .map_err(|e| anyhow::anyhow!("Tool execution failed: {}", e))?;

    // Persist tool result event
    let result_message_id = Uuid::now_v7().to_string();
    let result_event = AgUiEvent::tool_call_result(
        &result_message_id,
        &input.tool_call.id,
        result.result.clone().unwrap_or_default(),
    );
    persist_activity
        .persist_event(input.session_id, result_event)
        .await?;

    info!(
        session_id = %input.session_id,
        tool_id = %input.tool_call.id,
        success = result.error.is_none(),
        "Tool execution complete"
    );

    // Create tool execution step record
    let step =
        LoopStep::tool_execution(input.session_id, 0).complete(StepResult::ToolExecutionComplete {
            results: vec![everruns_contracts::tools::ToolResult {
                tool_call_id: result.tool_call_id.clone(),
                result: result.result.clone(),
                error: result.error.clone(),
            }],
        });

    Ok(ExecuteSingleToolOutput { result, step })
}

/// Finalize step activity - saves final message and updates session status
#[allow(dead_code)]
pub async fn finalize_step_activity(
    ctx: &ActivityContext,
    db: &Database,
    input: FinalizeStepInput,
) -> Result<FinalizeStepOutput> {
    info!(
        session_id = %input.session_id,
        iterations = input.total_iterations,
        "Finalizing session"
    );

    ctx.heartbeat("Saving final message");

    // Save final assistant message if present
    if let Some(ref response) = input.final_response {
        let create_msg = everruns_storage::models::CreateMessage {
            session_id: input.session_id,
            role: "assistant".to_string(),
            content: serde_json::json!({ "text": response }),
            tool_call_id: None,
        };

        db.create_message(create_msg)
            .await
            .context("Database error saving final message")?;
    }

    ctx.heartbeat("Updating session status");

    // Update session status back to pending (ready for more messages)
    let update = UpdateSession {
        status: Some("pending".to_string()),
        ..Default::default()
    };

    db.update_session(input.session_id, update)
        .await
        .context("Database error updating session status")?;

    // Persist session finished event
    let persist_activity = PersistEventActivity::new(db.clone());
    let finished_event = AgUiEvent::session_finished(input.session_id.to_string());
    persist_activity
        .persist_event(input.session_id, finished_event)
        .await?;

    info!(
        session_id = %input.session_id,
        "Session finalized"
    );

    // Create finalize step record
    let step = LoopStep::finalize(input.session_id, input.total_iterations).complete(
        StepResult::FinalizeComplete {
            final_response: input.final_response,
        },
    );

    Ok(FinalizeStepOutput {
        status: "pending".to_string(),
        step,
    })
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
