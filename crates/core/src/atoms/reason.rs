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
use std::collections::HashMap;
use std::sync::Arc;
use std::time::Instant;
use uuid::Uuid;

use super::{Atom, AtomContext};
use crate::capabilities::CapabilityRegistry;
use crate::error::{AgentLoopError, Result};
use crate::events::{
    EventContext, EventRequest, LlmGenerationData, MessageAgentData, ReasonCompletedData,
    ReasonStartedData, ReasonThinkingCompletedData, ReasonThinkingDeltaData,
    ReasonThinkingStartedData, TextDeltaData, TokenUsage, ToolDefinitionSummary,
};
use crate::llm_driver_registry::{
    DriverRegistry, LlmCallConfigBuilder, LlmCompletionMetadata, LlmMessage, LlmMessageContent,
    LlmMessageRole, LlmStreamEvent, ProviderConfig, ProviderType,
};
use crate::message::{Message, MessageRole};
use crate::message_retriever::MessageRetriever;
use crate::runtime_agent::RuntimeAgentBuilder;
use crate::tool_types::{ToolCall, ToolDefinition};
use crate::traits::{
    AgentStore, EventEmitter, ImageResolver, LlmProviderStore, ModelWithProvider, ResolvedImage,
    SessionStore,
};
use crate::typed_id::{AgentId, SessionId};

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
    pub agent_id: AgentId,
    /// Organization ID for multi-tenancy tracking
    #[serde(default)]
    pub org_id: i64,
    /// MCP tool definitions from agent's MCP capabilities (pre-resolved)
    /// These are passed from the control-plane since MCP capabilities
    /// are not in the CapabilityRegistry.
    #[serde(default)]
    pub mcp_tool_definitions: Vec<ToolDefinition>,
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
    /// Token usage from the LLM call
    #[serde(skip_serializing_if = "Option::is_none")]
    pub usage: Option<TokenUsage>,
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
/// 7. Resolves image_file content parts to actual image data (if ImageResolver provided)
/// 8. Calls the LLM with the messages
/// 9. Stores the assistant response
/// 10. Emits reason.completed event
/// 11. Returns the result with tool calls (if any)
pub struct ReasonAtom<A, S, M, P, E>
where
    A: AgentStore,
    S: SessionStore,
    M: MessageRetriever,
    P: LlmProviderStore,
    E: EventEmitter,
{
    agent_store: A,
    session_store: S,
    message_retriever: M,
    provider_store: P,
    capability_registry: CapabilityRegistry,
    driver_registry: DriverRegistry,
    event_emitter: E,
    /// Optional image resolver for resolving image_file content parts
    image_resolver: Option<Arc<dyn ImageResolver>>,
}

impl<A, S, M, P, E> ReasonAtom<A, S, M, P, E>
where
    A: AgentStore,
    S: SessionStore,
    M: MessageRetriever,
    P: LlmProviderStore,
    E: EventEmitter,
{
    /// Create a new ReasonAtom
    pub fn new(
        agent_store: A,
        session_store: S,
        message_retriever: M,
        provider_store: P,
        capability_registry: CapabilityRegistry,
        driver_registry: DriverRegistry,
        event_emitter: E,
    ) -> Self {
        Self {
            agent_store,
            session_store,
            message_retriever,
            provider_store,
            capability_registry,
            driver_registry,
            event_emitter,
            image_resolver: None,
        }
    }

    /// Set the image resolver for resolving image_file content parts
    ///
    /// When set, image_file references in messages will be resolved to actual
    /// image data before being sent to the LLM. This is required for multimodal
    /// conversations that include image attachments.
    ///
    /// # Example
    ///
    /// ```ignore
    /// let resolver = Arc::new(GrpcImageResolver::new(client));
    /// let atom = ReasonAtom::new(/* ... */).with_image_resolver(resolver);
    /// ```
    pub fn with_image_resolver(mut self, resolver: Arc<dyn ImageResolver>) -> Self {
        self.image_resolver = Some(resolver);
        self
    }
}

#[async_trait]
impl<A, S, M, P, E> Atom for ReasonAtom<A, S, M, P, E>
where
    A: AgentStore + Send + Sync,
    S: SessionStore + Send + Sync,
    M: MessageRetriever + Send + Sync,
    P: LlmProviderStore + Send + Sync,
    E: EventEmitter + Send + Sync,
{
    type Input = ReasonInput;
    type Output = ReasonResult;

    fn name(&self) -> &'static str {
        "reason"
    }

    async fn execute(&self, input: Self::Input) -> Result<Self::Output> {
        let ReasonInput {
            context,
            agent_id,
            org_id,
            mcp_tool_definitions,
        } = input;

        tracing::info!(
            session_id = %context.session_id,
            turn_id = %context.turn_id,
            exec_id = %context.exec_id,
            agent_id = %agent_id,
            mcp_tools_count = %mcp_tool_definitions.len(),
            "ReasonAtom: starting LLM call"
        );

        // Generate OTel-style span IDs for hierarchical tracing
        // trace_id: groups all events in this turn
        // span_id: unique identifier for this reason span (shared by started/completed)
        // parent_span_id: links to turn as parent
        //
        // NOTE: TurnId::to_string() returns prefixed format (e.g., "turn_abc123")
        // matching the format used by turn.started/completed events in Braintrust.
        let trace_id = context.turn_id.to_string();
        let reason_span_id = Uuid::now_v7().to_string();
        let parent_span_id = trace_id.clone(); // Parent is the turn

        // Create event context from atom context with span info
        let event_context = EventContext::from_atom_context(&context).with_span(
            trace_id.clone(),
            reason_span_id.clone(),
            Some(parent_span_id.clone()),
        );

        // Track reason phase timing for Braintrust observability
        let reason_start = Instant::now();

        // Emit reason.started event
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
            .execute_llm_call(
                context.session_id,
                agent_id,
                org_id,
                &context,
                &mcp_tool_definitions,
                &trace_id,
                &reason_span_id,
            )
            .await
        {
            Ok(result) => {
                // Calculate reason phase duration
                let reason_duration_ms = reason_start.elapsed().as_millis() as u64;

                // Emit reason.completed event (same span as reason.started, parent is turn)
                let completed_context = EventContext::from_atom_context(&context).with_span(
                    trace_id.clone(),
                    reason_span_id.clone(), // Same span_id as started
                    Some(parent_span_id.clone()),
                );
                if let Err(e) = self
                    .event_emitter
                    .emit(EventRequest::new(
                        context.session_id,
                        completed_context,
                        ReasonCompletedData::success(
                            &result.text,
                            result.has_tool_calls,
                            result.tool_calls.len() as u32,
                            Some(reason_duration_ms),
                            result.usage.clone(),
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
                // Calculate reason phase duration even for failures
                let reason_duration_ms = reason_start.elapsed().as_millis() as u64;

                // LLM call failure is a "normal" result per the spec
                // Return a result indicating failure with the error message
                tracing::warn!(
                    session_id = %context.session_id,
                    turn_id = %context.turn_id,
                    error = %e,
                    "ReasonAtom: LLM call failed"
                );

                let error_msg = e.to_string();

                // User-facing error message
                // For request-too-large errors, provide specific guidance
                let user_error_text = if e.is_request_too_large() {
                    "The conversation has become too long for the model to process. \
                     Please start a new session or reduce the context size."
                        .to_string()
                } else {
                    "I encountered an error while processing your request. Please try again later."
                        .to_string()
                };

                // Create error message for the user to see
                let error_message = Message::assistant(&user_error_text);

                // Emit message.agent event (stores message as event with proper turn context)
                // message.agent is child of reason span
                let error_msg_context = EventContext::from_atom_context(&context).with_span(
                    trace_id.clone(),
                    Uuid::now_v7().to_string(),   // Own span_id
                    Some(reason_span_id.clone()), // Parent is reason span
                );
                if let Err(emit_err) = self
                    .event_emitter
                    .emit(EventRequest::new(
                        context.session_id,
                        error_msg_context,
                        MessageAgentData::new(error_message),
                    ))
                    .await
                {
                    tracing::warn!(
                        session_id = %context.session_id,
                        error = %emit_err,
                        "ReasonAtom: failed to emit message.agent event for error"
                    );
                }

                // Emit reason.completed event for failure (same span as started, parent is turn)
                let completed_context = EventContext::from_atom_context(&context).with_span(
                    trace_id.clone(),
                    reason_span_id.clone(), // Same span_id as started
                    Some(parent_span_id.clone()),
                );
                if let Err(emit_err) = self
                    .event_emitter
                    .emit(EventRequest::new(
                        context.session_id,
                        completed_context,
                        ReasonCompletedData::failure(error_msg.clone(), Some(reason_duration_ms)),
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
                    text: user_error_text,
                    tool_calls: vec![],
                    has_tool_calls: false,
                    tool_definitions: vec![],
                    max_iterations: default_max_iterations(),
                    error: Some(error_msg),
                    usage: None,
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
    M: MessageRetriever + Send + Sync,
    P: LlmProviderStore + Send + Sync,
    E: EventEmitter + Send + Sync,
{
    /// Execute the actual LLM call
    #[allow(clippy::too_many_arguments)]
    async fn execute_llm_call(
        &self,
        session_id: SessionId,
        agent_id: AgentId,
        org_id: i64,
        context: &AtomContext,
        mcp_tool_definitions: &[ToolDefinition],
        trace_id: &str,
        reason_span_id: &str,
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
        let messages = self.message_retriever.load(session_id).await?;

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
        let (model_with_provider, resolved_model_id) = self
            .resolve_model(controls_model_id, session.model_id, agent.default_model_id)
            .await?;

        // 6. Build runtime agent from agent with capabilities applied
        // Also include MCP tool definitions that were pre-resolved by control-plane
        let runtime_agent = RuntimeAgentBuilder::new()
            .with_agent(&agent, &self.capability_registry)
            .tools(mcp_tool_definitions.iter().cloned())
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

        // 10. Resolve images from image_file references (if any)
        //
        // Image resolution converts image_file content parts (which only contain UUIDs)
        // into actual base64-encoded image data that can be sent to LLMs.
        let resolved_images = self.resolve_images(&patched_messages).await;

        // 11. Build LLM messages
        let mut llm_messages = Vec::new();

        // Add system prompt
        let has_system_prompt = !runtime_agent.system_prompt.is_empty();
        if has_system_prompt {
            llm_messages.push(LlmMessage {
                role: LlmMessageRole::System,
                content: LlmMessageContent::Text(runtime_agent.system_prompt.clone()),
                tool_calls: None,
                tool_call_id: None,
                thinking: None,
            });
        }

        // Build messages for llm.generation event (includes system message)
        let messages_for_event: Vec<Message> = if has_system_prompt {
            std::iter::once(Message::system(&runtime_agent.system_prompt))
                .chain(patched_messages.iter().cloned())
                .collect()
        } else {
            patched_messages.clone()
        };

        // Add conversation messages with resolved images
        for msg in &patched_messages {
            llm_messages.push(LlmMessage::from_message_with_images(msg, &resolved_images));
        }

        // 12. Build LLM call config with reasoning effort and metadata
        let mut llm_config_builder = LlmCallConfigBuilder::from(&runtime_agent);
        if let Some(effort) = reasoning_effort.clone() {
            llm_config_builder = llm_config_builder.reasoning_effort(effort);
        }

        // Add metadata for API tracking and debugging
        // These IDs help correlate API requests with Everruns entities
        // TypedId::to_string() produces prefixed format (e.g., "session_abc123")
        llm_config_builder = llm_config_builder
            .with_metadata("session_id", session_id.to_string())
            .with_metadata("agent_id", agent_id.to_string())
            .with_metadata("turn_id", context.turn_id.to_string())
            .with_metadata("exec_id", context.exec_id.to_string())
            .with_metadata("org_id", format!("org_{:032x}", org_id));

        // Add model_id if we have one (not available for system default model)
        if let Some(model_id) = &resolved_model_id {
            llm_config_builder = llm_config_builder.with_metadata("model_id", model_id.to_string());
        }

        let llm_config = llm_config_builder.build();

        tracing::debug!(
            session_id = %session_id,
            turn_id = %context.turn_id,
            model = %runtime_agent.model,
            message_count = %llm_messages.len(),
            "ReasonAtom: calling LLM"
        );

        // 13. Emit reason.thinking.started event BEFORE starting LLM call
        // This allows UI to show a thinking indicator immediately
        let streaming_event_context = EventContext::from_atom_context(context);
        tracing::info!(
            session_id = %session_id,
            turn_id = %context.turn_id,
            "ReasonAtom: emitting reason.thinking.started event"
        );
        if let Err(e) = self
            .event_emitter
            .emit(EventRequest::new(
                session_id,
                streaming_event_context.clone(),
                ReasonThinkingStartedData {
                    turn_id: context.turn_id,
                    model: Some(runtime_agent.model.clone()),
                },
            ))
            .await
        {
            tracing::warn!(
                session_id = %session_id,
                error = %e,
                "ReasonAtom: failed to emit reason.thinking.started event"
            );
        } else {
            tracing::info!(
                session_id = %session_id,
                "ReasonAtom: reason.thinking.started event emitted successfully"
            );
        }

        // Track LLM call timing
        let llm_start = Instant::now();

        let mut stream = llm_driver
            .chat_completion_stream(llm_messages, &llm_config)
            .await?;

        // 14. Process stream with batched text.delta emissions
        // Batch deltas every 100ms to reduce event volume while providing real-time feedback
        const DELTA_BATCH_INTERVAL_MS: u64 = 100;
        let mut text = String::new();
        let mut thinking = String::new();
        let mut tool_calls = Vec::new();
        let mut completion_metadata: Option<LlmCompletionMetadata> = None;
        let mut pending_delta = String::new();
        let mut pending_thinking_delta = String::new();
        let mut last_delta_emit = Instant::now();
        let mut last_thinking_delta_emit = Instant::now();
        let mut time_to_first_token_ms: Option<u64> = None;

        while let Some(event) = stream.next().await {
            match event? {
                LlmStreamEvent::TextDelta(delta) => {
                    // Track time-to-first-token on first non-empty delta
                    if time_to_first_token_ms.is_none() && !delta.is_empty() {
                        let ttft = llm_start.elapsed().as_millis() as u64;
                        time_to_first_token_ms = Some(ttft);
                        tracing::info!(
                            session_id = %session_id,
                            time_to_first_token_ms = ttft,
                            "ReasonAtom: received first token from LLM"
                        );
                    }
                    text.push_str(&delta);
                    pending_delta.push_str(&delta);

                    // Emit batched delta if interval elapsed
                    if last_delta_emit.elapsed().as_millis() as u64 >= DELTA_BATCH_INTERVAL_MS
                        && !pending_delta.is_empty()
                    {
                        if let Err(e) = self
                            .event_emitter
                            .emit(EventRequest::new(
                                session_id,
                                streaming_event_context.clone(),
                                TextDeltaData {
                                    turn_id: context.turn_id,
                                    delta: pending_delta.clone(),
                                    accumulated: text.clone(),
                                },
                            ))
                            .await
                        {
                            tracing::warn!(
                                session_id = %session_id,
                                error = %e,
                                "ReasonAtom: failed to emit text.delta event"
                            );
                        }
                        pending_delta.clear();
                        last_delta_emit = Instant::now();
                    }
                }
                LlmStreamEvent::ThinkingDelta(delta) => {
                    // Accumulate thinking content from extended thinking models
                    thinking.push_str(&delta);
                    pending_thinking_delta.push_str(&delta);
                    tracing::debug!(
                        session_id = %session_id,
                        delta_len = delta.len(),
                        total_thinking_len = thinking.len(),
                        "ReasonAtom: received ThinkingDelta from LLM"
                    );

                    // Emit batched thinking delta if interval elapsed
                    if last_thinking_delta_emit.elapsed().as_millis() as u64
                        >= DELTA_BATCH_INTERVAL_MS
                        && !pending_thinking_delta.is_empty()
                    {
                        if let Err(e) = self
                            .event_emitter
                            .emit(EventRequest::new(
                                session_id,
                                streaming_event_context.clone(),
                                ReasonThinkingDeltaData {
                                    turn_id: context.turn_id,
                                    delta: pending_thinking_delta.clone(),
                                    accumulated: thinking.clone(),
                                },
                            ))
                            .await
                        {
                            tracing::warn!(
                                session_id = %session_id,
                                error = %e,
                                "ReasonAtom: failed to emit reason.thinking.delta event"
                            );
                        }
                        pending_thinking_delta.clear();
                        last_thinking_delta_emit = Instant::now();
                    }
                }
                LlmStreamEvent::ToolCalls(calls) => {
                    tool_calls = calls;
                }
                LlmStreamEvent::Done(metadata) => {
                    // Emit any remaining pending delta before completing
                    if !pending_delta.is_empty()
                        && let Err(e) = self
                            .event_emitter
                            .emit(EventRequest::new(
                                session_id,
                                streaming_event_context.clone(),
                                TextDeltaData {
                                    turn_id: context.turn_id,
                                    delta: pending_delta.clone(),
                                    accumulated: text.clone(),
                                },
                            ))
                            .await
                    {
                        tracing::warn!(
                            session_id = %session_id,
                            error = %e,
                            "ReasonAtom: failed to emit final text.delta event"
                        );
                    }

                    // Emit any remaining pending thinking delta before completing
                    if !pending_thinking_delta.is_empty()
                        && let Err(e) = self
                            .event_emitter
                            .emit(EventRequest::new(
                                session_id,
                                streaming_event_context.clone(),
                                ReasonThinkingDeltaData {
                                    turn_id: context.turn_id,
                                    delta: pending_thinking_delta.clone(),
                                    accumulated: thinking.clone(),
                                },
                            ))
                            .await
                    {
                        tracing::warn!(
                            session_id = %session_id,
                            error = %e,
                            "ReasonAtom: failed to emit final reason.thinking.delta event"
                        );
                    }

                    // Emit reason.thinking.completed if we had any thinking content
                    if !thinking.is_empty()
                        && let Err(e) = self
                            .event_emitter
                            .emit(EventRequest::new(
                                session_id,
                                streaming_event_context.clone(),
                                ReasonThinkingCompletedData {
                                    turn_id: context.turn_id,
                                    thinking: thinking.clone(),
                                },
                            ))
                            .await
                    {
                        tracing::warn!(
                            session_id = %session_id,
                            error = %e,
                            "ReasonAtom: failed to emit reason.thinking.completed event"
                        );
                    }
                    completion_metadata = Some(metadata);
                    break;
                }
                LlmStreamEvent::Error(err) => {
                    // Emit llm.generation failure event (child of reason span)
                    let llm_duration_ms = llm_start.elapsed().as_millis() as u64;
                    let event_context = EventContext::from_atom_context(context).with_span(
                        trace_id.to_string(),
                        Uuid::now_v7().to_string(),
                        Some(reason_span_id.to_string()),
                    );
                    let tools_summary: Vec<ToolDefinitionSummary> =
                        runtime_agent.tools.iter().map(|t| t.into()).collect();
                    let _ = self
                        .event_emitter
                        .emit(EventRequest::new(
                            session_id,
                            event_context,
                            LlmGenerationData::failure(
                                messages_for_event.clone(),
                                tools_summary,
                                runtime_agent.model.clone(),
                                Some(model_with_provider.provider_type.to_string()),
                                err.clone(),
                                Some(llm_duration_ms),
                                time_to_first_token_ms,
                            ),
                        ))
                        .await;
                    return Err(AgentLoopError::llm(err));
                }
            }
        }

        let llm_duration_ms = llm_start.elapsed().as_millis() as u64;

        // 15. Convert completion metadata to TokenUsage
        let usage = completion_metadata.as_ref().and_then(|meta| {
            match (meta.prompt_tokens, meta.completion_tokens) {
                (Some(input), Some(output)) => Some(TokenUsage::with_cache(
                    input,
                    output,
                    meta.cache_read_tokens,
                    meta.cache_creation_tokens,
                )),
                _ => None,
            }
        });

        // 16. Emit llm.generation event (child of reason span)
        let event_context = EventContext::from_atom_context(context).with_span(
            trace_id.to_string(),
            Uuid::now_v7().to_string(),
            Some(reason_span_id.to_string()),
        );
        let tools_summary: Vec<ToolDefinitionSummary> =
            runtime_agent.tools.iter().map(|t| t.into()).collect();
        // Infer finish reasons from content
        let finish_reasons = if !tool_calls.is_empty() {
            Some(vec!["tool_calls".to_string()])
        } else {
            Some(vec!["stop".to_string()])
        };
        if let Err(e) = self
            .event_emitter
            .emit(EventRequest::new(
                session_id,
                event_context,
                LlmGenerationData::success_with_metadata(
                    messages_for_event.clone(),
                    tools_summary,
                    Some(text.clone()).filter(|s| !s.is_empty()),
                    tool_calls.clone(),
                    runtime_agent.model.clone(),
                    Some(model_with_provider.provider_type.to_string()),
                    usage.clone(),
                    Some(llm_duration_ms),
                    time_to_first_token_ms,
                    finish_reasons,
                    None, // response_id
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

        // 17. Build metadata with model and reasoning effort info
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

        // 18. Store and emit message.agent event with metadata and usage
        let has_tool_calls = !tool_calls.is_empty();
        let mut assistant_message = if has_tool_calls {
            Message::assistant_with_tools(&text, tool_calls.clone())
        } else {
            Message::assistant(&text)
        };
        assistant_message.metadata = Some(metadata);
        // Store thinking content for extended thinking models (required for subsequent API calls)
        if !thinking.is_empty() {
            assistant_message.thinking = Some(thinking.clone());
        }

        // Emit message.agent event (this stores the message as an event with proper turn context)
        // Include token usage for tracking (child of reason span)
        let message_event_context = EventContext::from_atom_context(context).with_span(
            trace_id.to_string(),
            Uuid::now_v7().to_string(),
            Some(reason_span_id.to_string()),
        );
        let mut message_agent_data = MessageAgentData::new(assistant_message);
        if let Some(ref u) = usage {
            message_agent_data = message_agent_data.with_usage(u.clone());
        }
        self.event_emitter
            .emit(EventRequest::new(
                session_id,
                message_event_context,
                message_agent_data,
            ))
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
            usage,
        })
    }

    /// Resolve model using priority chain
    /// Resolve model and return both the ModelWithProvider and the resolved model_id (if known)
    async fn resolve_model(
        &self,
        controls_model_id: Option<crate::typed_id::ModelId>,
        session_model_id: Option<crate::typed_id::ModelId>,
        agent_model_id: Option<crate::typed_id::ModelId>,
    ) -> Result<(ModelWithProvider, Option<crate::typed_id::ModelId>)> {
        // Try controls.model_id first (highest priority)
        if let Some(model_id) = controls_model_id
            && let Some(model_with_provider) = self
                .provider_store
                .get_model_with_provider(model_id)
                .await?
        {
            return Ok((model_with_provider, Some(model_id)));
        }

        // Try session.model_id second
        if let Some(model_id) = session_model_id
            && let Some(model_with_provider) = self
                .provider_store
                .get_model_with_provider(model_id)
                .await?
        {
            return Ok((model_with_provider, Some(model_id)));
        }

        // Try agent.default_model_id third
        if let Some(model_id) = agent_model_id
            && let Some(model_with_provider) = self
                .provider_store
                .get_model_with_provider(model_id)
                .await?
        {
            return Ok((model_with_provider, Some(model_id)));
        }

        // Fall back to system default model (no model_id available)
        let model = self
            .provider_store
            .get_default_model()
            .await?
            .ok_or_else(|| {
                AgentLoopError::llm(
                    "No model configured: no model_id in controls, session, or agent, and no system default model is set"
                )
            })?;
        Ok((model, None))
    }

    /// Create LLM driver using the driver registry
    fn create_llm_driver(
        &self,
        model: &ModelWithProvider,
    ) -> Result<crate::llm_driver_registry::BoxedLlmDriver> {
        let provider_type = match model.provider_type {
            crate::llm_models::LlmProviderType::Openai => ProviderType::OpenAI,
            crate::llm_models::LlmProviderType::OpenaiCompletions => {
                ProviderType::OpenAICompletions
            }
            crate::llm_models::LlmProviderType::Anthropic => ProviderType::Anthropic,
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

    /// Resolve image_file references to actual image data
    ///
    /// This method extracts all image_file IDs from the messages and resolves
    /// them to base64-encoded image data using the configured ImageResolver.
    ///
    /// # Returns
    ///
    /// A HashMap mapping image IDs to ResolvedImage data. If no ImageResolver
    /// is configured, or if resolution fails for some images, those images
    /// will simply be missing from the map (and converted to placeholder text).
    async fn resolve_images(&self, messages: &[Message]) -> HashMap<Uuid, ResolvedImage> {
        let mut resolved = HashMap::new();

        // Check if we have an image resolver
        let resolver = match &self.image_resolver {
            Some(r) => r,
            None => return resolved,
        };

        // Collect all unique image_file IDs from all messages
        let image_ids: Vec<Uuid> = messages
            .iter()
            .flat_map(LlmMessage::extract_image_file_ids)
            .collect::<std::collections::HashSet<_>>()
            .into_iter()
            .collect();

        if image_ids.is_empty() {
            return resolved;
        }

        tracing::debug!(
            image_count = image_ids.len(),
            "ReasonAtom: resolving image_file references"
        );

        // Resolve each image
        for image_id in image_ids {
            match resolver.resolve_image(image_id).await {
                Ok(Some(image)) => {
                    resolved.insert(image_id, image);
                }
                Ok(None) => {
                    tracing::warn!(
                        image_id = %image_id,
                        "ReasonAtom: image not found during resolution"
                    );
                }
                Err(e) => {
                    tracing::warn!(
                        image_id = %image_id,
                        error = %e,
                        "ReasonAtom: failed to resolve image"
                    );
                }
            }
        }

        tracing::debug!(
            resolved_count = resolved.len(),
            "ReasonAtom: image resolution complete"
        );

        resolved
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
