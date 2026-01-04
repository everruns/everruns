//! ReasonAtom - Atom for LLM reasoning (model call)
//!
//! This atom handles:
//! 1. Emitting reason.started event
//! 2. Context preparation (loading message history, adding system message)
//! 3. Fixing invalid context (e.g., missing tool_results for dangling tool calls)
//! 4. LLM call with streaming support
//! 5. Storing the assistant response
//! 6. Emitting reason.completed event
//! 7. Returning the result with tool calls (if any)
//!
//! NOTES from Python spec:
//! - Context preparation includes loading message history, adding system message, editing context if needed
//! - Before LLM call, invalid context (e.g. missing tool_results) should be fixed
//! - LLM call should emit start/end events
//! - Failure of the LLM call should be "normal" result, should user message that LLM call failed
//! - Reason should be cancellable, cancellation should stop LLM call and exit with message

use async_trait::async_trait;
use futures::StreamExt;
use serde::{Deserialize, Serialize};
use std::time::Instant;
use uuid::Uuid;

use super::{Atom, AtomContext};
use crate::capabilities::CapabilityRegistry;
use crate::error::{AgentLoopError, Result};
use crate::events::{
    EventContext, EventRequest, LlmGenerationData, ReasonCompletedData, ReasonStartedData,
};
use crate::llm_driver_registry::{
    DriverRegistry, LlmCallConfigBuilder, LlmMessage, LlmMessageContent, LlmMessageRole,
    LlmStreamEvent, ProviderConfig, ProviderType,
};
use crate::message::{Message, MessageRole};
use crate::runtime_agent::RuntimeAgentBuilder;
use crate::tool_types::{ToolCall, ToolDefinition};
use crate::traits::{
    AgentStore, EventEmitter, LlmProviderStore, MessageStore, ModelWithProvider, SessionStore,
};

// ============================================================================
// Helper Functions
// ============================================================================

/// Patch dangling tool calls by adding synthetic "cancelled" results.
///
/// This ensures every tool call has a corresponding tool result,
/// preventing LLM API errors (e.g., OpenAI requires every tool_call to have a result).
fn patch_dangling_tool_calls(messages: &[Message]) -> Vec<Message> {
    let mut result = Vec::new();

    for (i, msg) in messages.iter().enumerate() {
        result.push(msg.clone());

        // After an assistant message with tool calls, add cancelled results for any missing ones
        if msg.role == MessageRole::Assistant && msg.has_tool_calls() {
            for tc in msg.tool_calls() {
                // Look for a matching tool result in ALL subsequent messages
                let has_result = messages[(i + 1)..]
                    .iter()
                    .any(|m| m.role == MessageRole::ToolResult && m.tool_call_id() == Some(&tc.id));

                if !has_result {
                    result.push(Message::tool_result(
                        &tc.id,
                        None,
                        Some(
                            "cancelled - another message came in before it could be completed"
                                .to_string(),
                        ),
                    ));
                }
            }
        }
    }

    result
}

// ============================================================================
// Input and Output Types
// ============================================================================

/// Input for ReasonAtom
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ReasonInput {
    /// Atom execution context
    pub context: AtomContext,
    /// Agent ID for loading configuration
    pub agent_id: Uuid,
}

/// Result of the ReasonAtom
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct ReasonResult {
    /// Whether the LLM call succeeded
    pub success: bool,
    /// Text response from the model
    pub text: String,
    /// Tool calls requested by the model
    #[serde(default)]
    pub tool_calls: Vec<ToolCall>,
    /// Whether tool execution is needed
    pub has_tool_calls: bool,
    /// Tool definitions from applied capabilities (for tool execution)
    #[serde(default)]
    pub tool_definitions: Vec<ToolDefinition>,
    /// Maximum iterations configured for the agent
    #[serde(default = "default_max_iterations")]
    pub max_iterations: usize,
    /// Error message if the call failed
    #[serde(skip_serializing_if = "Option::is_none")]
    pub error: Option<String>,
}

fn default_max_iterations() -> usize {
    100
}

// ============================================================================
// ReasonAtom
// ============================================================================

/// Atom that calls the LLM model for reasoning
///
/// This atom:
/// 1. Emits reason.started event
/// 2. Retrieves agent and session configuration from stores
/// 3. Resolves model using priority: controls.model_id > session.model_id > agent.default_model_id
/// 4. Builds configuration with capabilities applied
/// 5. Loads messages from the store
/// 6. Patches dangling tool calls
/// 7. Calls the LLM with the messages
/// 8. Stores the assistant response
/// 9. Emits reason.completed event
/// 10. Returns the result with tool calls (if any)
pub struct ReasonAtom<A, S, M, P, E>
where
    A: AgentStore,
    S: SessionStore,
    M: MessageStore,
    P: LlmProviderStore,
    E: EventEmitter,
{
    agent_store: A,
    session_store: S,
    message_store: M,
    provider_store: P,
    capability_registry: CapabilityRegistry,
    driver_registry: DriverRegistry,
    event_emitter: E,
}

impl<A, S, M, P, E> ReasonAtom<A, S, M, P, E>
where
    A: AgentStore,
    S: SessionStore,
    M: MessageStore,
    P: LlmProviderStore,
    E: EventEmitter,
{
    /// Create a new ReasonAtom
    pub fn new(
        agent_store: A,
        session_store: S,
        message_store: M,
        provider_store: P,
        capability_registry: CapabilityRegistry,
        driver_registry: DriverRegistry,
        event_emitter: E,
    ) -> Self {
        Self {
            agent_store,
            session_store,
            message_store,
            provider_store,
            capability_registry,
            driver_registry,
            event_emitter,
        }
    }
}

#[async_trait]
impl<A, S, M, P, E> Atom for ReasonAtom<A, S, M, P, E>
where
    A: AgentStore + Send + Sync,
    S: SessionStore + Send + Sync,
    M: MessageStore + Send + Sync,
    P: LlmProviderStore + Send + Sync,
    E: EventEmitter + Send + Sync,
{
    type Input = ReasonInput;
    type Output = ReasonResult;

    fn name(&self) -> &'static str {
        "reason"
    }

    async fn execute(&self, input: Self::Input) -> Result<Self::Output> {
        let ReasonInput { context, agent_id } = input;

        tracing::info!(
            session_id = %context.session_id,
            turn_id = %context.turn_id,
            exec_id = %context.exec_id,
            agent_id = %agent_id,
            "ReasonAtom: starting LLM call"
        );

        // Create event context from atom context
        let event_context = EventContext::from_atom_context(&context);

        // Emit reason.started event (note: we'll emit a more detailed event after model resolution)
        if let Err(e) = self
            .event_emitter
            .emit(EventRequest::new(
                context.session_id,
                event_context.clone(),
                ReasonStartedData {
                    agent_id,
                    metadata: None, // Will be populated after model resolution
                },
            ))
            .await
        {
            tracing::warn!(
                session_id = %context.session_id,
                error = %e,
                "ReasonAtom: failed to emit reason.started event"
            );
        }

        // Execute the LLM call and handle errors gracefully
        let result = match self
            .execute_llm_call(context.session_id, agent_id, &context)
            .await
        {
            Ok(result) => {
                // Emit reason.completed event for success
                if let Err(e) = self
                    .event_emitter
                    .emit(EventRequest::new(
                        context.session_id,
                        event_context.clone(),
                        ReasonCompletedData::success(
                            &result.text,
                            result.has_tool_calls,
                            result.tool_calls.len() as u32,
                        ),
                    ))
                    .await
                {
                    tracing::warn!(
                        session_id = %context.session_id,
                        error = %e,
                        "ReasonAtom: failed to emit reason.completed event"
                    );
                }
                result
            }
            Err(e) => {
                // LLM call failure is a "normal" result per the spec
                // Return a result indicating failure with the error message
                tracing::warn!(
                    session_id = %context.session_id,
                    turn_id = %context.turn_id,
                    error = %e,
                    "ReasonAtom: LLM call failed"
                );

                let error_msg = e.to_string();

                // Emit reason.completed event for failure
                if let Err(emit_err) = self
                    .event_emitter
                    .emit(EventRequest::new(
                        context.session_id,
                        event_context,
                        ReasonCompletedData::failure(error_msg.clone()),
                    ))
                    .await
                {
                    tracing::warn!(
                        session_id = %context.session_id,
                        error = %emit_err,
                        "ReasonAtom: failed to emit reason.completed event"
                    );
                }

                ReasonResult {
                    success: false,
                    text: format!(
                        "I encountered an error while processing your request: {}",
                        e
                    ),
                    tool_calls: vec![],
                    has_tool_calls: false,
                    tool_definitions: vec![],
                    max_iterations: default_max_iterations(),
                    error: Some(error_msg),
                }
            }
        };

        Ok(result)
    }
}

impl<A, S, M, P, E> ReasonAtom<A, S, M, P, E>
where
    A: AgentStore + Send + Sync,
    S: SessionStore + Send + Sync,
    M: MessageStore + Send + Sync,
    P: LlmProviderStore + Send + Sync,
    E: EventEmitter + Send + Sync,
{
    /// Execute the actual LLM call
    async fn execute_llm_call(
        &self,
        session_id: Uuid,
        agent_id: Uuid,
        context: &AtomContext,
    ) -> Result<ReasonResult> {
        // 1. Retrieve agent
        let agent = self
            .agent_store
            .get_agent(agent_id)
            .await?
            .ok_or_else(|| AgentLoopError::agent_not_found(agent_id))?;

        // 2. Retrieve session
        let session = self
            .session_store
            .get_session(session_id)
            .await?
            .ok_or_else(|| AgentLoopError::session_not_found(session_id))?;

        // 3. Load messages (needed for controls.model_id extraction)
        let messages = self.message_store.load(session_id).await?;

        if messages.is_empty() {
            return Err(AgentLoopError::NoMessages);
        }

        // 4. Extract model_id from the last user message's controls (highest priority)
        let controls_model_id = messages
            .iter()
            .rev()
            .find(|m| m.role == MessageRole::User)
            .and_then(|m| m.controls.as_ref())
            .and_then(|c| c.model_id);

        // 5. Resolve model using chain: controls.model_id > session.model_id > agent.default_model_id
        let model_with_provider = self
            .resolve_model(controls_model_id, session.model_id, agent.default_model_id)
            .await?;

        // 6. Build runtime agent from agent with capabilities applied
        let runtime_agent = RuntimeAgentBuilder::new()
            .with_agent(&agent, &self.capability_registry)
            .model(&model_with_provider.model)
            .build();

        // 7. Create LLM driver using factory
        let llm_driver = self.create_llm_driver(&model_with_provider)?;

        // 8. Extract reasoning effort from the last user message's controls
        let reasoning_effort = messages
            .iter()
            .rev()
            .find(|m| m.role == MessageRole::User)
            .and_then(|m| m.controls.as_ref())
            .and_then(|c| c.reasoning.as_ref())
            .and_then(|r| r.effort.clone());

        // 9. Patch dangling tool calls (add cancelled results for tool calls without responses)
        let patched_messages = patch_dangling_tool_calls(&messages);

        // 10. Build LLM messages
        let mut llm_messages = Vec::new();

        // Add system prompt
        if !runtime_agent.system_prompt.is_empty() {
            llm_messages.push(LlmMessage {
                role: LlmMessageRole::System,
                content: LlmMessageContent::Text(runtime_agent.system_prompt.clone()),
                tool_calls: None,
                tool_call_id: None,
            });
        }

        // Add conversation messages
        for msg in &patched_messages {
            llm_messages.push(msg.into());
        }

        // 11. Build LLM call config with reasoning effort
        let mut llm_config_builder = LlmCallConfigBuilder::from(&runtime_agent);
        if let Some(effort) = reasoning_effort.clone() {
            llm_config_builder = llm_config_builder.reasoning_effort(effort);
        }
        let llm_config = llm_config_builder.build();

        tracing::debug!(
            session_id = %session_id,
            turn_id = %context.turn_id,
            model = %runtime_agent.model,
            message_count = %llm_messages.len(),
            "ReasonAtom: calling LLM"
        );

        // Track LLM call timing
        let llm_start = Instant::now();

        let mut stream = llm_driver
            .chat_completion_stream(llm_messages, &llm_config)
            .await?;

        // 12. Process stream
        let mut text = String::new();
        let mut tool_calls = Vec::new();

        while let Some(event) = stream.next().await {
            match event? {
                LlmStreamEvent::TextDelta(delta) => {
                    text.push_str(&delta);
                }
                LlmStreamEvent::ToolCalls(calls) => {
                    tool_calls = calls;
                }
                LlmStreamEvent::Done(_) => {
                    break;
                }
                LlmStreamEvent::Error(err) => {
                    // Emit llm.generation failure event
                    let llm_duration_ms = llm_start.elapsed().as_millis() as u64;
                    let event_context = EventContext::from_atom_context(context);
                    let _ = self
                        .event_emitter
                        .emit(EventRequest::new(
                            session_id,
                            event_context,
                            LlmGenerationData::failure(
                                patched_messages.clone(),
                                runtime_agent.model.clone(),
                                Some(model_with_provider.provider_type.to_string()),
                                err.clone(),
                                Some(llm_duration_ms),
                            ),
                        ))
                        .await;
                    return Err(AgentLoopError::llm(err));
                }
            }
        }

        let llm_duration_ms = llm_start.elapsed().as_millis() as u64;

        // 13. Emit llm.generation event
        let event_context = EventContext::from_atom_context(context);
        if let Err(e) = self
            .event_emitter
            .emit(EventRequest::new(
                session_id,
                event_context,
                LlmGenerationData::success(
                    patched_messages.clone(),
                    Some(text.clone()).filter(|s| !s.is_empty()),
                    tool_calls.clone(),
                    runtime_agent.model.clone(),
                    Some(model_with_provider.provider_type.to_string()),
                    None, // usage - not available from stream yet
                    Some(llm_duration_ms),
                ),
            ))
            .await
        {
            tracing::warn!(
                session_id = %session_id,
                error = %e,
                "ReasonAtom: failed to emit llm.generation event"
            );
        }

        // 14. Build metadata with model and reasoning effort info
        let mut metadata = std::collections::HashMap::new();
        metadata.insert(
            "model".to_string(),
            serde_json::Value::String(runtime_agent.model.clone()),
        );
        if let Some(ref effort) = reasoning_effort {
            metadata.insert(
                "reasoning_effort".to_string(),
                serde_json::Value::String(effort.clone()),
            );
        }

        // 14. Store assistant message with metadata
        let has_tool_calls = !tool_calls.is_empty();
        let mut assistant_message = if has_tool_calls {
            Message::assistant_with_tools(&text, tool_calls.clone())
        } else {
            Message::assistant(&text)
        };
        assistant_message.metadata = Some(metadata);

        self.message_store
            .store(session_id, assistant_message.clone())
            .await?;

        tracing::info!(
            session_id = %session_id,
            turn_id = %context.turn_id,
            has_tool_calls = %has_tool_calls,
            tool_count = %tool_calls.len(),
            "ReasonAtom: LLM call completed"
        );

        Ok(ReasonResult {
            success: true,
            text,
            tool_calls,
            has_tool_calls,
            tool_definitions: runtime_agent.tools.clone(),
            max_iterations: runtime_agent.max_iterations,
            error: None,
        })
    }

    /// Resolve model using priority chain
    async fn resolve_model(
        &self,
        controls_model_id: Option<Uuid>,
        session_model_id: Option<Uuid>,
        agent_model_id: Option<Uuid>,
    ) -> Result<ModelWithProvider> {
        // Try controls.model_id first (highest priority)
        if let Some(model_id) = controls_model_id {
            if let Some(model_with_provider) = self
                .provider_store
                .get_model_with_provider(model_id)
                .await?
            {
                return Ok(model_with_provider);
            }
        }

        // Try session.model_id second
        if let Some(model_id) = session_model_id {
            if let Some(model_with_provider) = self
                .provider_store
                .get_model_with_provider(model_id)
                .await?
            {
                return Ok(model_with_provider);
            }
        }

        // Try agent.default_model_id third
        if let Some(model_id) = agent_model_id {
            if let Some(model_with_provider) = self
                .provider_store
                .get_model_with_provider(model_id)
                .await?
            {
                return Ok(model_with_provider);
            }
        }

        // Fall back to system default model
        self.provider_store
            .get_default_model()
            .await?
            .ok_or_else(|| {
                AgentLoopError::llm(
                    "No model configured: no model_id in controls, session, or agent, and no system default model is set"
                )
            })
    }

    /// Create LLM driver using the driver registry
    fn create_llm_driver(
        &self,
        model: &ModelWithProvider,
    ) -> Result<crate::llm_driver_registry::BoxedLlmDriver> {
        let provider_type = match model.provider_type {
            crate::llm_models::LlmProviderType::Openai => ProviderType::OpenAI,
            crate::llm_models::LlmProviderType::Anthropic => ProviderType::Anthropic,
            crate::llm_models::LlmProviderType::AzureOpenAI => ProviderType::AzureOpenAI,
            crate::llm_models::LlmProviderType::LlmSim => ProviderType::LlmSim,
        };

        let mut config = ProviderConfig::new(provider_type);
        if let Some(ref api_key) = model.api_key {
            config = config.with_api_key(api_key);
        }
        if let Some(ref base_url) = model.base_url {
            config = config.with_base_url(base_url);
        }

        self.driver_registry.create_driver(&config)
    }
}

// ============================================================================
// Tests
// ============================================================================

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_reason_result_default() {
        let result = ReasonResult::default();
        assert!(!result.success);
        assert!(result.text.is_empty());
        assert!(result.tool_calls.is_empty());
        assert!(!result.has_tool_calls);
        // Default derive gives 0, but serde deserialization gives 100 via default_max_iterations()
        assert_eq!(result.max_iterations, 0);
    }

    #[test]
    fn test_reason_result_serde_default() {
        // Test that serde uses the default_max_iterations function
        let json = r#"{"success":true,"text":"","has_tool_calls":false}"#;
        let result: ReasonResult = serde_json::from_str(json).unwrap();
        assert_eq!(result.max_iterations, 100);
    }

    #[test]
    fn test_patch_dangling_tool_calls_no_tool_calls() {
        let messages = vec![Message::user("Hello"), Message::assistant("Hi there!")];
        let patched = patch_dangling_tool_calls(&messages);
        assert_eq!(patched.len(), 2);
    }

    #[test]
    fn test_patch_dangling_tool_calls_with_result() {
        let tool_call = ToolCall {
            id: "call_123".to_string(),
            name: "get_weather".to_string(),
            arguments: serde_json::json!({"city": "NYC"}),
        };

        let messages = vec![
            Message::user("What's the weather?"),
            Message::assistant_with_tools("Let me check", vec![tool_call]),
            Message::tool_result("call_123", Some(serde_json::json!({"temp": 72})), None),
        ];

        let patched = patch_dangling_tool_calls(&messages);
        assert_eq!(patched.len(), 3);
    }

    #[test]
    fn test_patch_dangling_tool_calls_missing_result() {
        let tool_call = ToolCall {
            id: "call_456".to_string(),
            name: "search_web".to_string(),
            arguments: serde_json::json!({"query": "rust"}),
        };

        let messages = vec![
            Message::user("Search for rust"),
            Message::assistant_with_tools("Searching...", vec![tool_call]),
            Message::user("Actually, never mind"),
        ];

        let patched = patch_dangling_tool_calls(&messages);
        // Should have added a cancelled result
        assert_eq!(patched.len(), 4);
        assert_eq!(patched[2].role, MessageRole::ToolResult);
        assert_eq!(patched[2].tool_call_id(), Some("call_456"));
    }
}
