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
use serde_json::json;
use std::collections::{BTreeSet, HashMap};
use std::sync::Arc;
use std::time::Instant;
use uuid::Uuid;

use super::{Atom, AtomContext};
use crate::capabilities::CapabilityRegistry;
use crate::driver_registry::{
    DriverRegistry, LlmCallConfigBuilder, LlmCompletionMetadata, LlmMessage, LlmMessageContent,
    LlmMessageRole, LlmStreamEvent, ProviderConfig,
};
use crate::error::{AgentLoopError, Result};
use crate::events::{
    CapabilityUsageData, CapabilityUsageKind, CapabilityUsageRecord, EventContext, EventRequest,
    LlmCompactionInfo, LlmGenerationData, LlmPromptCacheInfo, LlmRequestOptions, LlmRetryInfo,
    LlmToolSearchInfo, OutputMessageCompletedData, OutputMessageDeltaData,
    OutputMessageReplacedData, OutputMessageStartedData, ReasonCompletedData, ReasonItemData,
    ReasonRecoveredData, ReasonStartedData, ReasonThinkingCompletedData, ReasonThinkingDeltaData,
    ReasonThinkingStartedData, RecoveryMode, TokenUsage, ToolDefinitionSummary,
    TranscriptRepairAction, TranscriptRepairedData,
};
use crate::llm_retry::is_transient_error_message;
use crate::message::{Message, MessageRole};
use crate::message_retriever::MessageRetriever;
use crate::openresponses_protocol::{
    CompactInputItem, CompactRequest, compact_output_to_messages, messages_to_compact_input,
};
use crate::output_guardrail::{
    ArmedGuardrail, OutputGuardrailContext, TrippedGuardrail, evaluate_guardrails,
};
use crate::runtime_context::{AssembledTurnContext, assemble_turn_context};
use crate::tool_types::{ToolCall, ToolDefinition};
use crate::traits::{
    AgentStore, DurableToolCallStatus, DurableToolResultStore, EventEmitter, HarnessStore,
    ImageResolver, PartialStreamStore, ProviderStore, ResolvedImage, ResolvedModel, SessionStore,
};
use crate::typed_id::{AgentId, HarnessId, MessageId, SessionId};
use crate::{ErrorDisclosure, UserFacingError, UserFacingErrorContext, user_facing_error_codes};

// ============================================================================
// Helper Functions
// ============================================================================

/// Repair dangling tool calls (EVE-533): for every assistant tool_call with no matching
/// ToolResult, synthesize a well-formed result so the next LLM call does not reject the
/// transcript. Consults `durable_tool_results` (EVE-530) when available:
///
/// - `Settled` row found   → replay the stored result directly.
/// - `Interrupted` row     → replay the stored interrupted error.
/// - `Running` (stale)     → synthesize an "interrupted – uncertain – do not retry" placeholder.
/// - Row not found         → synthesize an "interrupted – not executed – safe to retry" placeholder.
/// - Store error           → synthesize an "interrupted – status unknown – do not retry" placeholder.
///
/// Emits `transcript.repaired` events via `event_emitter` for each repaired call.
async fn repair_dangling_tool_calls(
    messages: &[Message],
    durable_store: Option<&dyn DurableToolResultStore>,
    event_emitter: &dyn EventEmitter,
    session_id: crate::typed_id::SessionId,
    event_context: &EventContext,
    turn_id: &str,
) -> Vec<Message> {
    let mut result = Vec::new();

    for (i, msg) in messages.iter().enumerate() {
        result.push(msg.clone());

        if msg.role != MessageRole::Agent || !msg.has_tool_calls() {
            continue;
        }

        for tc in msg.tool_calls() {
            let has_result = messages[(i + 1)..]
                .iter()
                .any(|m| m.role == MessageRole::ToolResult && m.tool_call_id() == Some(&tc.id));

            if has_result {
                continue;
            }

            // Consult durable_tool_results to determine the best repair strategy.
            let (repair_msg, action) = if let Some(store) = durable_store {
                match store.get_tool_call_status(turn_id, &tc.id).await {
                    Ok(Some(DurableToolCallStatus::Settled { result_json })) => {
                        // A settled result exists in durable storage; deserialize and replay it.
                        let repair = match serde_json::from_value::<crate::tool_types::ToolResult>(
                            result_json.clone(),
                        ) {
                            Ok(tr) => Message::tool_result(&tc.id, tr.result, tr.error),
                            Err(_) => Message::tool_result(&tc.id, Some(result_json), None),
                        };
                        (repair, TranscriptRepairAction::Replay)
                    }
                    Ok(Some(DurableToolCallStatus::Interrupted { result_json })) => {
                        // Settled as interrupted by a prior recovery.
                        let err = result_json
                            .as_ref()
                            .and_then(|v| {
                                serde_json::from_value::<crate::tool_types::ToolResult>(v.clone())
                                    .ok()
                            })
                            .and_then(|tr| tr.error)
                            .unwrap_or_else(|| {
                                "tool execution did not complete before recovery; result unknown"
                                    .to_string()
                            });
                        (
                            Message::tool_result(&tc.id, None, Some(err)),
                            TranscriptRepairAction::Replay,
                        )
                    }
                    Ok(Some(DurableToolCallStatus::Running)) => {
                        // Stale running claim from a dead worker; safe to synthesize.
                        (
                            Message::tool_result(
                                &tc.id,
                                None,
                                Some(
                                    "interrupted - tool execution was interrupted by worker \
                                     failure and the result is uncertain; do not retry \
                                     automatically"
                                        .to_string(),
                                ),
                            ),
                            TranscriptRepairAction::Synthesize,
                        )
                    }
                    Ok(None) => {
                        // No durable record: tool was never dispatched before recovery.
                        (
                            Message::tool_result(
                                &tc.id,
                                None,
                                Some(
                                    "interrupted - tool was not executed before recovery; \
                                     safe to retry"
                                        .to_string(),
                                ),
                            ),
                            TranscriptRepairAction::Synthesize,
                        )
                    }
                    Err(e) => {
                        // Store temporarily unavailable: we cannot know whether the tool ran.
                        tracing::warn!(
                            tool_call_id = %tc.id,
                            error = %e,
                            "transcript repair: durable store error; status unknown"
                        );
                        (
                            Message::tool_result(
                                &tc.id,
                                None,
                                Some(
                                    "interrupted - tool execution status unknown due to \
                                     store error; do not retry automatically"
                                        .to_string(),
                                ),
                            ),
                            TranscriptRepairAction::Synthesize,
                        )
                    }
                }
            } else {
                // No durable store; fall back to generic cancelled.
                (
                    Message::tool_result(
                        &tc.id,
                        None,
                        Some(
                            "cancelled - another message came in before it could be completed"
                                .to_string(),
                        ),
                    ),
                    TranscriptRepairAction::Synthesize,
                )
            };

            // Emit transcript.repaired event for observability.
            let repair_event = EventRequest::new(
                session_id,
                event_context.clone(),
                TranscriptRepairedData {
                    tool_call_id: tc.id.clone(),
                    tool_name: Some(tc.name.clone()),
                    action,
                },
            );
            if let Err(e) = event_emitter.emit(repair_event).await {
                tracing::warn!(
                    tool_call_id = %tc.id,
                    error = %e,
                    "transcript repair: failed to emit transcript.repaired event"
                );
            }

            result.push(repair_msg);
        }
    }

    result
}

/// Known error placeholder texts emitted by the DLQ handler and user_facing_message().
/// These add no conversational value and inflate subsequent LLM requests.
const ERROR_PLACEHOLDER_MESSAGES: &[&str] = &[
    "I encountered an error while processing your request. Please try again later.",
    "The AI provider is experiencing issues. Please try again shortly.",
    "Rate limited by the AI provider. Please wait a moment.",
    "The conversation has become too long for the model to process. Please start a new session or reduce the context size.",
    "There is a misconfiguration with the AI provider. Please contact support.",
    "The AI provider account is out of credits or quota. Add credits or raise the provider account limits to continue.",
];

/// Returns true when a stream event carries assistant output progress.
fn stream_event_advances_stall_deadline(event: &LlmStreamEvent) -> bool {
    match event {
        LlmStreamEvent::TextDelta(delta) | LlmStreamEvent::ThinkingDelta(delta) => {
            !delta.is_empty()
        }
        LlmStreamEvent::ReasonItem {
            encrypted_content,
            summary,
            token_count,
            ..
        } => {
            encrypted_content
                .as_ref()
                .is_some_and(|content| !content.is_empty())
                || summary.iter().any(|item| !item.is_empty())
                || token_count.is_some_and(|count| count > 0)
        }
        LlmStreamEvent::ToolCalls(calls) => !calls.is_empty(),
        LlmStreamEvent::ThinkingSignature(_)
        | LlmStreamEvent::Done(_)
        | LlmStreamEvent::Error(_) => false,
    }
}

fn unix_now_secs() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs()
}

/// Returns true if the message is an error placeholder that should be stripped
/// from the conversation history before sending to the LLM.
fn is_error_placeholder_message(msg: &Message) -> bool {
    if msg.role != MessageRole::Agent {
        return false;
    }
    // Must have no tool calls (pure text-only error message)
    if msg.has_tool_calls() {
        return false;
    }
    if let Some(metadata) = &msg.metadata
        && let Some(serde_json::Value::String(code)) = metadata.get("error_code")
    {
        return matches!(
            code.as_str(),
            user_facing_error_codes::BUDGET_EXHAUSTED
                | user_facing_error_codes::BUDGET_PAUSED
                | user_facing_error_codes::MODEL_UNAVAILABLE
                | user_facing_error_codes::REQUEST_TOO_LARGE
                | user_facing_error_codes::PROVIDER_RATE_LIMITED
                | user_facing_error_codes::PROVIDER_MISCONFIGURED
                | user_facing_error_codes::PROVIDER_QUOTA_EXHAUSTED
                | user_facing_error_codes::PROVIDER_UNAVAILABLE
                | user_facing_error_codes::DEPENDENCY_UNAVAILABLE
                | user_facing_error_codes::PROCESSING_ERROR
        );
    }
    let text = msg.text().unwrap_or("");
    ERROR_PLACEHOLDER_MESSAGES.contains(&text) || is_dynamic_error_placeholder(text)
}

fn append_guarded_thinking_delta(
    armed_guardrails: &mut [ArmedGuardrail],
    thinking: &mut String,
    pending_thinking_delta: &mut String,
    delta: &str,
) -> Option<TrippedGuardrail> {
    thinking.push_str(delta);

    // Thinking streams are user-visible and persisted on completion, so they
    // must pass the same output guardrails as assistant text before any delta
    // is emitted.
    if let Some(t) = evaluate_guardrails(armed_guardrails, thinking, delta) {
        pending_thinking_delta.clear();
        Some(t)
    } else {
        pending_thinking_delta.push_str(delta);
        None
    }
}

/// Per-message error-disclosure override from the most recent user message's
/// controls (mirrors how reasoning effort is resolved). The value is clamped
/// against the capability-configured ceiling in `resolve_error_disclosure`.
fn error_disclosure_override(messages: &[Message]) -> Option<String> {
    messages
        .iter()
        .rev()
        .find(|m| m.role == MessageRole::User)?
        .controls
        .as_ref()?
        .error_disclosure
        .clone()
}

fn is_dynamic_error_placeholder(text: &str) -> bool {
    (text.starts_with("Budget exhausted.") && text.ends_with("Increase the budget to continue."))
        || (text.starts_with("Budget paused.")
            && text.ends_with("Increase or resume the budget to continue."))
        || (text.starts_with("Budget paused with ")
            && text.ends_with("Increase or resume the budget to continue."))
        || (text.starts_with("Soft limit reached.") && text.ends_with("soft limit."))
        || (text.starts_with("The model `") && text.ends_with("Please select a different model."))
}

// ============================================================================
// Input and Output Types
// ============================================================================

/// Input for ReasonAtom
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ReasonInput {
    /// Atom execution context
    pub context: AtomContext,
    /// Harness ID for loading base configuration
    pub harness_id: HarnessId,
    /// Agent ID for loading configuration (optional)
    #[serde(skip_serializing_if = "Option::is_none")]
    pub agent_id: Option<AgentId>,
    /// Organization ID for multi-tenancy tracking
    #[serde(default)]
    pub org_id: i64,
    /// MCP tool definitions from agent's MCP capabilities (pre-resolved)
    /// These are passed from the control-plane since MCP capabilities
    /// are not in the CapabilityRegistry.
    #[serde(default)]
    pub mcp_tool_definitions: Vec<ToolDefinition>,
    /// Previous LLM response ID for stateful continuation.
    /// Enables server-side context caching across reason iterations.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub previous_response_id: Option<String>,
    /// Current iteration number within this turn (1-based).
    /// Used for output.message.started events so UI can show progress.
    #[serde(default = "default_iteration")]
    pub iteration: u32,
}

fn default_iteration() -> u32 {
    1
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
    /// Disclosed user-facing classification of the failure, already filtered
    /// through the resolved error-disclosure mode. Hosts must prefer this over
    /// re-classifying `error`/`text` strings so disclosure stays consistent.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub user_facing_error: Option<UserFacingError>,
    /// Error-disclosure mode that was applied to `user_facing_error`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub error_disclosure: Option<ErrorDisclosure>,
    /// Token usage from the LLM call
    #[serde(skip_serializing_if = "Option::is_none")]
    pub usage: Option<TokenUsage>,
    /// Assistant message emitted by `output.message.completed` for this generation.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub output_message_id: Option<MessageId>,
    /// Streaming latency for this LLM call, when available.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub time_to_first_token_ms: Option<u64>,
    /// LLM provider's response ID for chaining with `previous_response_id`
    #[serde(skip_serializing_if = "Option::is_none")]
    pub response_id: Option<String>,
    /// Resolved locale used for this turn's prompt and backend-authored strings.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub locale: Option<String>,
    /// Merged network access list for URL filtering in tools.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub network_access: Option<crate::network_access::NetworkAccessList>,
}

fn default_max_iterations() -> usize {
    500
}

fn build_request_options(
    config: &crate::driver_registry::LlmCallConfig,
    provider: &str,
) -> Option<LlmRequestOptions> {
    let prompt_cache = config
        .prompt_cache
        .as_ref()
        .filter(|cfg| cfg.enabled)
        .map(|cfg| LlmPromptCacheInfo {
            enabled: true,
            strategy: cfg.strategy,
            provider_mode: match provider {
                "openai" => Some("prompt_cache_key".to_string()),
                "anthropic" => Some("cache_control".to_string()),
                "gemini" => Some(
                    if cfg.gemini_cached_content.is_some() {
                        "cached_content"
                    } else {
                        "implicit"
                    }
                    .to_string(),
                ),
                _ => None,
            },
        });

    let tool_search = config
        .tool_search
        .as_ref()
        .filter(|cfg| cfg.enabled)
        .map(|cfg| LlmToolSearchInfo {
            enabled: true,
            threshold: cfg.threshold,
        });

    let mut provider_options = HashMap::new();
    if provider == "openai" && config.previous_response_id.is_some() {
        provider_options.insert(
            "openai".to_string(),
            json!({ "previous_response_id": true }),
        );
    }
    if provider == "gemini"
        && config
            .prompt_cache
            .as_ref()
            .filter(|cfg| cfg.enabled)
            .and_then(|cfg| cfg.gemini_cached_content.as_ref())
            .is_some()
    {
        provider_options.insert("gemini".to_string(), json!({ "cached_content": true }));
    }

    let request_options = LlmRequestOptions {
        prompt_cache,
        tool_search,
        provider_options,
        metadata: config.metadata.clone(),
    };

    (!request_options.is_empty()).then_some(request_options)
}

fn capability_name_snapshot(registry: &CapabilityRegistry, capability_id: &str) -> Option<String> {
    registry
        .get(capability_id)
        .map(|capability| capability.name().to_string())
}

fn capability_usage_snapshot_records(
    registry: &CapabilityRegistry,
    resolved_capability_configs: &[crate::AgentCapabilityConfig],
    tool_definitions: &[ToolDefinition],
) -> Vec<CapabilityUsageRecord> {
    let mut records = Vec::new();
    let mut seen = BTreeSet::new();

    for config in resolved_capability_configs {
        let capability_id = config.capability_id().to_string();
        if seen.insert((
            "resolved".to_string(),
            capability_id.clone(),
            None::<String>,
        )) {
            records.push(CapabilityUsageRecord {
                capability_name: capability_name_snapshot(registry, &capability_id),
                capability_id,
                usage_kind: CapabilityUsageKind::Resolved,
                tool_name: None,
                usage_count: Some(1),
                duration_ms: None,
            });
        }
    }

    for tool in tool_definitions {
        let Some((capability_id, capability_name)) = tool.capability_attribution() else {
            continue;
        };
        let capability_id = capability_id.to_string();
        let tool_name = tool.name().to_string();
        if seen.insert((
            "exposed".to_string(),
            capability_id.clone(),
            Some(tool_name.clone()),
        )) {
            records.push(CapabilityUsageRecord {
                capability_name: capability_name
                    .map(str::to_string)
                    .or_else(|| capability_name_snapshot(registry, &capability_id)),
                capability_id,
                usage_kind: CapabilityUsageKind::Exposed,
                tool_name: Some(tool_name),
                usage_count: Some(1),
                duration_ms: None,
            });
        }
    }

    records
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
pub struct ReasonAtom {
    harness_store: Arc<dyn HarnessStore>,
    agent_store: Arc<dyn AgentStore>,
    session_store: Arc<dyn SessionStore>,
    message_retriever: Arc<dyn MessageRetriever>,
    provider_store: Arc<dyn ProviderStore>,
    capability_registry: CapabilityRegistry,
    driver_registry: DriverRegistry,
    event_emitter: Arc<dyn EventEmitter>,
    /// Optional image resolver for resolving image_file content parts
    image_resolver: Option<Arc<dyn ImageResolver>>,
    /// Optional file store for capabilities that need filesystem access
    /// (e.g., agent_instructions reads AGENTS.md, skills_discovery scans for skills)
    file_store: Option<Arc<dyn crate::traits::SessionFileSystem>>,
    /// Optional heartbeater for stream-liveness signalling (EVE-531).
    stream_heartbeater: Option<Arc<dyn crate::traits::StreamHeartbeater>>,
    /// Optional provider stall timeout (EVE-531). Default: 120s.
    provider_stall_timeout: Option<std::time::Duration>,
    /// Optional durable tool result store for transcript repair (EVE-533).
    durable_tool_result_store: Option<Arc<dyn DurableToolResultStore>>,
    /// Optional partial-stream store for ContinuePartial recovery (EVE-532).
    partial_stream_store: Option<Arc<dyn PartialStreamStore>>,
}

impl ReasonAtom {
    /// Create a new ReasonAtom
    #[allow(clippy::too_many_arguments)]
    pub fn new(
        harness_store: impl HarnessStore + 'static,
        agent_store: impl AgentStore + 'static,
        session_store: impl SessionStore + 'static,
        message_retriever: impl MessageRetriever + 'static,
        provider_store: impl ProviderStore + 'static,
        capability_registry: CapabilityRegistry,
        driver_registry: DriverRegistry,
        event_emitter: impl EventEmitter + 'static,
    ) -> Self {
        Self {
            harness_store: Arc::new(harness_store),
            agent_store: Arc::new(agent_store),
            session_store: Arc::new(session_store),
            message_retriever: Arc::new(message_retriever),
            provider_store: Arc::new(provider_store),
            capability_registry,
            driver_registry,
            event_emitter: Arc::new(event_emitter),
            image_resolver: None,
            file_store: None,
            stream_heartbeater: None,
            provider_stall_timeout: None,
            durable_tool_result_store: None,
            partial_stream_store: None,
        }
    }

    /// Set the file store for capabilities that need filesystem access.
    ///
    /// Provides filesystem access to capabilities via `SystemPromptContext`.
    /// Capabilities like `agent_instructions` (reads AGENTS.md) and
    /// `skills_discovery` (scans for skills) use this to generate dynamic
    /// system prompt content.
    pub fn with_file_store(
        mut self,
        file_store: Arc<dyn crate::traits::SessionFileSystem>,
    ) -> Self {
        self.file_store = Some(file_store);
        self
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

    /// Set the stream heartbeater for liveness signalling during LLM streaming.
    pub fn with_stream_heartbeater(
        mut self,
        heartbeater: Arc<dyn crate::traits::StreamHeartbeater>,
    ) -> Self {
        self.stream_heartbeater = Some(heartbeater);
        self
    }

    /// Set the provider stall timeout. If no token arrives within this window,
    /// the stream is aborted and the activity fails with a retryable error.
    pub fn with_provider_stall_timeout(mut self, timeout: std::time::Duration) -> Self {
        self.provider_stall_timeout = Some(timeout);
        self
    }

    /// Set the durable tool result store for transcript repair (EVE-533).
    ///
    /// When provided, transcript repair consults this store to replay settled tool
    /// results or synthesize appropriate interrupted placeholders rather than always
    /// emitting a generic "cancelled" message.
    pub fn with_durable_tool_result_store(
        mut self,
        store: Arc<dyn DurableToolResultStore>,
    ) -> Self {
        self.durable_tool_result_store = Some(store);
        self
    }

    /// Set the partial-stream store for ContinuePartial recovery (EVE-532).
    pub fn with_partial_stream_store(mut self, store: Arc<dyn PartialStreamStore>) -> Self {
        self.partial_stream_store = Some(store);
        self
    }
}

#[async_trait]
impl Atom for ReasonAtom {
    type Input = ReasonInput;
    type Output = ReasonResult;

    fn name(&self) -> &'static str {
        "reason"
    }

    async fn execute(&self, input: Self::Input) -> Result<Self::Output> {
        self.execute_inner(input, None).await
    }
}

impl ReasonAtom {
    /// Execute using a pre-assembled turn context.
    ///
    /// Hosts that already assembled turn context for the current reason phase can
    /// pass it through here to avoid reloading messages and rebuilding the agent.
    pub async fn execute_with_assembled_context(
        &self,
        input: ReasonInput,
        assembled: AssembledTurnContext,
    ) -> Result<ReasonResult> {
        self.execute_inner(input, Some(assembled)).await
    }

    async fn emit_capability_usage_snapshot(
        &self,
        session_id: SessionId,
        context: &AtomContext,
        resolved_capability_configs: &[crate::AgentCapabilityConfig],
        tool_definitions: &[ToolDefinition],
    ) {
        let records = capability_usage_snapshot_records(
            &self.capability_registry,
            resolved_capability_configs,
            tool_definitions,
        );
        if records.is_empty() {
            return;
        }

        if let Err(error) = self
            .event_emitter
            .emit(EventRequest::new(
                session_id,
                EventContext::from_atom_context(context),
                CapabilityUsageData { records },
            ))
            .await
        {
            tracing::warn!(
                session_id = %session_id,
                error = %error,
                "ReasonAtom: failed to emit capability.usage event"
            );
        }
    }

    async fn execute_inner(
        &self,
        input: ReasonInput,
        assembled: Option<AssembledTurnContext>,
    ) -> Result<ReasonResult> {
        let ReasonInput {
            context,
            harness_id,
            agent_id,
            org_id,
            mcp_tool_definitions,
            previous_response_id,
            iteration,
        } = input;

        tracing::info!(
            session_id = %context.session_id,
            turn_id = %context.turn_id,
            exec_id = %context.exec_id,
            harness_id = %harness_id,
            agent_id = ?agent_id,
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
                    harness_id,
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

        // Assemble the turn context up-front so the error path below knows
        // the resolved provider/model and the error-disclosure mode even when
        // the LLM call (or the assembly itself) fails.
        let assembled = match assembled {
            Some(assembled) => Ok(assembled),
            None => {
                assemble_turn_context(
                    self.harness_store.as_ref(),
                    self.agent_store.as_ref(),
                    self.session_store.as_ref(),
                    self.message_retriever.as_ref(),
                    self.provider_store.as_ref(),
                    &self.capability_registry,
                    context.session_id,
                    harness_id,
                    agent_id,
                    &mcp_tool_definitions,
                    self.file_store.clone(),
                )
                .await
            }
        };

        let (error_disclosure, error_context, call_result) = match assembled {
            Ok(assembled) => {
                let error_disclosure = crate::capabilities::resolve_error_disclosure(
                    &assembled.resolved_capability_configs,
                    error_disclosure_override(&assembled.messages).as_deref(),
                );
                let error_context = UserFacingErrorContext::default()
                    .with_provider(assembled.model_with_provider.provider_type.to_string())
                    .with_model_id(assembled.model_with_provider.model.clone());
                let call_result = self
                    .execute_llm_call(
                        context.session_id,
                        harness_id,
                        agent_id,
                        org_id,
                        &context,
                        &trace_id,
                        &reason_span_id,
                        previous_response_id,
                        iteration,
                        assembled,
                    )
                    .await;
                (error_disclosure, error_context, call_result)
            }
            Err(error) => (
                ErrorDisclosure::default(),
                UserFacingErrorContext::default(),
                Err(error),
            ),
        };

        // Handle LLM call errors gracefully
        let result = match call_result {
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
                let source_error = e.user_facing_error(error_context);
                let user_error = source_error.apply_disclosure(error_disclosure, Some(&error_msg));
                let user_error_text = user_error.fallback_message();

                // Only emit user-facing error events for non-transient errors.
                // Transient errors (server errors, rate limits, timeouts) will be
                // retried by the durable task engine. Emitting error events on each
                // retry attempt causes duplicate error messages in the UI.
                // The durable worker emits a single error event when all retries
                // are exhausted (DLQ).
                let is_transient = is_transient_error_message(&error_msg);
                let mut output_message_id = None;

                if !is_transient {
                    // Create error message for the user to see
                    let mut error_message = Message::assistant(&user_error_text);
                    let mut metadata = std::collections::HashMap::new();
                    user_error.apply_to_message_metadata(&mut metadata);
                    UserFacingError::apply_disclosure_to_message_metadata(
                        &mut metadata,
                        error_disclosure,
                        &source_error.code,
                    );
                    error_message.metadata = Some(metadata);

                    output_message_id = Some(error_message.id);

                    // Emit output.message.completed event (stores message as event with proper turn context)
                    // output.message.completed is child of reason span
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
                            OutputMessageCompletedData::new(error_message)
                                .with_user_facing_error(&user_error)
                                .with_error_disclosure(error_disclosure),
                        ))
                        .await
                    {
                        tracing::warn!(
                            session_id = %context.session_id,
                            error = %emit_err,
                            "ReasonAtom: failed to emit output.message.completed event for error"
                        );
                    }
                } else {
                    tracing::info!(
                        session_id = %context.session_id,
                        "ReasonAtom: skipping error event for transient LLM error (will be retried)"
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
                    user_facing_error: Some(user_error),
                    error_disclosure: Some(error_disclosure),
                    usage: None,
                    output_message_id,
                    time_to_first_token_ms: None,
                    response_id: None,
                    locale: None,
                    network_access: None,
                }
            }
        };

        Ok(result)
    }

    /// Execute the actual LLM call
    #[allow(clippy::too_many_arguments)]
    async fn execute_llm_call(
        &self,
        session_id: SessionId,
        harness_id: HarnessId,
        agent_id: Option<AgentId>,
        org_id: i64,
        context: &AtomContext,
        trace_id: &str,
        reason_span_id: &str,
        previous_response_id: Option<String>,
        iteration: u32,
        assembled: AssembledTurnContext,
    ) -> Result<ReasonResult> {
        let messages = assembled.messages;
        let prior_usage = assembled.session.usage.clone();
        let model_with_provider = assembled.model_with_provider;
        let resolved_model_id = assembled.resolved_model_id;
        let resolved_locale = assembled.resolved_locale;
        let compaction_config = assembled.compaction_config;
        let resolved_capability_configs = assembled.resolved_capability_configs;
        let runtime_agent = assembled.runtime_agent;
        let embedder_metadata = assembled.embedder_metadata;

        self.emit_capability_usage_snapshot(
            session_id,
            context,
            &resolved_capability_configs,
            &runtime_agent.tools,
        )
        .await;

        // Collect streaming output guardrail providers contributed by enabled
        // capabilities. Each tuple carries the contributing capability id, a
        // borrow of that capability's per-agent config (so arming below doesn't
        // need a second scan), and the provider itself. Capabilities that
        // contribute no guardrails — the common case — are skipped at zero
        // allocation cost.
        let guardrail_providers: Vec<(
            &str,
            &serde_json::Value,
            Arc<dyn crate::output_guardrail::OutputGuardrail>,
        )> = resolved_capability_configs
            .iter()
            .filter_map(|cfg| {
                let cap_id = cfg.capability_ref.as_str();
                let cap = self.capability_registry.get(cap_id)?;
                let guards = cap.output_guardrails();
                if guards.is_empty() {
                    return None;
                }
                Some(
                    guards
                        .into_iter()
                        .map(move |g| (cap_id, &cfg.config, g))
                        .collect::<Vec<_>>(),
                )
            })
            .flatten()
            .collect();

        // 7. Create LLM driver using factory
        let chat_driver = self.create_chat_driver(&model_with_provider)?;

        // 8. Extract reasoning effort from the last user message's controls,
        //    but only if the model actually supports reasoning (per its profile).
        //    This prevents sending unsupported `reasoning` params to non-thinking
        //    models like gpt-4o-mini, which would cause API errors.
        let reasoning_effort = messages
            .iter()
            .rev()
            .find(|m| m.role == MessageRole::User)
            .and_then(|m| m.controls.as_ref())
            .and_then(|c| c.reasoning.as_ref())
            .and_then(|r| r.effort.clone())
            .filter(|effort| {
                // Skip "none" — it means "don't use reasoning"
                if effort.eq_ignore_ascii_case("none") {
                    return false;
                }
                // Check model profile; if profile exists and reasoning is false, strip it.
                // Unknown models (no profile) pass through — let the API decide.
                let profile = crate::model_profiles::get_model_profile(
                    &model_with_provider.provider_type,
                    &model_with_provider.model,
                );
                match profile {
                    Some(p) if !p.reasoning => {
                        tracing::warn!(
                            model = %model_with_provider.model,
                            effort = %effort,
                            "Stripping reasoning_effort: model does not support reasoning"
                        );
                        false
                    }
                    _ => true,
                }
            });

        // 9. Check for an in-flight partial assistant stream from a previous worker (EVE-532).
        // If found, apply the ContinuePartial recovery policy: finalize from accumulated
        // text (if non-empty) or restart clean (if empty/usable partial only).
        if let Some(ref store) = self.partial_stream_store {
            let turn_id_str = context.turn_id.to_string();
            match store.get_partial_stream(session_id, &turn_id_str).await {
                Ok(Some(partial)) if !partial.accumulated.is_empty() => {
                    // Finalize: emit completed from persisted accumulated text.
                    return self
                        .finalize_partial_stream(
                            session_id,
                            context,
                            partial.accumulated,
                            iteration,
                            runtime_agent.max_iterations,
                            &runtime_agent.tools,
                        )
                        .await;
                }
                Ok(Some(_)) => {
                    // Empty accumulated: restart clean — fall through to normal LLM call.
                    // Emit reason.recovered { mode: Restart } for observability.
                    let recovery_ctx = EventContext::from_atom_context(context);
                    let _ = self
                        .event_emitter
                        .emit(EventRequest::new(
                            session_id,
                            recovery_ctx,
                            ReasonRecoveredData {
                                turn_id: context.turn_id,
                                mode: RecoveryMode::Restart,
                                accumulated_len: 0,
                            },
                        ))
                        .await;
                    tracing::info!(
                        session_id = %session_id,
                        turn_id = %context.turn_id,
                        "ReasonAtom: partial stream detected with empty accumulated; restarting clean"
                    );
                }
                Ok(None) => {} // No partial; normal first-run execution.
                Err(e) => {
                    // Best-effort: log and continue with normal execution.
                    tracing::warn!(
                        session_id = %session_id,
                        turn_id = %context.turn_id,
                        error = %e,
                        "ReasonAtom: partial-stream store error; proceeding with normal execution"
                    );
                }
            }
        }

        // 10. Repair dangling tool calls (EVE-533): ensure every assistant tool_call
        // has a matching ToolResult before the LLM call. Consults durable_tool_results
        // when available to replay settled results or synthesize interrupted placeholders.
        let repair_event_context = EventContext::from_atom_context(context);
        let patched_messages = repair_dangling_tool_calls(
            &messages,
            self.durable_tool_result_store.as_deref(),
            self.event_emitter.as_ref(),
            session_id,
            &repair_event_context,
            &context.turn_id.to_string(),
        )
        .await;

        // 9b. Let enabled capabilities build a prompt-facing model view from
        // lossless stored messages. Storage remains unchanged.
        let model_view_providers = crate::capabilities::collect_model_view_providers(
            &resolved_capability_configs,
            &self.capability_registry,
            Some(model_with_provider.model.as_str()),
        );
        let model_view_context = crate::capabilities::ModelViewContext {
            session_id,
            prior_usage: prior_usage.as_ref(),
        };
        let context_messages =
            model_view_providers.apply_model_view(patched_messages, &model_view_context);

        // 10. Resolve images from image_file references (if any)
        //
        // Image resolution converts image_file content parts (which only contain UUIDs)
        // into actual base64-encoded image data that can be sent to LLMs.
        let resolved_images = self.resolve_images(&context_messages).await;

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
                phase: None,
                thinking: None,
                thinking_signature: None,
            });
        }

        // Build messages for llm.generation event (includes system message)
        let messages_for_event: Vec<Message> = if has_system_prompt {
            std::iter::once(Message::system(&runtime_agent.system_prompt))
                .chain(context_messages.iter().cloned())
                .collect()
        } else {
            context_messages.clone()
        };

        // Add conversation messages with resolved images.
        // For user messages with an external_actor, prefix the first text part
        // with the actor's display label so the LLM knows who is speaking.
        // Skip error placeholder messages from prior failed turns — they add
        // no conversational value and inflate the request.
        let mut stripped_error_count = 0u32;
        for msg in &context_messages {
            if is_error_placeholder_message(msg) {
                stripped_error_count += 1;
                continue;
            }
            let mut llm_msg = LlmMessage::from_message_with_images(msg, &resolved_images);
            if msg.role == MessageRole::User
                && let Some(ref actor) = msg.external_actor
            {
                llm_msg.prepend_text_prefix(&format!("[{}] ", actor.display_label()));
            }
            llm_messages.push(llm_msg);
        }
        if stripped_error_count > 0 {
            tracing::info!(
                session_id = %session_id,
                stripped_error_count,
                "ReasonAtom: stripped error placeholder messages from LLM input"
            );
        }

        // 12. Build LLM call config with reasoning effort and metadata
        let mut llm_config_builder = LlmCallConfigBuilder::from(&runtime_agent);
        if let Some(effort) = reasoning_effort.clone() {
            llm_config_builder = llm_config_builder.reasoning_effort(effort);
        }

        // Inject embedder metadata first; system keys added below take precedence
        for (k, v) in &embedder_metadata {
            llm_config_builder = llm_config_builder.with_metadata(k, v.clone());
        }

        // Add metadata for API tracking and debugging
        // These IDs help correlate API requests with Everruns entities
        // TypedId::to_string() produces prefixed format (e.g., "session_abc123")
        llm_config_builder = llm_config_builder
            .with_metadata("session_id", session_id.to_string())
            .with_metadata("harness_id", harness_id.to_string())
            .with_metadata("turn_id", context.turn_id.to_string())
            .with_metadata("exec_id", context.exec_id.to_string())
            .with_metadata("org_id", format!("org_{:032x}", org_id));
        if let Some(agent_id) = agent_id {
            llm_config_builder = llm_config_builder.with_metadata("agent_id", agent_id.to_string());
        }

        // Add model_id if we have one (not available for system default model)
        if let Some(model_id) = &resolved_model_id {
            llm_config_builder = llm_config_builder.with_metadata("model_id", model_id.to_string());
        }

        let llm_config = llm_config_builder
            .previous_response_id(previous_response_id.clone())
            .build();

        tracing::debug!(
            session_id = %session_id,
            turn_id = %context.turn_id,
            model = %runtime_agent.model,
            message_count = %llm_messages.len(),
            "ReasonAtom: calling LLM"
        );

        // 13. Emit output.message.started event BEFORE starting LLM call
        // This allows UI to show a thinking indicator immediately
        let streaming_event_context = EventContext::from_atom_context(context);

        // Arm output guardrails for this stream. Each guardrail sees the
        // assembled system prompt and its own per-capability config (already
        // borrowed in `guardrail_providers` above, so no second scan over
        // `resolved_capability_configs`). Guardrails that decline to arm —
        // e.g. the canary couldn't extract a long-enough sentence — are
        // skipped, leaving the streaming hot path entirely free of work.
        let mut armed_guardrails: Vec<ArmedGuardrail> = Vec::new();
        for (cap_id, cfg, provider) in &guardrail_providers {
            let ctx = OutputGuardrailContext {
                system_prompt: &runtime_agent.system_prompt,
                config: cfg,
            };
            let guardrail_id = provider.id().to_string();
            if let Some(run) = provider.arm(&ctx) {
                armed_guardrails.push(ArmedGuardrail {
                    capability_id: (*cap_id).to_string(),
                    guardrail_id,
                    run,
                });
            }
        }
        let mut tripped: Option<TrippedGuardrail> = None;
        tracing::info!(
            session_id = %session_id,
            turn_id = %context.turn_id,
            "ReasonAtom: emitting output.message.started event"
        );
        if let Err(e) = self
            .event_emitter
            .emit(EventRequest::new(
                session_id,
                streaming_event_context.clone(),
                OutputMessageStartedData {
                    turn_id: context.turn_id,
                    model: Some(runtime_agent.model.clone()),
                    iteration: Some(iteration),
                },
            ))
            .await
        {
            tracing::warn!(
                session_id = %session_id,
                error = %e,
                "ReasonAtom: failed to emit output.message.started event"
            );
        } else {
            tracing::info!(
                session_id = %session_id,
                "ReasonAtom: output.message.started event emitted successfully"
            );
        }

        // Also emit reason.thinking.started if extended thinking is enabled
        let thinking_enabled = reasoning_effort.is_some();
        if thinking_enabled {
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
        }

        // Track LLM call timing
        let llm_start = Instant::now();

        // Try LLM call with automatic compaction on RequestTooLarge.
        // Transient errors (429, 5xx) are retried at the driver level.
        // Stream-level errors are not retried here to avoid duplicate user-visible messages.
        let mut compaction_info: Option<LlmCompactionInfo> = None;
        let mut llm_messages_for_call = llm_messages.clone();

        // 13b. Proactive compaction: check token budget BEFORE calling the LLM.
        // This avoids the latency of a RequestTooLarge round-trip.
        if let Some(ref config) = compaction_config {
            let context_window = crate::model_profiles::get_model_profile(
                &model_with_provider.provider_type,
                &model_with_provider.model,
            )
            .and_then(|p| p.limits.map(|l| l.context as usize))
            .unwrap_or(128_000);

            if crate::capabilities::should_compact_proactively(
                &llm_messages_for_call,
                config,
                context_window,
            ) {
                use crate::capabilities::{
                    CompactionStrategy, aggressive_trim, apply_observation_masking,
                    estimate_total_tokens,
                };
                use crate::events::{
                    CompactionReason, CompactionStepData, ContextCompactedData,
                    ContextCompactingData,
                };

                let messages_before = llm_messages_for_call.len();
                let cascade_start = Instant::now();
                let mut strategies_used: Vec<String> = Vec::new();
                let mut steps: Vec<CompactionStepData> = Vec::new();

                tracing::info!(
                    session_id = %session_id,
                    strategy = %config.strategy,
                    messages = messages_before,
                    "ReasonAtom: proactive compaction triggered (budget threshold exceeded)"
                );

                // Emit context.compacting event
                let _ = self
                    .event_emitter
                    .emit(EventRequest::new(
                        session_id,
                        streaming_event_context.clone(),
                        ContextCompactingData {
                            reason: CompactionReason::ProactiveBudget,
                            strategy: config.strategy.to_string(),
                            messages_before,
                        },
                    ))
                    .await;

                let run_masking = matches!(
                    config.strategy,
                    CompactionStrategy::Auto | CompactionStrategy::ObservationMasking
                );

                // Step 1: Observation masking (free)
                if run_masking {
                    let step_start = Instant::now();
                    let conversation_msgs = if has_system_prompt {
                        &llm_messages_for_call[1..]
                    } else {
                        &llm_messages_for_call[..]
                    };

                    let masking_result =
                        apply_observation_masking(conversation_msgs, &config.observation_masking);

                    if masking_result.masked_count > 0 {
                        let mut new_messages = Vec::new();
                        if has_system_prompt {
                            new_messages.push(llm_messages_for_call[0].clone());
                        }
                        new_messages.extend(masking_result.messages);
                        llm_messages_for_call = new_messages;

                        let step_duration = step_start.elapsed().as_millis() as u64;
                        strategies_used.push("observation_masking".to_string());
                        steps.push(CompactionStepData {
                            strategy: "observation_masking".to_string(),
                            messages_after: llm_messages_for_call.len(),
                            duration_ms: step_duration,
                        });
                    }
                }

                // Step 2: If still over budget after masking, apply aggressive trim
                let budget_tokens = (context_window as f32 * config.budget_percent) as usize;
                if estimate_total_tokens(&llm_messages_for_call) > budget_tokens {
                    let step_start = Instant::now();
                    llm_messages_for_call =
                        aggressive_trim(&llm_messages_for_call, budget_tokens, has_system_prompt);

                    let step_duration = step_start.elapsed().as_millis() as u64;
                    strategies_used.push("aggressive_trim".to_string());
                    steps.push(CompactionStepData {
                        strategy: "aggressive_trim".to_string(),
                        messages_after: llm_messages_for_call.len(),
                        duration_ms: step_duration,
                    });
                }

                let cascade_duration = cascade_start.elapsed().as_millis() as u64;
                let messages_after = llm_messages_for_call.len();

                if !strategies_used.is_empty() {
                    let strategy_used = strategies_used.join("+");

                    let _ = self
                        .event_emitter
                        .emit(EventRequest::new(
                            session_id,
                            streaming_event_context.clone(),
                            ContextCompactedData {
                                strategy_used: strategy_used.clone(),
                                messages_before,
                                messages_after,
                                duration_ms: cascade_duration,
                                steps,
                            },
                        ))
                        .await;

                    tracing::info!(
                        session_id = %session_id,
                        strategy = %strategy_used,
                        messages_before,
                        messages_after,
                        duration_ms = cascade_duration,
                        "ReasonAtom: proactive compaction completed"
                    );
                }
            }
        }

        // 14. Process stream with batched output.message.delta emissions
        // Batch deltas every 100ms to reduce event volume while providing real-time feedback
        const DELTA_BATCH_INTERVAL_MS: u64 = 100;
        let (
            text,
            thinking,
            thinking_signature,
            tool_calls,
            completion_metadata,
            time_to_first_token_ms,
        ) = {
            let mut stream = match chat_driver
                .chat_completion_stream(llm_messages_for_call.clone(), &llm_config)
                .await
            {
                Ok(stream) => stream,
                Err(e) if e.is_request_too_large() => {
                    // Context too large — run compaction cascade
                    use crate::capabilities::{CompactionStrategy, apply_observation_masking};
                    use crate::events::{
                        CompactionReason, CompactionStepData, ContextCompactedData,
                        ContextCompactingData,
                    };

                    let Some(config) = compaction_config.clone() else {
                        tracing::warn!(
                            session_id = %session_id,
                            turn_id = %context.turn_id,
                            "ReasonAtom: context too large and compaction capability is not enabled"
                        );
                        return Err(e);
                    };
                    let messages_before = llm_messages_for_call.len();

                    tracing::info!(
                        session_id = %session_id,
                        turn_id = %context.turn_id,
                        strategy = %config.strategy,
                        messages = messages_before,
                        "ReasonAtom: context too large, attempting compaction"
                    );

                    // Emit context.compacting event
                    let _ = self
                        .event_emitter
                        .emit(EventRequest::new(
                            session_id,
                            streaming_event_context.clone(),
                            ContextCompactingData {
                                reason: CompactionReason::RequestTooLarge,
                                strategy: config.strategy.to_string(),
                                messages_before,
                            },
                        ))
                        .await;

                    let cascade_start = Instant::now();
                    let mut steps: Vec<CompactionStepData> = Vec::new();
                    let mut strategies_used: Vec<String> = Vec::new();

                    // Determine which strategies to run based on config
                    let run_masking = matches!(
                        config.strategy,
                        CompactionStrategy::Auto | CompactionStrategy::ObservationMasking
                    );
                    let run_native = matches!(
                        config.strategy,
                        CompactionStrategy::Auto | CompactionStrategy::Native
                    ) && chat_driver.supports_compact();
                    let run_summarization = matches!(
                        config.strategy,
                        CompactionStrategy::Auto | CompactionStrategy::Summarization
                    );

                    // Step 1: Observation masking (free, no LLM call)
                    if run_masking {
                        let step_start = Instant::now();
                        let conversation_msgs = if has_system_prompt {
                            &llm_messages_for_call[1..]
                        } else {
                            &llm_messages_for_call[..]
                        };

                        let masking_result = apply_observation_masking(
                            conversation_msgs,
                            &config.observation_masking,
                        );

                        if masking_result.masked_count > 0 {
                            let mut new_messages = Vec::new();
                            if has_system_prompt {
                                new_messages.push(llm_messages_for_call[0].clone());
                            }
                            new_messages.extend(masking_result.messages);
                            llm_messages_for_call = new_messages;

                            let step_duration = step_start.elapsed().as_millis() as u64;
                            strategies_used.push("observation_masking".to_string());
                            steps.push(CompactionStepData {
                                strategy: "observation_masking".to_string(),
                                messages_after: llm_messages_for_call.len(),
                                duration_ms: step_duration,
                            });

                            tracing::info!(
                                session_id = %session_id,
                                masked_count = masking_result.masked_count,
                                duration_ms = step_duration,
                                "ReasonAtom: observation masking applied"
                            );
                        }
                    }

                    // Step 2: Native provider compaction
                    if run_native {
                        let step_start = Instant::now();
                        let messages_to_compact = if has_system_prompt {
                            &llm_messages_for_call[1..]
                        } else {
                            &llm_messages_for_call[..]
                        };

                        let compact_input = messages_to_compact_input(messages_to_compact);
                        let input_count = compact_input.len();

                        let compact_request = CompactRequest {
                            model: runtime_agent.model.clone(),
                            input: compact_input,
                            previous_response_id: previous_response_id.clone(),
                            instructions: if has_system_prompt {
                                Some(runtime_agent.system_prompt.clone())
                            } else {
                                None
                            },
                        };

                        match chat_driver.compact(compact_request).await {
                            Ok(Some(compact_response)) => {
                                let (compacted_messages, compaction_items) =
                                    compact_output_to_messages(&compact_response.output);

                                let input_tokens_after = compact_response
                                    .usage
                                    .as_ref()
                                    .and_then(|u| u.output_tokens);

                                compaction_info = Some(LlmCompactionInfo::new(
                                    Some(input_count as u32),
                                    input_tokens_after,
                                    Some(step_start.elapsed().as_millis() as u64),
                                ));

                                let mut compacted_llm_messages = Vec::new();
                                if has_system_prompt {
                                    compacted_llm_messages.push(llm_messages_for_call[0].clone());
                                }
                                compacted_llm_messages.extend(compacted_messages);

                                for item in compaction_items {
                                    if let CompactInputItem::Compaction { encrypted_content } = item
                                    {
                                        compacted_llm_messages.push(LlmMessage {
                                            role: LlmMessageRole::System,
                                            content: LlmMessageContent::Text(format!(
                                                "[COMPACTED_CONTEXT:{encrypted_content}]"
                                            )),
                                            tool_calls: None,
                                            tool_call_id: None,
                                            phase: None,
                                            thinking: None,
                                            thinking_signature: None,
                                        });
                                    }
                                }

                                llm_messages_for_call = compacted_llm_messages;

                                let step_duration = step_start.elapsed().as_millis() as u64;
                                strategies_used.push("native".to_string());
                                steps.push(CompactionStepData {
                                    strategy: "native".to_string(),
                                    messages_after: llm_messages_for_call.len(),
                                    duration_ms: step_duration,
                                });

                                tracing::info!(
                                    session_id = %session_id,
                                    duration_ms = step_duration,
                                    messages_after = llm_messages_for_call.len(),
                                    "ReasonAtom: native compaction applied"
                                );
                            }
                            Ok(None) | Err(_) => {
                                tracing::warn!(
                                    session_id = %session_id,
                                    "ReasonAtom: native compaction unavailable, continuing cascade"
                                );
                            }
                        }
                    }

                    // Step 3: Summarization (if configured, and native didn't run or isn't available)
                    // Only run if we haven't done native compaction (which already compressed everything)
                    if run_summarization && !strategies_used.contains(&"native".to_string()) {
                        use crate::capabilities::{
                            build_summarization_prompt, build_summary_message,
                            format_messages_for_summarization,
                        };

                        let step_start = Instant::now();
                        let conversation_msgs = if has_system_prompt {
                            &llm_messages_for_call[1..]
                        } else {
                            &llm_messages_for_call[..]
                        };

                        // Keep the last few messages verbatim, summarize the rest
                        let keep_recent = 10.min(conversation_msgs.len());
                        let to_summarize =
                            &conversation_msgs[..conversation_msgs.len() - keep_recent];
                        let recent = &conversation_msgs[conversation_msgs.len() - keep_recent..];

                        if !to_summarize.is_empty() {
                            let summary_prompt = build_summarization_prompt(&config.summarization);
                            let messages_text = format_messages_for_summarization(to_summarize);

                            // Use the LLM to generate a summary
                            let summary_messages = vec![
                                LlmMessage {
                                    role: LlmMessageRole::System,
                                    content: LlmMessageContent::Text(summary_prompt),
                                    tool_calls: None,
                                    tool_call_id: None,
                                    phase: None,
                                    thinking: None,
                                    thinking_signature: None,
                                },
                                LlmMessage {
                                    role: LlmMessageRole::User,
                                    content: LlmMessageContent::Text(messages_text),
                                    tool_calls: None,
                                    tool_call_id: None,
                                    phase: None,
                                    thinking: None,
                                    thinking_signature: None,
                                },
                            ];

                            let summary_config = crate::driver_registry::LlmCallConfig {
                                model: config
                                    .summarization
                                    .model
                                    .clone()
                                    .unwrap_or_else(|| runtime_agent.model.clone()),
                                temperature: Some(0.0),
                                max_tokens: Some(2000),
                                tools: vec![],
                                reasoning_effort: None,
                                metadata: HashMap::new(),
                                previous_response_id: None,
                                tool_search: None,
                                prompt_cache: None,
                                openrouter_routing: None,
                            };

                            match chat_driver
                                .chat_completion(summary_messages, &summary_config)
                                .await
                            {
                                Ok(response) => {
                                    let summary_text = response.text;
                                    let summary_msg = build_summary_message(&summary_text);

                                    let mut new_messages = Vec::new();
                                    if has_system_prompt {
                                        new_messages.push(llm_messages_for_call[0].clone());
                                    }
                                    new_messages.push(summary_msg);
                                    new_messages.extend_from_slice(recent);
                                    llm_messages_for_call = new_messages;

                                    let step_duration = step_start.elapsed().as_millis() as u64;
                                    strategies_used.push("summarization".to_string());
                                    steps.push(CompactionStepData {
                                        strategy: "summarization".to_string(),
                                        messages_after: llm_messages_for_call.len(),
                                        duration_ms: step_duration,
                                    });

                                    tracing::info!(
                                        session_id = %session_id,
                                        duration_ms = step_duration,
                                        messages_after = llm_messages_for_call.len(),
                                        "ReasonAtom: summarization applied"
                                    );
                                }
                                Err(e) => {
                                    tracing::warn!(
                                        session_id = %session_id,
                                        error = %e,
                                        "ReasonAtom: summarization failed, continuing"
                                    );
                                }
                            }
                        }
                    }

                    // Step 4: Aggressive trim (last resort — drop oldest messages)
                    // Only run if previous strategies didn't reduce context enough.
                    // Use a generous target (half the estimated original size).
                    if strategies_used.is_empty()
                        || llm_messages_for_call.len() > messages_before / 2
                    {
                        use crate::capabilities::aggressive_trim;
                        let step_start = Instant::now();
                        // Target: keep roughly half the messages by token budget
                        let estimated_total =
                            crate::capabilities::estimate_total_tokens(&llm_messages_for_call);
                        let target = estimated_total / 2;
                        let trimmed =
                            aggressive_trim(&llm_messages_for_call, target, has_system_prompt);
                        if trimmed.len() < llm_messages_for_call.len() {
                            llm_messages_for_call = trimmed;
                            let step_duration = step_start.elapsed().as_millis() as u64;
                            strategies_used.push("aggressive_trim".to_string());
                            steps.push(CompactionStepData {
                                strategy: "aggressive_trim".to_string(),
                                messages_after: llm_messages_for_call.len(),
                                duration_ms: step_duration,
                            });
                            tracing::info!(
                                session_id = %session_id,
                                messages_after = llm_messages_for_call.len(),
                                "ReasonAtom: aggressive trim applied (last resort)"
                            );
                        }
                    }

                    let cascade_duration = cascade_start.elapsed().as_millis() as u64;
                    let messages_after = llm_messages_for_call.len();

                    // Emit context.compacted event
                    let strategy_used = if strategies_used.is_empty() {
                        "none".to_string()
                    } else {
                        strategies_used.join("+")
                    };

                    let _ = self
                        .event_emitter
                        .emit(EventRequest::new(
                            session_id,
                            streaming_event_context.clone(),
                            ContextCompactedData {
                                strategy_used: strategy_used.clone(),
                                messages_before,
                                messages_after,
                                duration_ms: cascade_duration,
                                steps,
                            },
                        ))
                        .await;

                    tracing::info!(
                        session_id = %session_id,
                        strategy = %strategy_used,
                        messages_before,
                        messages_after,
                        duration_ms = cascade_duration,
                        "ReasonAtom: compaction cascade completed, retrying LLM call"
                    );

                    chat_driver
                        .chat_completion_stream(llm_messages_for_call.clone(), &llm_config)
                        .await?
                }
                Err(e) => return Err(e),
            };

            let mut text = String::new();
            let mut thinking = String::new();
            let mut thinking_signature: Option<String> = None;
            let mut tool_calls = Vec::new();
            let mut completion_metadata: Option<LlmCompletionMetadata> = None;
            let mut pending_delta = String::new();
            let mut pending_thinking_delta = String::new();
            let mut last_delta_emit = Instant::now();
            let mut last_thinking_delta_emit = Instant::now();
            let mut time_to_first_token_ms: Option<u64> = None;

            // EVE-531: stall timeout + keepalive heartbeat for stream-liveness
            let stall_timeout = self
                .provider_stall_timeout
                .unwrap_or(std::time::Duration::from_secs(120));
            let mut stall_sleep = Box::pin(tokio::time::sleep(stall_timeout));
            let mut keepalive_ticker = tokio::time::interval(std::time::Duration::from_secs(12));
            keepalive_ticker.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Skip);
            keepalive_ticker.tick().await; // consume immediate first tick
            let mut last_stream_heartbeat = Instant::now();
            // Tracks the wall-clock time of the last actual token received.
            // Updated only on content events; keepalive heartbeats use this
            // so the control plane can distinguish "alive/slow" from "making
            // progress" without conflating keepalive pings with real tokens.
            let mut last_token_at_unix: u64 = unix_now_secs();

            loop {
                let event = tokio::select! {
                    biased;
                    next = stream.next() => match next {
                        Some(e) => e,
                        None => break,
                    },
                    _ = &mut stall_sleep => {
                        tracing::warn!(
                            session_id = %session_id,
                            turn_id = %context.turn_id,
                            stall_secs = stall_timeout.as_secs(),
                            "ReasonAtom: provider stream stall timeout"
                        );
                        return Err(AgentLoopError::llm(format!(
                            "provider stream stall: no tokens for {}s",
                            stall_timeout.as_secs()
                        )));
                    },
                    _ = keepalive_ticker.tick() => {
                        if let Some(ref hb) = self.stream_heartbeater {
                            hb.heartbeat(crate::traits::StreamProgress {
                                accumulated_len: text.len() + thinking.len(),
                                last_delta_at: last_token_at_unix,
                            })
                            .await;
                            last_stream_heartbeat = Instant::now();
                        }
                        continue;
                    },
                };
                let event = event?;
                let advances_stall_deadline = stream_event_advances_stall_deadline(&event);
                if advances_stall_deadline {
                    stall_sleep
                        .as_mut()
                        .reset(tokio::time::Instant::now() + stall_timeout);
                    last_token_at_unix = unix_now_secs();
                }
                match event {
                    LlmStreamEvent::TextDelta(delta) => {
                        if delta.is_empty() {
                            continue;
                        }
                        // Track time-to-first-token on first non-empty delta
                        if time_to_first_token_ms.is_none() {
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

                        // Run output guardrails on the new accumulated text.
                        // Cheap by contract — runs in the streaming hot path.
                        // On block: suppress the pending delta (the bad text
                        // never reaches the client as a delta), record the
                        // trip, and break the loop. The replacement message is
                        // emitted below after the streaming block.
                        if !armed_guardrails.is_empty()
                            && let Some(t) =
                                evaluate_guardrails(&mut armed_guardrails, &text, &delta)
                        {
                            tracing::warn!(
                                session_id = %session_id,
                                turn_id = %context.turn_id,
                                guardrail_capability_id = %t.capability_id,
                                guardrail_id = %t.guardrail_id,
                                reason_code = %t.block.reason_code,
                                "ReasonAtom: output guardrail tripped, replacing assistant message"
                            );
                            pending_delta.clear();
                            tripped = Some(t);
                            break;
                        }

                        // Emit batched delta if interval elapsed
                        if last_delta_emit.elapsed().as_millis() as u64 >= DELTA_BATCH_INTERVAL_MS
                            && !pending_delta.is_empty()
                        {
                            if let Err(e) = self
                                .event_emitter
                                .emit(EventRequest::new(
                                    session_id,
                                    streaming_event_context.clone(),
                                    OutputMessageDeltaData {
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
                                    "ReasonAtom: failed to emit output.message.delta event"
                                );
                            }
                            pending_delta.clear();
                            last_delta_emit = Instant::now();
                        }
                    }
                    LlmStreamEvent::ThinkingDelta(delta) => {
                        if delta.is_empty() {
                            continue;
                        }
                        if let Some(t) = append_guarded_thinking_delta(
                            &mut armed_guardrails,
                            &mut thinking,
                            &mut pending_thinking_delta,
                            &delta,
                        ) {
                            tracing::warn!(
                                session_id = %session_id,
                                guardrail_capability_id = %t.capability_id,
                                guardrail_id = %t.guardrail_id,
                                "ReasonAtom: output guardrail tripped on thinking stream, replacing assistant message"
                            );
                            tripped = Some(t);
                            break;
                        }
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
                    LlmStreamEvent::ThinkingSignature(signature) => {
                        // Capture the cryptographic signature for thinking content (required to send it back)
                        tracing::debug!(
                            session_id = %session_id,
                            signature_len = signature.len(),
                            "ReasonAtom: received ThinkingSignature from LLM"
                        );
                        thinking_signature = Some(signature);
                    }
                    LlmStreamEvent::ReasonItem {
                        provider,
                        model,
                        item_id,
                        encrypted_content,
                        summary,
                        token_count,
                    } => {
                        // Preserve the opaque artifact as the assistant message's
                        // thinking_signature so the next request can replay
                        // reasoning context, and emit a durable reason.item event
                        // for trace/session review. Plaintext reasoning content is
                        // never included.
                        if let Some(sig) = encrypted_content.as_ref() {
                            tracing::debug!(
                                session_id = %session_id,
                                signature_len = sig.len(),
                                provider = %provider,
                                item_id = %item_id,
                                "ReasonAtom: captured encrypted reasoning content from ReasonItem"
                            );
                            thinking_signature = Some(sig.clone());
                        }
                        if let Err(e) = self
                            .event_emitter
                            .emit(EventRequest::new(
                                session_id,
                                streaming_event_context.clone(),
                                ReasonItemData {
                                    turn_id: context.turn_id,
                                    provider,
                                    model,
                                    item_id,
                                    encrypted_content,
                                    summary,
                                    token_count,
                                },
                            ))
                            .await
                        {
                            tracing::warn!(
                                session_id = %session_id,
                                error = %e,
                                "ReasonAtom: failed to emit reason.item event"
                            );
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
                                    OutputMessageDeltaData {
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
                                "ReasonAtom: failed to emit final output.message.delta event"
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
                        completion_metadata = Some(*metadata);
                        break;
                    }
                    LlmStreamEvent::Error(err) => {
                        // If we already collected valid tool calls or text before
                        // the error arrived, treat it as a partial success. This
                        // handles OpenAI Responses API behaviour where a trailing
                        // server_error can follow fully-streamed function calls.
                        let has_partial_output = !tool_calls.is_empty() || !text.is_empty();

                        if has_partial_output {
                            tracing::warn!(
                                session_id = %session_id,
                                error = %err,
                                tool_call_count = tool_calls.len(),
                                text_len = text.len(),
                                "ReasonAtom: trailing stream error after valid output — treating as partial success"
                            );
                            // Break out of the stream loop and use the output
                            // we already collected. completion_metadata will be
                            // None since we never got a Done event.
                            break;
                        }

                        // No useful output collected — treat as a real failure.
                        let llm_duration_ms = llm_start.elapsed().as_millis() as u64;
                        let event_context = EventContext::from_atom_context(context).with_span(
                            trace_id.to_string(),
                            Uuid::now_v7().to_string(),
                            Some(reason_span_id.to_string()),
                        );
                        let tools_summary: Vec<ToolDefinitionSummary> =
                            runtime_agent.tools.iter().map(|t| t.into()).collect();
                        let generation_data = LlmGenerationData::failure(
                            messages_for_event.clone(),
                            tools_summary,
                            runtime_agent.model.clone(),
                            Some(model_with_provider.provider_type.to_string()),
                            err.clone(),
                            Some(llm_duration_ms),
                            time_to_first_token_ms,
                        );
                        let _ = self
                            .event_emitter
                            .emit(EventRequest::new(
                                session_id,
                                event_context,
                                generation_data,
                            ))
                            .await;
                        return Err(AgentLoopError::llm(err));
                    }
                }
                // Per-event heartbeat after processing the event, so accumulated_len
                // reflects the just-received tokens. Throttled to every 5s.
                if last_stream_heartbeat.elapsed().as_millis() as u64 >= 5_000
                    && let Some(ref hb) = self.stream_heartbeater
                {
                    hb.heartbeat(crate::traits::StreamProgress {
                        accumulated_len: text.len() + thinking.len(),
                        last_delta_at: last_token_at_unix,
                    })
                    .await;
                    last_stream_heartbeat = Instant::now();
                }
            }
            (
                text,
                thinking,
                thinking_signature,
                tool_calls,
                completion_metadata,
                time_to_first_token_ms,
            )
        };
        let (mut text, mut thinking, thinking_signature, mut tool_calls) =
            (text, thinking, thinking_signature, tool_calls);

        // If a streaming output guardrail tripped, emit
        // output.message.replaced and overwrite the assistant output now so
        // every downstream event (llm.generation, output.message.completed)
        // carries the replacement instead of the model's withheld tokens.
        // The original tokens are never persisted or replayed.
        if let Some(ref t) = tripped {
            let replaced_event_context = EventContext::from_atom_context(context).with_span(
                trace_id.to_string(),
                Uuid::now_v7().to_string(),
                Some(reason_span_id.to_string()),
            );
            if let Err(e) = self
                .event_emitter
                .emit(EventRequest::new(
                    session_id,
                    replaced_event_context,
                    OutputMessageReplacedData {
                        turn_id: context.turn_id,
                        guardrail_capability_id: t.capability_id.clone(),
                        guardrail_id: t.guardrail_id.clone(),
                        reason_code: t.block.reason_code.clone(),
                        replacement: t.block.replacement.clone(),
                    },
                ))
                .await
            {
                tracing::warn!(
                    session_id = %session_id,
                    error = %e,
                    "ReasonAtom: failed to emit output.message.replaced event"
                );
            }
            text = t.block.replacement.clone();
            tool_calls.clear();
            thinking.clear();
        }

        let llm_duration_ms = llm_start.elapsed().as_millis() as u64;

        // Extract response_id from completion metadata for chaining and OTel
        let response_id = completion_metadata
            .as_ref()
            .and_then(|meta| meta.response_id.clone());

        // 15. Convert completion metadata to TokenUsage.
        //
        // Cost is tracked as two independent values: the provider's authoritative
        // inline cost when present (e.g. OpenRouter's usage.cost), and a price-table
        // estimate from the model profile computed whenever profile cost data
        // exists. Keeping both lets downstream consumers prefer the actual charge
        // while still reconciling estimate-vs-actual drift.
        let usage = completion_metadata.as_ref().and_then(|meta| {
            match (meta.prompt_tokens, meta.completion_tokens) {
                (Some(input), Some(output)) => {
                    let actual_cost_usd = meta.provider_cost_usd;
                    let estimated_cost_usd = crate::model_profiles::estimate_cost_usd(
                        &model_with_provider.provider_type,
                        &runtime_agent.model,
                        input,
                        output,
                    );
                    Some(
                        TokenUsage::with_cache(
                            input,
                            output,
                            meta.cache_read_tokens,
                            meta.cache_creation_tokens,
                        )
                        .with_cost(actual_cost_usd, estimated_cost_usd),
                    )
                }
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
        // Extract retry info from completion metadata (if retries occurred)
        let retry_info = completion_metadata
            .as_ref()
            .and_then(|meta| meta.retry_metadata.as_ref())
            .filter(|rm| rm.had_retries())
            .map(|rm| LlmRetryInfo {
                attempts: rm.attempts,
                total_wait_ms: rm.total_retry_wait.as_millis() as u64,
            });
        // Build LlmGenerationData with retry and compaction info
        let mut generation_data = LlmGenerationData::success_with_retry(
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
            response_id.clone(),
            retry_info,
        );

        // Add compaction info if compaction was performed
        if let Some(info) = compaction_info {
            generation_data = generation_data.with_compaction(info);
        }

        if let Some(request_options) =
            build_request_options(&llm_config, &model_with_provider.provider_type.to_string())
        {
            generation_data = generation_data.with_request_options(request_options);
        }

        if let Err(e) = self
            .event_emitter
            .emit(EventRequest::new(
                session_id,
                event_context,
                generation_data,
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

        // 18. Store and emit output.message.completed event with metadata and usage
        let has_tool_calls = !tool_calls.is_empty();
        let mut assistant_message = if has_tool_calls {
            Message::assistant_with_tools(&text, tool_calls.clone())
        } else {
            Message::assistant(&text)
        };
        // Use the API-provided phase when available (preserving the provider's value),
        // otherwise derive from state: Commentary for intermediate iterations (with tool
        // calls), FinalAnswer for the completed response.
        assistant_message.phase = completion_metadata
            .as_ref()
            .and_then(|meta| meta.phase.as_deref())
            .and_then(crate::message::ExecutionPhase::from_provider_str)
            .or_else(|| {
                Some(crate::message::ExecutionPhase::from_has_tool_calls(
                    has_tool_calls,
                ))
            });
        assistant_message.metadata = Some(metadata);
        // Store thinking content and signature for extended thinking models
        // Both are required for subsequent API calls when thinking is enabled
        if !thinking.is_empty() {
            assistant_message.thinking = Some(thinking.clone());
            assistant_message.thinking_signature = thinking_signature.clone();
        }
        let output_message_id = assistant_message.id;

        // Emit output.message.completed event (this stores the message as an event with proper turn context)
        // Include token usage for tracking (child of reason span)
        let message_event_context = EventContext::from_atom_context(context).with_span(
            trace_id.to_string(),
            Uuid::now_v7().to_string(),
            Some(reason_span_id.to_string()),
        );
        let mut output_message_data = OutputMessageCompletedData::new(assistant_message);
        if let Some(ref u) = usage {
            output_message_data = output_message_data.with_usage(u.clone());
        }
        self.event_emitter
            .emit(EventRequest::new(
                session_id,
                message_event_context,
                output_message_data,
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
            user_facing_error: None,
            error_disclosure: None,
            usage,
            output_message_id: Some(output_message_id),
            time_to_first_token_ms,
            response_id,
            locale: resolved_locale,
            network_access: runtime_agent.network_access.clone(),
        })
    }

    /// Finalize a partial assistant stream without making a new provider call (EVE-532).
    ///
    /// Emits `output.message.started`, `output.message.completed` from the persisted
    /// `accumulated` text, and `reason.recovered { mode: Finalize }`.
    async fn finalize_partial_stream(
        &self,
        session_id: SessionId,
        context: &AtomContext,
        accumulated: String,
        iteration: u32,
        max_iterations: usize,
        tool_definitions: &[ToolDefinition],
    ) -> Result<ReasonResult> {
        let event_context = EventContext::from_atom_context(context);
        let turn_id = context.turn_id;

        // Signal that output is starting (keeps the streaming protocol intact).
        let _ = self
            .event_emitter
            .emit(EventRequest::new(
                session_id,
                event_context.clone(),
                OutputMessageStartedData {
                    turn_id,
                    model: None,
                    iteration: Some(iteration),
                },
            ))
            .await;

        // Build the assistant message from accumulated text and persist via event.
        let assistant_message = Message::assistant(&accumulated);
        let output_message_id = assistant_message.id;
        self.event_emitter
            .emit(EventRequest::new(
                session_id,
                event_context.clone(),
                OutputMessageCompletedData::new(assistant_message),
            ))
            .await?;

        // Emit observability event.
        let accumulated_len = accumulated.len();
        let _ = self
            .event_emitter
            .emit(EventRequest::new(
                session_id,
                event_context.clone(),
                ReasonRecoveredData {
                    turn_id,
                    mode: RecoveryMode::Finalize,
                    accumulated_len,
                },
            ))
            .await;

        tracing::info!(
            session_id = %session_id,
            turn_id = %turn_id,
            accumulated_len,
            "ReasonAtom: finalized partial stream from persisted accumulated text"
        );

        Ok(ReasonResult {
            success: true,
            text: accumulated,
            tool_calls: vec![],
            has_tool_calls: false,
            tool_definitions: tool_definitions.to_vec(),
            max_iterations,
            error: None,
            user_facing_error: None,
            error_disclosure: None,
            usage: None,
            output_message_id: Some(output_message_id),
            time_to_first_token_ms: None,
            response_id: None,
            locale: None,
            network_access: None,
        })
    }

    /// Resolve model using priority chain: controls > session > agent > harness > system default
    /// Create LLM driver using the driver registry
    fn create_chat_driver(
        &self,
        model: &ResolvedModel,
    ) -> Result<crate::driver_registry::BoxedChatDriver> {
        self.driver_registry
            .create_chat_driver(&ProviderConfig::from(model))
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
    use crate::driver_registry::{LlmCallConfig, PromptCacheConfig, PromptCacheStrategy};
    use std::collections::HashMap;

    struct BlockWhenDeltaContains {
        needle: &'static str,
    }

    impl crate::output_guardrail::OutputGuardrailRun for BlockWhenDeltaContains {
        fn check(
            &mut self,
            _accumulated: &str,
            delta: &str,
        ) -> crate::output_guardrail::GuardrailDecision {
            if delta.contains(self.needle) {
                crate::output_guardrail::GuardrailDecision::block("test_leak", "[blocked]")
            } else {
                crate::output_guardrail::GuardrailDecision::Pass
            }
        }
    }

    fn test_armed_guardrail() -> ArmedGuardrail {
        ArmedGuardrail {
            capability_id: "test_capability".to_string(),
            guardrail_id: "test_guardrail".to_string(),
            run: Box::new(BlockWhenDeltaContains { needle: "secret" }),
        }
    }

    #[test]
    fn test_append_guarded_thinking_delta_blocks_before_pending_emit() {
        let mut guardrails = vec![test_armed_guardrail()];
        let mut thinking = "safe ".to_string();
        let mut pending = "safe ".to_string();

        let tripped = append_guarded_thinking_delta(
            &mut guardrails,
            &mut thinking,
            &mut pending,
            "secret instructions",
        )
        .expect("thinking delta should trip guardrail");

        assert_eq!(tripped.capability_id, "test_capability");
        assert_eq!(tripped.guardrail_id, "test_guardrail");
        assert_eq!(tripped.block.reason_code, "test_leak");
        assert_eq!(tripped.block.replacement, "[blocked]");
        assert_eq!(thinking, "safe secret instructions");
        assert!(pending.is_empty());
    }

    #[test]
    fn test_append_guarded_thinking_delta_allows_safe_pending_emit() {
        let mut guardrails = vec![test_armed_guardrail()];
        let mut thinking = String::new();
        let mut pending = String::new();

        let tripped = append_guarded_thinking_delta(
            &mut guardrails,
            &mut thinking,
            &mut pending,
            "ordinary reasoning",
        );

        assert!(tripped.is_none());
        assert_eq!(thinking, "ordinary reasoning");
        assert_eq!(pending, "ordinary reasoning");
    }

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
        assert_eq!(result.max_iterations, 500);
    }

    #[test]
    fn test_capability_usage_snapshot_keeps_resolved_and_exposed_separate() {
        let registry = CapabilityRegistry::with_builtins();
        let tool = ToolDefinition::Builtin(crate::tool_types::BuiltinTool {
            name: "demo_tool".to_string(),
            display_name: None,
            description: "demo".to_string(),
            parameters: json!({"type": "object"}),
            policy: crate::tool_types::ToolPolicy::Auto,
            category: None,
            deferrable: crate::tool_types::DeferrablePolicy::default(),
            hints: crate::tool_types::ToolHints::default(),
            full_parameters: None,
        })
        .with_capability_attribution("cap:demo", Some("Demo Capability"));

        let records = capability_usage_snapshot_records(
            &registry,
            &[crate::AgentCapabilityConfig::new("current_time")],
            &[tool],
        );

        assert!(records.iter().any(|record| {
            matches!(record.usage_kind, CapabilityUsageKind::Resolved)
                && record.capability_id == "current_time"
                && record.tool_name.is_none()
        }));
        assert!(records.iter().any(|record| {
            matches!(record.usage_kind, CapabilityUsageKind::Exposed)
                && record.capability_id == "cap:demo"
                && record.tool_name.as_deref() == Some("demo_tool")
        }));
    }

    #[test]
    fn stream_stall_deadline_ignores_empty_keepalive_events() {
        assert!(!stream_event_advances_stall_deadline(
            &LlmStreamEvent::TextDelta(String::new())
        ));
        assert!(!stream_event_advances_stall_deadline(
            &LlmStreamEvent::ThinkingDelta(String::new())
        ));
        assert!(!stream_event_advances_stall_deadline(
            &LlmStreamEvent::ThinkingSignature("signature".to_string())
        ));
        assert!(!stream_event_advances_stall_deadline(
            &LlmStreamEvent::ReasonItem {
                provider: "openai".to_string(),
                model: None,
                item_id: "item_1".to_string(),
                encrypted_content: None,
                summary: vec![String::new()],
                token_count: Some(0),
            }
        ));
    }

    #[test]
    fn stream_stall_deadline_advances_on_output_progress() {
        assert!(stream_event_advances_stall_deadline(
            &LlmStreamEvent::TextDelta("hello".to_string())
        ));
        assert!(stream_event_advances_stall_deadline(
            &LlmStreamEvent::ThinkingDelta("thinking".to_string())
        ));
        assert!(stream_event_advances_stall_deadline(
            &LlmStreamEvent::ReasonItem {
                provider: "openai".to_string(),
                model: Some("gpt-5.4".to_string()),
                item_id: "item_1".to_string(),
                encrypted_content: Some("encrypted".to_string()),
                summary: vec![],
                token_count: None,
            }
        ));
        assert!(stream_event_advances_stall_deadline(
            &LlmStreamEvent::ReasonItem {
                provider: "openai".to_string(),
                model: None,
                item_id: "item_2".to_string(),
                encrypted_content: None,
                summary: vec!["summary".to_string()],
                token_count: None,
            }
        ));
        assert!(stream_event_advances_stall_deadline(
            &LlmStreamEvent::ReasonItem {
                provider: "openai".to_string(),
                model: None,
                item_id: "item_3".to_string(),
                encrypted_content: None,
                summary: vec![],
                token_count: Some(1),
            }
        ));
        assert!(stream_event_advances_stall_deadline(
            &LlmStreamEvent::ToolCalls(vec![ToolCall {
                id: "call_1".to_string(),
                name: "demo".to_string(),
                arguments: json!({}),
            }])
        ));
    }

    #[tokio::test]
    async fn test_repair_dangling_tool_calls_no_tool_calls() {
        use crate::events::EventContext;
        use crate::typed_id::SessionId;
        let messages = vec![Message::user("Hello"), Message::assistant("Hi there!")];
        let emitter = crate::traits::NoopEventEmitter;
        let session_id = SessionId::new();
        let ctx = EventContext::empty();
        let patched =
            repair_dangling_tool_calls(&messages, None, &emitter, session_id, &ctx, "turn_01")
                .await;
        assert_eq!(patched.len(), 2);
    }

    #[tokio::test]
    async fn test_repair_dangling_tool_calls_with_result() {
        use crate::events::EventContext;
        use crate::typed_id::SessionId;
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

        let emitter = crate::traits::NoopEventEmitter;
        let session_id = SessionId::new();
        let ctx = EventContext::empty();
        let patched =
            repair_dangling_tool_calls(&messages, None, &emitter, session_id, &ctx, "turn_01")
                .await;
        assert_eq!(patched.len(), 3);
    }

    #[tokio::test]
    async fn test_repair_dangling_tool_calls_missing_result_no_store() {
        use crate::events::EventContext;
        use crate::typed_id::SessionId;
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

        let emitter = crate::traits::NoopEventEmitter;
        let session_id = SessionId::new();
        let ctx = EventContext::empty();
        let patched =
            repair_dangling_tool_calls(&messages, None, &emitter, session_id, &ctx, "turn_01")
                .await;
        // Should have added a cancelled result
        assert_eq!(patched.len(), 4);
        assert_eq!(patched[2].role, MessageRole::ToolResult);
        assert_eq!(patched[2].tool_call_id(), Some("call_456"));
    }

    #[tokio::test]
    async fn test_repair_dangling_tool_calls_settled_result_replayed() {
        use crate::events::EventContext;
        use crate::traits::{DurableToolCallStatus, DurableToolResultStore, ToolCallClaimResult};
        use crate::typed_id::SessionId;

        struct MockSettledStore;
        #[async_trait::async_trait]
        impl DurableToolResultStore for MockSettledStore {
            async fn try_claim_tool_call(
                &self,
                _: &str,
                _: &str,
                _: &str,
                _: &str,
            ) -> crate::error::Result<ToolCallClaimResult> {
                Ok(ToolCallClaimResult::Claimed {
                    claim_token: uuid::Uuid::new_v4(),
                })
            }
            async fn settle_tool_call(
                &self,
                _: &str,
                _: &str,
                _: serde_json::Value,
                _: &str,
                _: uuid::Uuid,
            ) -> crate::error::Result<bool> {
                Ok(true)
            }
            async fn get_tool_call_status(
                &self,
                _turn_id: &str,
                _tool_call_id: &str,
            ) -> crate::error::Result<Option<DurableToolCallStatus>> {
                Ok(Some(DurableToolCallStatus::Settled {
                    result_json: serde_json::json!({
                        "tool_call_id": "call_789",
                        "result": {"answer": 42},
                        "error": null,
                        "images": null,
                        "connection_required": null,
                        "raw_output": null
                    }),
                }))
            }
        }

        let tool_call = ToolCall {
            id: "call_789".to_string(),
            name: "compute".to_string(),
            arguments: serde_json::json!({"x": 21}),
        };
        let messages = vec![
            Message::user("Compute"),
            Message::assistant_with_tools("Computing...", vec![tool_call]),
        ];

        let store = MockSettledStore;
        let emitter = crate::traits::NoopEventEmitter;
        let session_id = SessionId::new();
        let ctx = EventContext::empty();
        let patched = repair_dangling_tool_calls(
            &messages,
            Some(&store as &dyn DurableToolResultStore),
            &emitter,
            session_id,
            &ctx,
            "turn_01",
        )
        .await;

        // Settled result should be replayed (not cancelled message)
        assert_eq!(patched.len(), 3);
        assert_eq!(patched[2].role, MessageRole::ToolResult);
        assert_eq!(patched[2].tool_call_id(), Some("call_789"));
    }

    #[tokio::test]
    async fn test_repair_dangling_tool_calls_interrupted_result_replayed() {
        use crate::events::EventContext;
        use crate::traits::{DurableToolCallStatus, DurableToolResultStore, ToolCallClaimResult};
        use crate::typed_id::SessionId;

        struct MockInterruptedStore;
        #[async_trait::async_trait]
        impl DurableToolResultStore for MockInterruptedStore {
            async fn try_claim_tool_call(
                &self,
                _: &str,
                _: &str,
                _: &str,
                _: &str,
            ) -> crate::error::Result<ToolCallClaimResult> {
                Ok(ToolCallClaimResult::Claimed {
                    claim_token: uuid::Uuid::new_v4(),
                })
            }
            async fn settle_tool_call(
                &self,
                _: &str,
                _: &str,
                _: serde_json::Value,
                _: &str,
                _: uuid::Uuid,
            ) -> crate::error::Result<bool> {
                Ok(true)
            }
            async fn get_tool_call_status(
                &self,
                _turn_id: &str,
                _tool_call_id: &str,
            ) -> crate::error::Result<Option<DurableToolCallStatus>> {
                Ok(Some(DurableToolCallStatus::Interrupted {
                    result_json: None,
                }))
            }
        }

        let tool_call = ToolCall {
            id: "call_int".to_string(),
            name: "slow_op".to_string(),
            arguments: serde_json::json!({}),
        };
        let messages = vec![
            Message::user("Do it"),
            Message::assistant_with_tools("Doing...", vec![tool_call]),
        ];

        let store = MockInterruptedStore;
        let emitter = crate::traits::NoopEventEmitter;
        let session_id = SessionId::new();
        let ctx = EventContext::empty();
        let patched = repair_dangling_tool_calls(
            &messages,
            Some(&store as &dyn DurableToolResultStore),
            &emitter,
            session_id,
            &ctx,
            "turn_01",
        )
        .await;

        assert_eq!(patched.len(), 3);
        let repair = &patched[2];
        assert_eq!(repair.role, MessageRole::ToolResult);
        assert_eq!(repair.tool_call_id(), Some("call_int"));
        // Interrupted replay uses the stored error or fallback text; must contain "interrupted"
        let content = format!("{:?}", repair);
        assert!(
            content.contains("interrupted") || content.contains("not complete"),
            "expected interrupted message, got: {content}"
        );
    }

    #[tokio::test]
    async fn test_repair_dangling_tool_calls_running_synthesized() {
        use crate::events::EventContext;
        use crate::traits::{DurableToolCallStatus, DurableToolResultStore, ToolCallClaimResult};
        use crate::typed_id::SessionId;

        struct MockRunningStore;
        #[async_trait::async_trait]
        impl DurableToolResultStore for MockRunningStore {
            async fn try_claim_tool_call(
                &self,
                _: &str,
                _: &str,
                _: &str,
                _: &str,
            ) -> crate::error::Result<ToolCallClaimResult> {
                Ok(ToolCallClaimResult::Claimed {
                    claim_token: uuid::Uuid::new_v4(),
                })
            }
            async fn settle_tool_call(
                &self,
                _: &str,
                _: &str,
                _: serde_json::Value,
                _: &str,
                _: uuid::Uuid,
            ) -> crate::error::Result<bool> {
                Ok(true)
            }
            async fn get_tool_call_status(
                &self,
                _turn_id: &str,
                _tool_call_id: &str,
            ) -> crate::error::Result<Option<DurableToolCallStatus>> {
                Ok(Some(DurableToolCallStatus::Running))
            }
        }

        let tool_call = ToolCall {
            id: "call_run".to_string(),
            name: "long_job".to_string(),
            arguments: serde_json::json!({}),
        };
        let messages = vec![
            Message::user("Start job"),
            Message::assistant_with_tools("Starting...", vec![tool_call]),
        ];

        let store = MockRunningStore;
        let emitter = crate::traits::NoopEventEmitter;
        let session_id = SessionId::new();
        let ctx = EventContext::empty();
        let patched = repair_dangling_tool_calls(
            &messages,
            Some(&store as &dyn DurableToolResultStore),
            &emitter,
            session_id,
            &ctx,
            "turn_01",
        )
        .await;

        assert_eq!(patched.len(), 3);
        let repair = &patched[2];
        assert_eq!(repair.role, MessageRole::ToolResult);
        assert_eq!(repair.tool_call_id(), Some("call_run"));
        // Running stale claim must warn "uncertain; do not retry automatically"
        let content = format!("{:?}", repair);
        assert!(
            content.contains("uncertain") || content.contains("do not retry"),
            "expected uncertain/do-not-retry message, got: {content}"
        );
    }

    #[tokio::test]
    async fn test_repair_dangling_tool_calls_store_error_unknown() {
        use crate::error::AgentLoopError;
        use crate::events::EventContext;
        use crate::traits::{DurableToolResultStore, ToolCallClaimResult};
        use crate::typed_id::SessionId;

        struct MockErrorStore;
        #[async_trait::async_trait]
        impl DurableToolResultStore for MockErrorStore {
            async fn try_claim_tool_call(
                &self,
                _: &str,
                _: &str,
                _: &str,
                _: &str,
            ) -> crate::error::Result<ToolCallClaimResult> {
                Ok(ToolCallClaimResult::Claimed {
                    claim_token: uuid::Uuid::new_v4(),
                })
            }
            async fn settle_tool_call(
                &self,
                _: &str,
                _: &str,
                _: serde_json::Value,
                _: &str,
                _: uuid::Uuid,
            ) -> crate::error::Result<bool> {
                Ok(true)
            }
            async fn get_tool_call_status(
                &self,
                _turn_id: &str,
                _tool_call_id: &str,
            ) -> crate::error::Result<Option<crate::traits::DurableToolCallStatus>> {
                Err(AgentLoopError::tool("simulated store failure"))
            }
        }

        let tool_call = ToolCall {
            id: "call_err".to_string(),
            name: "risky_op".to_string(),
            arguments: serde_json::json!({}),
        };
        let messages = vec![
            Message::user("Do risky op"),
            Message::assistant_with_tools("On it...", vec![tool_call]),
        ];

        let store = MockErrorStore;
        let emitter = crate::traits::NoopEventEmitter;
        let session_id = SessionId::new();
        let ctx = EventContext::empty();
        let patched = repair_dangling_tool_calls(
            &messages,
            Some(&store as &dyn DurableToolResultStore),
            &emitter,
            session_id,
            &ctx,
            "turn_01",
        )
        .await;

        assert_eq!(patched.len(), 3);
        let repair = &patched[2];
        assert_eq!(repair.role, MessageRole::ToolResult);
        assert_eq!(repair.tool_call_id(), Some("call_err"));
        // Store error must NOT say "safe to retry"
        let content = format!("{:?}", repair);
        assert!(
            !content.contains("safe to retry"),
            "store error must not say 'safe to retry', got: {content}"
        );
        assert!(
            content.contains("do not retry") || content.contains("status unknown"),
            "expected do-not-retry/status-unknown message, got: {content}"
        );
    }

    #[test]
    fn test_build_request_options_for_openai_prompt_cache() {
        let config = LlmCallConfig {
            model: "gpt-5.4".to_string(),
            temperature: None,
            max_tokens: None,
            tools: vec![],
            reasoning_effort: None,
            metadata: HashMap::new(),
            previous_response_id: Some("resp_123".to_string()),
            tool_search: None,
            prompt_cache: Some(PromptCacheConfig {
                enabled: true,
                strategy: PromptCacheStrategy::Auto,
                gemini_cached_content: None,
            }),
            openrouter_routing: None,
        };

        let request_options = build_request_options(&config, "openai").unwrap();
        assert_eq!(
            request_options
                .prompt_cache
                .and_then(|info| info.provider_mode),
            Some("prompt_cache_key".to_string())
        );
        assert_eq!(
            request_options.provider_options.get("openai"),
            Some(&json!({ "previous_response_id": true }))
        );
    }

    #[test]
    fn test_build_request_options_for_gemini_explicit_cache() {
        let config = LlmCallConfig {
            model: "gemini-2.5-pro".to_string(),
            temperature: None,
            max_tokens: None,
            tools: vec![],
            reasoning_effort: None,
            metadata: HashMap::new(),
            previous_response_id: None,
            tool_search: None,
            prompt_cache: Some(PromptCacheConfig {
                enabled: true,
                strategy: PromptCacheStrategy::Auto,
                gemini_cached_content: Some("cachedContents/demo-cache".to_string()),
            }),
            openrouter_routing: None,
        };

        let request_options = build_request_options(&config, "gemini").unwrap();
        assert_eq!(
            request_options
                .prompt_cache
                .and_then(|info| info.provider_mode),
            Some("cached_content".to_string())
        );
        assert_eq!(
            request_options.provider_options.get("gemini"),
            Some(&json!({ "cached_content": true }))
        );
    }

    #[test]
    fn test_build_request_options_omits_gemini_cache_flag_when_disabled() {
        let config = LlmCallConfig {
            model: "gemini-2.5-pro".to_string(),
            temperature: None,
            max_tokens: None,
            tools: vec![],
            reasoning_effort: None,
            metadata: HashMap::new(),
            previous_response_id: None,
            tool_search: None,
            prompt_cache: Some(PromptCacheConfig {
                enabled: false,
                strategy: PromptCacheStrategy::Auto,
                gemini_cached_content: Some("cachedContents/demo-cache".to_string()),
            }),
            openrouter_routing: None,
        };

        assert!(build_request_options(&config, "gemini").is_none());
    }

    #[test]
    fn system_keys_override_embedder_keys_in_metadata() {
        // Pins the injection order: embedder keys inserted first, then system
        // keys overwrite any collision. A harness author cannot shadow session_id,
        // turn_id, etc. by including them in embedder_metadata.
        let embedder_metadata: HashMap<String, String> = [
            ("session_id".to_string(), "attacker_value".to_string()),
            ("custom_key".to_string(), "custom_value".to_string()),
        ]
        .into();

        let mut metadata: HashMap<String, String> = HashMap::new();

        // Inject embedder metadata first (mirrors execute_single_turn logic)
        for (k, v) in &embedder_metadata {
            metadata.insert(k.clone(), v.clone());
        }

        // System key injected second — must overwrite
        metadata.insert("session_id".to_string(), "real_session_id".to_string());

        assert_eq!(
            metadata.get("session_id").map(String::as_str),
            Some("real_session_id"),
            "system key must overwrite embedder key with same name"
        );
        assert_eq!(
            metadata.get("custom_key").map(String::as_str),
            Some("custom_value"),
            "non-colliding embedder key must be preserved"
        );
    }

    // =========================================================================
    // ContinuePartial recovery tests (EVE-532)
    // =========================================================================

    use crate::traits::{NoopPartialStreamStore, PartialStreamState, PartialStreamStore};

    struct MockPartialStore(Option<String>);

    #[async_trait::async_trait]
    impl PartialStreamStore for MockPartialStore {
        async fn get_partial_stream(
            &self,
            _session_id: crate::typed_id::SessionId,
            _turn_id: &str,
        ) -> crate::error::Result<Option<PartialStreamState>> {
            Ok(self.0.as_deref().map(|s| PartialStreamState {
                accumulated: s.to_string(),
            }))
        }
    }

    #[tokio::test]
    async fn test_noop_partial_stream_store_returns_none() {
        let store = NoopPartialStreamStore;
        let result = store
            .get_partial_stream(crate::typed_id::SessionId::new(), "turn_01")
            .await
            .unwrap();
        assert!(result.is_none());
    }

    #[tokio::test]
    async fn test_partial_stream_store_returns_accumulated_when_partial_exists() {
        let store = MockPartialStore(Some("partial text so far".to_string()));
        let result = store
            .get_partial_stream(crate::typed_id::SessionId::new(), "turn_01")
            .await
            .unwrap();
        assert_eq!(result.unwrap().accumulated, "partial text so far");
    }

    #[tokio::test]
    async fn test_partial_stream_store_returns_empty_when_started_no_delta() {
        let store = MockPartialStore(Some(String::new()));
        let result = store
            .get_partial_stream(crate::typed_id::SessionId::new(), "turn_01")
            .await
            .unwrap();
        assert!(result.unwrap().accumulated.is_empty());
    }
}
