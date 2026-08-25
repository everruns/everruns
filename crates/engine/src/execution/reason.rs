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

use futures::StreamExt;
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::sync::Arc;
use std::time::Instant;
use uuid::Uuid;

use super::ExecutionContext;
use crate::annotation_hook::{collect_annotations, verify_annotations};
use crate::capabilities::CapabilityRegistry;
use crate::driver_registry::{LlmMessage, LlmMessageContent, LlmMessageRole, LlmStreamEvent};
use crate::error::{AgentLoopError, Result};
use crate::events::{
    CapabilityUsageData, EventContext, EventRequest, LlmCompactionInfo, LlmGenerationData,
    LlmRetryInfo, OutputMessageCompletedData, OutputMessageDeltaData, OutputMessageReplacedData,
    OutputMessageStartedData, ReasonCompletedData, ReasonItemData, ReasonRecoveredData,
    ReasonStartedData, ReasonThinkingCompletedData, ReasonThinkingDeltaData,
    ReasonThinkingStartedData, RecoveryMode, TokenUsage, ToolDefinitionSummary,
};
use crate::llm_retry::{
    LlmRetryConfig, RetryMetadata, is_transient_error_message, remaining_retry_time,
    reserve_retry_wait,
};
use crate::message::{ContentPart, Message, MessageRole};
use everruns_provider::reasoning::{ReasoningContentPart, ReasoningText};
use crate::message_retriever::MessageRetriever;
use crate::output_guardrail::{
    ArmedGuardrail, OutputGuardrailContext, PostGenerationOutputContext, evaluate_guardrails,
    evaluate_post_generation_guardrails, post_generation_guardrail_text,
};
use crate::phase_effects::{PhaseEffectEmitter, PhaseEffectSink};
use crate::runtime_context::{AssembledTurnContext, TurnContextRequest, TurnContextResolver};
use crate::tool_types::{ToolCall, ToolDefinition};
use crate::typed_id::{AgentId, HarnessId, MessageId, SessionId};
use crate::{ErrorDisclosure, UserFacingError, UserFacingErrorContext};
use crate::{
    durability::DurableToolResultStore, durability::PartialStreamState,
    durability::PartialStreamStore, event_emitter::EventEmitter, image_services::ImageResolver,
    image_services::ResolvedImage,
};

mod compaction;
mod error_policy;
mod observability;
mod output_hooks;
mod request_controls;
mod stream_state;
mod transcript;

use compaction::{
    ProactiveCompactionContext, ReactiveCompactionContext, apply_proactive_compaction,
    apply_reactive_compaction,
};
use error_policy::{
    error_disclosure_override, filter_response_text, is_error_placeholder_message,
    resolve_error_disclosure,
};
use observability::{build_request_options, capability_usage_snapshot_records};
use output_hooks::collect_output_hooks;
use request_controls::resolve_request_controls;
use stream_state::{
    StreamReplayState, StreamTermination, advances_stall_deadline, append_guarded_thinking_delta,
    merge_retry_metadata,
};
use transcript::repair_dangling_tool_calls;

// ============================================================================
// Helper Functions
// ============================================================================

/// Apply capability-owned transforms to the finalized model tool-call batch.
/// The reason atom owns timing and context; each implementation owns its policy.
#[allow(clippy::too_many_arguments)]
async fn apply_finalized_tool_calls_hooks(
    capability_registry: &CapabilityRegistry,
    event_emitter: &dyn EventEmitter,
    session_id: SessionId,
    context: &ExecutionContext,
    resolved_capability_configs: &[crate::CapabilityRef],
    tool_definitions: &[ToolDefinition],
    tool_calls: &mut [ToolCall],
    iteration: u32,
) {
    let hook_context = crate::finalized_tool_calls::FinalizedToolCallsContext {
        event_emitter,
        session_id,
        execution_context: context,
        tool_definitions,
        iteration,
    };
    for config in resolved_capability_configs {
        let Some(capability) = capability_registry.get(config.capability_id()) else {
            continue;
        };
        if let Some(hook) = capability.finalized_tool_calls_hook(config.config_value()) {
            hook.apply(&hook_context, tool_calls).await;
        }
    }
}

fn unix_now_secs() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs()
}

/// Input for ReasonAtom
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ReasonInput {
    /// Atom execution context
    pub context: ExecutionContext,
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
    /// Raw provider finish reason for this generation.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub finish_reason: Option<String>,
    /// Resolved locale used for this turn's prompt and backend-authored strings.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub locale: Option<String>,
    /// Merged network access list for URL filtering in tools.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub network_access: Option<crate::network_access::NetworkAccessList>,
    /// Request-level parallel tool calling preference (EVE-598), carried from
    /// the resolved agent config into `ActInput` so the act scheduler can honor
    /// `Some(false)` (force serialize). `None` preserves the default schedule.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub parallel_tool_calls: Option<bool>,
}

fn default_max_iterations() -> usize {
    500
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
    context_resolver: Arc<dyn TurnContextResolver>,
    message_retriever: Arc<dyn MessageRetriever>,
    capability_registry: CapabilityRegistry,
    event_emitter: PhaseEffectEmitter<dyn PhaseEffectSink>,
    /// Optional image resolver for resolving image_file content parts
    image_resolver: Option<Arc<dyn ImageResolver>>,
    /// Optional heartbeater for stream-liveness signalling (EVE-531).
    stream_heartbeater: Option<Arc<dyn crate::durability::StreamHeartbeater>>,
    /// Optional provider stall timeout (EVE-531). Default: 120s.
    provider_stall_timeout: Option<std::time::Duration>,
    /// Shared attempt/backoff/time budget for automatic provider recovery.
    provider_retry_config: LlmRetryConfig,
    /// Optional durable tool result store for transcript repair (EVE-533).
    durable_tool_result_store: Option<Arc<dyn DurableToolResultStore>>,
    /// Optional partial-stream store for ContinuePartial recovery (EVE-532).
    partial_stream_store: Option<Arc<dyn PartialStreamStore>>,
    /// Optional live reasoning-effort handle (EVE-595). When set and holding a
    /// value, it overrides the message-derived effort on every LLM step, so a
    /// tool can change effort mid-turn and have subsequent steps observe it.
    reasoning_effort_handle: Option<crate::tool_context::ReasoningEffortHandle>,
    /// Optional utility LLM service (EVE-573). Powers model-backed
    /// end-of-message output guardrails (e.g. moderation). When absent, those
    /// guardrails fail open and the seam is a no-op.
    utility_llm_service: Option<Arc<dyn crate::UtilityLlmService>>,
    /// Optional session schedule store. Used by the `usage_limit_auto_continue`
    /// capability to schedule a one-shot continuation after a provider usage
    /// limit resets. When absent, the capability degrades to a no-op (no
    /// continuation is scheduled and the error copy makes no auto-resume
    /// promise).
    schedule_store: Option<Arc<dyn crate::session_services::SessionScheduleStore>>,
    /// Optional durable store for replacement context checkpoints.
    compaction_checkpoint_store: Option<Arc<dyn crate::CompactionCheckpointStore>>,
}

impl ReasonAtom {
    /// Create a new ReasonAtom
    pub fn new(
        context_resolver: impl TurnContextResolver + 'static,
        message_retriever: impl MessageRetriever + 'static,
        capability_registry: CapabilityRegistry,
        event_emitter: impl PhaseEffectSink + 'static,
    ) -> Self {
        Self {
            context_resolver: Arc::new(context_resolver),
            message_retriever: Arc::new(message_retriever),
            capability_registry,
            event_emitter: PhaseEffectEmitter::new(Arc::new(event_emitter)),
            image_resolver: None,
            stream_heartbeater: None,
            provider_stall_timeout: None,
            provider_retry_config: LlmRetryConfig::default(),
            durable_tool_result_store: None,
            partial_stream_store: None,
            reasoning_effort_handle: None,
            utility_llm_service: None,
            schedule_store: None,
            compaction_checkpoint_store: None,
        }
    }

    /// Set the session schedule store used by `usage_limit_auto_continue` to
    /// schedule a continuation after a provider usage limit resets.
    pub fn with_schedule_store(
        mut self,
        store: Arc<dyn crate::session_services::SessionScheduleStore>,
    ) -> Self {
        self.schedule_store = Some(store);
        self
    }

    pub fn with_compaction_checkpoint_store(
        mut self,
        store: Arc<dyn crate::CompactionCheckpointStore>,
    ) -> Self {
        self.compaction_checkpoint_store = Some(store);
        self
    }

    /// Collect the [`LlmErrorHook`]s contributed by the active capabilities,
    /// paired with each capability's per-agent config. Hooks are invoked
    /// generically on the terminal-error path; the reason atom has no knowledge
    /// of any specific capability's behavior. Capabilities that contribute no
    /// hook — the common case — are skipped at zero allocation cost.
    fn collect_llm_error_hooks(
        &self,
        resolved_capability_configs: &[crate::CapabilityRef],
    ) -> Vec<(
        Arc<dyn crate::llm_error_hook::LlmErrorHook>,
        serde_json::Value,
    )> {
        resolved_capability_configs
            .iter()
            .filter_map(|cfg| {
                let cap = self.capability_registry.get(cfg.capability_id())?;
                let hook = cap.llm_error_hook()?;
                Some((hook, cfg.config_value().clone()))
            })
            .collect()
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
        heartbeater: Arc<dyn crate::durability::StreamHeartbeater>,
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

    /// Set the bounded provider recovery policy.
    pub fn with_provider_retry_config(mut self, config: LlmRetryConfig) -> Self {
        self.provider_retry_config = config;
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

    /// Set the live reasoning-effort handle (EVE-595).
    ///
    /// When set and holding a value, the effort it carries overrides the
    /// message-derived effort for every LLM step. Because the handle is shared
    /// and re-read on each step, a tool that mutates it mid-turn causes
    /// subsequent steps in the same turn to use the new effort.
    pub fn with_reasoning_effort_handle(
        mut self,
        handle: crate::tool_context::ReasoningEffortHandle,
    ) -> Self {
        self.reasoning_effort_handle = Some(handle);
        self
    }

    /// Set the utility LLM service used by model-backed end-of-message output
    /// guardrails (EVE-573). When unset, those guardrails fail open.
    pub fn with_utility_llm_service(mut self, service: Arc<dyn crate::UtilityLlmService>) -> Self {
        self.utility_llm_service = Some(service);
        self
    }
}

impl ReasonAtom {
    /// Stable phase name used by logs and durable activity adapters.
    pub fn name(&self) -> &'static str {
        "reason"
    }

    /// Execute a reason phase using host-injected portable contracts.
    pub async fn execute(&self, input: ReasonInput) -> Result<ReasonResult> {
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
        context: &ExecutionContext,
        resolved_capability_configs: &[crate::CapabilityRef],
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
                EventContext::from_execution_context(context),
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

    /// Run configured capability hooks after model tool calls are finalized and
    /// before the assistant message is persisted.
    async fn apply_finalized_tool_call_hooks(
        &self,
        session_id: SessionId,
        context: &ExecutionContext,
        resolved_capability_configs: &[crate::CapabilityRef],
        tool_definitions: &[ToolDefinition],
        tool_calls: &mut [ToolCall],
        iteration: u32,
    ) {
        apply_finalized_tool_calls_hooks(
            &self.capability_registry,
            self.event_emitter.as_ref(),
            session_id,
            context,
            resolved_capability_configs,
            tool_definitions,
            tool_calls,
            iteration,
        )
        .await;
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
        let event_context = EventContext::from_execution_context(&context).with_span(
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
                self.context_resolver
                    .resolve_turn_context(TurnContextRequest {
                        session_id: context.session_id,
                        harness_id,
                        agent_id,
                        mcp_tool_definitions: mcp_tool_definitions.clone(),
                    })
                    .await
            }
        };

        let (error_disclosure, error_context, error_hooks, call_result) = match assembled {
            Ok(assembled) => {
                let error_disclosure = resolve_error_disclosure(
                    &self.capability_registry,
                    &assembled.resolved_capability_configs,
                    error_disclosure_override(&assembled.messages).as_deref(),
                );
                // Collected before `assembled` is consumed by the LLM call so the
                // terminal-error path below can run capability error hooks even
                // though it no longer has the capability configs.
                let error_hooks =
                    self.collect_llm_error_hooks(&assembled.resolved_capability_configs);
                let error_context = UserFacingErrorContext::default()
                    .with_provider(assembled.model.provider_type.to_string())
                    .with_model_id(assembled.model.model.clone());
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
                (error_disclosure, error_context, error_hooks, call_result)
            }
            Err(error) => (
                ErrorDisclosure::default(),
                UserFacingErrorContext::default(),
                Vec::new(),
                Err(error),
            ),
        };

        // Handle LLM call errors gracefully
        let result = match call_result {
            Ok(result) => {
                // Calculate reason phase duration
                let reason_duration_ms = reason_start.elapsed().as_millis() as u64;

                // Emit reason.completed event (same span as reason.started, parent is turn)
                let completed_context = EventContext::from_execution_context(&context).with_span(
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
                let mut source_error = e.user_facing_error(error_context);

                // Only emit user-facing error events for non-transient errors.
                // Transient errors (server errors, rate limits, timeouts) will be
                // retried by the durable task engine. Emitting error events on each
                // retry attempt causes duplicate error messages in the UI.
                // The durable worker emits a single error event when all retries
                // are exhausted (DLQ).
                let is_transient = e.is_transient_llm_error()
                    || (e.llm_error_kind().is_none() && is_transient_error_message(&error_msg));

                // Capability error-hook seam: on the terminal (non-retried)
                // error path, let active capabilities react — perform a side
                // effect and/or augment the user-facing error fields — before the
                // message is built. The atom stays behavior-agnostic; each hook
                // (e.g. `usage_limit_auto_continue`) owns its own logic.
                if !is_transient && !error_hooks.is_empty() {
                    let services = crate::llm_error_hook::LlmErrorHookServices {
                        schedule_store: self.schedule_store.clone(),
                    };
                    for (hook, config) in &error_hooks {
                        let outcome = {
                            let ctx = crate::llm_error_hook::LlmErrorContext {
                                session_id: context.session_id,
                                error_code: &source_error.code,
                                error_fields: &source_error.fields,
                                config,
                                services: &services,
                            };
                            hook.on_llm_error(&ctx).await
                        };
                        for (key, value) in outcome.extra_error_fields {
                            source_error = source_error.with_field(key, value);
                        }
                    }
                }

                let user_error = source_error.apply_disclosure(error_disclosure, Some(&error_msg));
                let user_error_text = user_error.fallback_message();

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
                    let error_msg_context = EventContext::from_execution_context(&context)
                        .with_span(
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
                let completed_context = EventContext::from_execution_context(&context).with_span(
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
                    error: Some(error_msg.clone()),
                    user_facing_error: Some(user_error),
                    error_disclosure: Some(error_disclosure),
                    usage: None,
                    output_message_id,
                    time_to_first_token_ms: None,
                    response_id: None,
                    finish_reason: error_msg
                        .to_ascii_lowercase()
                        .contains("model refused")
                        .then(|| "refusal".to_string()),
                    locale: None,
                    network_access: None,
                    parallel_tool_calls: None,
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
        context: &ExecutionContext,
        trace_id: &str,
        reason_span_id: &str,
        previous_response_id: Option<String>,
        iteration: u32,
        assembled: AssembledTurnContext,
    ) -> Result<ReasonResult> {
        let prior_usage = assembled.cumulative_usage();
        let mut messages = assembled.messages;
        let mut message_source_sequence = assembled.message_source_sequence;
        let model_with_provider = assembled.model;
        let resolved_model_id = assembled.resolved_model_id;
        let resolved_locale = assembled.resolved_locale;
        let compaction_policy = assembled.compaction_policy;
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

        let output_hooks =
            collect_output_hooks(&self.capability_registry, &resolved_capability_configs);
        let guardrail_providers = output_hooks.streaming;
        let post_output_providers = output_hooks.post_generation;
        let annotation_providers = output_hooks.annotations;
        let citation_verifiers = output_hooks.citation_verifiers;

        // 7. Create LLM driver using factory
        let chat_driver = Arc::clone(&model_with_provider.driver);
        let stateful_response_continuation =
            previous_response_id.is_some() && chat_driver.supports_stateful_responses();
        let mut restored_checkpoint: Option<crate::CompactionCheckpoint> = None;
        let mut checkpoint_suffix_message_count = 0usize;

        if compaction_policy.is_some()
            && let Some(store) = self.compaction_checkpoint_store.as_ref()
            && let Some(checkpoint) = store
                .get_latest(
                    session_id,
                    model_with_provider.provider_type.as_str(),
                    &model_with_provider.model,
                )
                .await?
            && checkpoint.is_compatible(
                model_with_provider.provider_type.as_str(),
                &model_with_provider.model,
            )
        {
            let filters = crate::capabilities::collect_message_filters_only(
                &resolved_capability_configs,
                &self.capability_registry,
            );
            let mut query =
                crate::MessageQuery::new(session_id).after_sequence(checkpoint.source_sequence);
            filters.apply_message_filters(&mut query);
            let history = self.message_retriever.load_filtered_history(query).await?;
            messages = history.messages;
            checkpoint_suffix_message_count = messages.len();
            filters.apply_post_load_filters(&mut messages);
            if let crate::CompactionCheckpointPayload::Summary { text } = &checkpoint.payload {
                messages.insert(
                    0,
                    Message::system(format!(
                        "[CONVERSATION_SUMMARY]\n{text}\n[/CONVERSATION_SUMMARY]"
                    )),
                );
            }
            message_source_sequence = history.source_sequence.or(message_source_sequence);
            restored_checkpoint = Some(checkpoint);
        }

        let controls = resolve_request_controls(
            &messages,
            self.reasoning_effort_handle.as_ref(),
            &model_with_provider.provider_type,
            &model_with_provider.model,
        );
        let reasoning_effort = controls.reasoning_effort;
        let speed = controls.speed;
        let verbosity = controls.verbosity;

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
                            partial,
                            iteration,
                            &runtime_agent,
                            &resolved_capability_configs,
                        )
                        .await;
                }
                Ok(Some(_)) => {
                    // Empty accumulated: restart clean — fall through to normal LLM call.
                    // Emit reason.recovered { mode: Restart } for observability.
                    let recovery_ctx = EventContext::from_execution_context(context);
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
        let repair_event_context = EventContext::from_execution_context(context);
        let patched_messages = repair_dangling_tool_calls(
            &messages,
            self.durable_tool_result_store.as_deref(),
            self.event_emitter.as_ref(),
            session_id,
            &repair_event_context,
            &context.turn_id.to_string(),
        )
        .await;
        let raw_tool_result_bytes = compaction_policy
            .as_ref()
            .map(|policy| policy.total_tool_result_bytes(&patched_messages))
            .unwrap_or(0);

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
        let mut context_messages =
            model_view_providers.apply_model_view(patched_messages, &model_view_context);
        context_messages = crate::tool_call_integrity::retain_complete_message_tool_exchanges(
            &context_messages,
            stateful_response_continuation || restored_checkpoint.is_some(),
        );

        // 9c. Append live dynamic facts (e.g. the current time) at the tail.
        // Collected fresh each request so values are current, and delivered as a
        // trailing user-role message so they never fold into the cached system
        // prompt. `volatile_suffix_len` tells the Anthropic driver to anchor its
        // message cache breakpoint *before* this block, so the volatile tail
        // rides uncached while the conversation prefix stays cached.
        let mut volatile_suffix_len = 0usize;
        {
            let facts_ctx = crate::capabilities::FactsContext::new(session_id);
            let dynamic_facts = crate::capabilities::collect_dynamic_facts(
                &resolved_capability_configs,
                &self.capability_registry,
                Some(model_with_provider.model.as_str()),
                &facts_ctx,
            );
            if let Some(block) = crate::capabilities::render_facts_block(&dynamic_facts) {
                context_messages.push(Message::user(block));
                volatile_suffix_len = 1;
            }
        }

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
                reasoning: Vec::new(),
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
            let mut llm_msg =
                crate::llm_conversions::llm_message_from_message_with_images(msg, &resolved_images);
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

        // Context reducers operate on prompt-facing copies and may select only
        // one side of a tool exchange at a window boundary. Stateless requests
        // must be self-contained; stateful Responses requests may retain
        // result-only deltas whose calls live behind `previous_response_id`.
        llm_messages = crate::tool_call_integrity::retain_complete_llm_tool_exchanges_for_request(
            llm_messages,
            stateful_response_continuation || restored_checkpoint.is_some(),
        );

        // 12. Build LLM call config with reasoning effort and metadata
        let mut llm_config_builder =
            crate::llm_conversions::llm_call_config_builder_from_agent(&runtime_agent);
        if let Some(effort) = reasoning_effort.clone() {
            llm_config_builder = llm_config_builder.reasoning_effort(effort);
        }
        if let Some(speed) = speed {
            llm_config_builder = llm_config_builder.speed(speed);
        }
        if let Some(verbosity) = verbosity {
            llm_config_builder = llm_config_builder.verbosity(verbosity);
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

        let mut llm_config = llm_config_builder
            .previous_response_id(previous_response_id.clone())
            .volatile_suffix_len(volatile_suffix_len)
            .build();
        if let Some(checkpoint) = restored_checkpoint.as_ref()
            && let crate::CompactionCheckpointPayload::ProviderOpaque { context } =
                &checkpoint.payload
        {
            llm_config.previous_response_id = None;
            llm_config.provider_opaque_context = Some(context.clone());
        }

        tracing::debug!(
            session_id = %session_id,
            turn_id = %context.turn_id,
            model = %runtime_agent.model,
            message_count = %llm_messages.len(),
            "ReasonAtom: calling LLM"
        );

        // 13. Emit output.message.started event BEFORE starting LLM call
        // This allows UI to show a thinking indicator immediately
        let streaming_event_context = EventContext::from_execution_context(context);

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
                    capability_id: cap_id.clone(),
                    guardrail_id,
                    run,
                });
            }
        }
        // Blocking post-generation guardrails need the full assistant message
        // before they can decide. When active, withhold text deltas until the
        // seam allows the finalized output so blocked tokens are never emitted
        // or persisted as output.message.delta events.
        let buffer_output_deltas = !post_output_providers.is_empty();
        // Allocate the public message id before the first lifecycle event so
        // started/delta/replaced/completed can be grouped without turn-level
        // heuristics. Each reasoning iteration reaches this point separately.
        let output_message_id = MessageId::new();
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
                    message_id: output_message_id,
                    model: Some(runtime_agent.model.clone()),
                    iteration: Some(iteration),
                    // Emitted before the LLM call — phase is not yet known, so the
                    // streamed hint starts `None` (treat as assistant text).
                    phase: None,
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

        if let Some(policy) = compaction_policy.as_deref() {
            compaction_info = apply_proactive_compaction(
                ProactiveCompactionContext {
                    chat_driver: chat_driver.as_ref(),
                    policy,
                    checkpoint_store: self.compaction_checkpoint_store.as_ref(),
                    event_emitter: self.event_emitter.as_ref(),
                    event_context: &streaming_event_context,
                    session_id,
                    message_source_sequence,
                    provider_type: model_with_provider.provider_type.as_str(),
                    model: &model_with_provider.model,
                    system_prompt: has_system_prompt
                        .then_some(runtime_agent.system_prompt.as_str()),
                    stateful_response_continuation,
                    checkpoint_restored: restored_checkpoint.is_some(),
                    checkpoint_suffix_message_count,
                    raw_tool_result_bytes,
                    prior_usage: prior_usage.as_ref(),
                },
                &mut llm_messages_for_call,
                &mut llm_config,
            )
            .await?;
        }

        // 14. Process stream with batched output.message.delta emissions
        // Batch deltas every 100ms to reduce event volume while providing real-time feedback
        const DELTA_BATCH_INTERVAL_MS: u64 = 100;
        let retry_config = self.provider_retry_config.clone();
        let mut stream_retry_metadata = RetryMetadata::default();
        let mut retry_started_at = None;
        // Best-effort streamed phase hint (EVE-774). Starts `None` ("not yet
        // classified — treat as assistant text") and is refined monotonically
        // once a provider reveals a native phase mid-stream. Declared outside the
        // retry loop so it is available to the post-loop guarded delta emission.
        let mut streamed_phase: Option<everruns_provider::ExecutionPhase> = None;
        let (
            text,
            thinking,
            reasoning,
            tool_calls,
            completion_metadata,
            time_to_first_token_ms,
            pending_delta,
            mut tripped,
        ) = 'stream_attempt: loop {
            // Provider id for reasoning artifacts synthesized after the stream.
            // Model name for reasoning events emitted from inside the stream,
            // where `model_with_provider` is no longer borrowable.
            let stream_model_name = llm_config.model.clone();
            let stream_result = if let Some(remaining) =
                remaining_retry_time(&retry_config, retry_started_at)
            {
                match tokio::time::timeout(
                    remaining,
                    chat_driver.chat_completion_stream(
                        &crate::ProviderEndpoint::default(),
                        llm_messages_for_call.clone(),
                        &llm_config,
                    ),
                )
                .await
                {
                    Ok(result) => result,
                    Err(_) => {
                        return Err(AgentLoopError::llm_kind(
                            crate::error::LlmErrorKind::Unavailable,
                            format!(
                                "provider retry time budget exhausted after {} retries over {:.1}s; the turn is safe to resume",
                                stream_retry_metadata.attempts,
                                retry_config.max_retry_elapsed.as_secs_f64()
                            ),
                        )
                        .with_retry_metadata(&stream_retry_metadata));
                    }
                }
            } else {
                chat_driver
                    .chat_completion_stream(
                        &crate::ProviderEndpoint::default(),
                        llm_messages_for_call.clone(),
                        &llm_config,
                    )
                    .await
            };
            let mut stream = match stream_result {
                Ok(stream) => stream,
                Err(e) if e.is_request_too_large() => {
                    let Some(policy) = compaction_policy.as_deref() else {
                        tracing::warn!(
                            session_id = %session_id,
                            turn_id = %context.turn_id,
                            "ReasonAtom: context too large and compaction capability is not enabled"
                        );
                        return Err(e);
                    };
                    let outcome = apply_reactive_compaction(
                        ReactiveCompactionContext {
                            chat_driver: chat_driver.as_ref(),
                            policy,
                            checkpoint_store: self.compaction_checkpoint_store.as_ref(),
                            event_emitter: self.event_emitter.as_ref(),
                            event_context: &streaming_event_context,
                            session_id,
                            message_source_sequence,
                            provider_type: model_with_provider.provider_type.as_str(),
                            model: &model_with_provider.model,
                            summarization_model_fallback: &runtime_agent.model,
                            system_prompt: has_system_prompt
                                .then_some(runtime_agent.system_prompt.as_str()),
                            stateful_response_continuation,
                        },
                        &mut llm_messages_for_call,
                        &mut llm_config,
                    )
                    .await?;
                    let Some(outcome) = outcome else {
                        return Err(e);
                    };
                    if outcome.generation_info.is_some() {
                        compaction_info = outcome.generation_info;
                    }

                    chat_driver
                        .chat_completion_stream(
                            &crate::ProviderEndpoint::default(),
                            llm_messages_for_call.clone(),
                            &llm_config,
                        )
                        .await?
                }
                Err(e)
                    if e.is_transient_llm_error()
                        && !e.llm_retry_handled()
                        && stream_retry_metadata.attempts < retry_config.max_retries =>
                {
                    let proposed_wait =
                        retry_config.calculate_backoff(stream_retry_metadata.attempts);
                    let Some(wait_duration) =
                        reserve_retry_wait(&retry_config, &mut retry_started_at, proposed_wait)
                    else {
                        return Err(AgentLoopError::llm_kind(
                            e.llm_error_kind()
                                .unwrap_or(crate::error::LlmErrorKind::Unavailable),
                            format!(
                                "{e}; automatic recovery time budget exhausted after {} retries; the turn is safe to resume",
                                stream_retry_metadata.attempts
                            ),
                        )
                        .with_retry_metadata(&stream_retry_metadata));
                    };
                    tracing::warn!(
                        session_id = %session_id,
                        turn_id = %context.turn_id,
                        attempt = stream_retry_metadata.attempts + 1,
                        max_retries = retry_config.max_retries,
                        wait_secs = wait_duration.as_secs_f64(),
                        error = %e,
                        "ReasonAtom: transient provider failure before stream, retrying"
                    );
                    stream_retry_metadata.record_retry(wait_duration, None);
                    tokio::time::sleep(wait_duration).await;
                    continue 'stream_attempt;
                }
                Err(e) => return Err(e),
            };

            let mut text = String::new();
            // Reasoning artifacts in emission order. One entry per provider
            // block, each keeping its own signature/id, so interleaved thinking
            // and per-call thought signatures survive replay.
            let mut reasoning: Vec<ReasoningContentPart> = Vec::new();
            // Live-render buffer only; the durable text lives on the artifacts.
            let mut thinking = String::new();
            let mut tool_calls = Vec::new();
            let mut termination = StreamTermination::Exhausted;
            let mut replay_state = StreamReplayState::default();
            let mut pending_delta = String::new();
            let mut pending_thinking_delta = String::new();
            let mut last_delta_emit = Instant::now();
            let mut last_thinking_delta_emit = Instant::now();
            let mut time_to_first_token_ms: Option<u64> = None;

            // EVE-531: stall timeout + keepalive heartbeat for stream-liveness
            let stall_timeout = self
                .provider_stall_timeout
                .unwrap_or(std::time::Duration::from_secs(120));
            let initial_stall_timeout = remaining_retry_time(&retry_config, retry_started_at)
                .map_or(stall_timeout, |remaining| remaining.min(stall_timeout));
            let mut stall_sleep = Box::pin(tokio::time::sleep(initial_stall_timeout));
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
                        // EVE-806: a stream that produced no tokens within the
                        // liveness window is equivalent to a dropped connection.
                        // Route it through the same bounded transient-retry path
                        // as an in-stream provider error (everruns-provider
                        // classifies this message as transient) instead of
                        // failing the turn immediately. Retrying re-issues the
                        // same request with no artificial history messages; a
                        // stall after partial output is not retried, and repeated
                        // stalls stay bounded by retry_config.max_retries.
                        let stall_error =
                            crate::driver_registry::LlmStreamError::new(format!(
                                "provider stream stall: no tokens for {}s",
                                stall_timeout.as_secs()
                            ));
                        tracing::warn!(
                            session_id = %session_id,
                            turn_id = %context.turn_id,
                            stall_secs = stall_timeout.as_secs(),
                            "ReasonAtom: provider stream stall timeout"
                        );
                        if replay_state.should_retry(
                            &stall_error,
                            stream_retry_metadata.attempts,
                            retry_config.max_retries,
                        ) {
                            let proposed_wait = retry_config
                                .calculate_backoff(stream_retry_metadata.attempts);
                            let Some(wait_duration) = reserve_retry_wait(
                                &retry_config,
                                &mut retry_started_at,
                                proposed_wait,
                            ) else {
                                return Err(AgentLoopError::llm_kind(
                                    crate::error::LlmErrorKind::Unavailable,
                                    format!(
                                        "{}; automatic recovery time budget exhausted after {} retries; the turn is safe to resume",
                                        stall_error.message,
                                        stream_retry_metadata.attempts
                                    ),
                                )
                                .with_retry_metadata(&stream_retry_metadata));
                            };
                            tracing::warn!(
                                session_id = %session_id,
                                turn_id = %context.turn_id,
                                attempt = stream_retry_metadata.attempts + 1,
                                max_retries = retry_config.max_retries,
                                wait_secs = wait_duration.as_secs_f64(),
                                "ReasonAtom: provider stream stall, retrying"
                            );
                            stream_retry_metadata.record_retry(wait_duration, None);
                            tokio::time::sleep(wait_duration).await;
                            continue 'stream_attempt;
                        }
                        return Err(AgentLoopError::llm(stall_error.message));
                    },
                    _ = keepalive_ticker.tick() => {
                        if let Some(ref hb) = self.stream_heartbeater {
                            hb.heartbeat(crate::durability::StreamProgress {
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
                replay_state.observe(&event);
                let advanced_stall_deadline = advances_stall_deadline(&event);
                if advanced_stall_deadline {
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
                            termination = StreamTermination::GuardrailBlocked(t);
                            break;
                        }

                        // Emit batched delta if interval elapsed
                        if !buffer_output_deltas
                            && last_delta_emit.elapsed().as_millis() as u64
                                >= DELTA_BATCH_INTERVAL_MS
                            && !pending_delta.is_empty()
                        {
                            if let Err(e) = self
                                .event_emitter
                                .emit(EventRequest::new(
                                    session_id,
                                    streaming_event_context.clone(),
                                    OutputMessageDeltaData {
                                        turn_id: context.turn_id,
                                        message_id: output_message_id,
                                        delta: pending_delta.clone(),
                                        accumulated: text.clone(),
                                        phase: streamed_phase,
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
                    LlmStreamEvent::ReasoningDelta { delta, summary: _ } => {
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
                            termination = StreamTermination::GuardrailBlocked(t);
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
                    LlmStreamEvent::ReasoningItem(item) => {
                        // One durable artifact per provider block, appended in
                        // order. Replay walks these; nothing is collapsed into
                        // a single per-message slot.
                        tracing::debug!(
                            session_id = %session_id,
                            provider = %item.provider,
                            item_id = ?item.item_id,
                            has_signature = item.signature.is_some(),
                            has_encrypted = item.encrypted.is_some(),
                            "ReasonAtom: captured reasoning artifact"
                        );
                        if let Err(e) = self
                            .event_emitter
                            .emit(EventRequest::new(
                                session_id,
                                streaming_event_context.clone(),
                                ReasonItemData {
                                    turn_id: context.turn_id,
                                    provider: item.provider.clone(),
                                    model: Some(stream_model_name.clone()),
                                    // Opaque payloads are replay state, never
                                    // event data: `reason.item` carries
                                    // identity and safe summary only.
                                    item_id: item.item_id.clone().unwrap_or_default(),
                                    summary: item
                                        .display_text()
                                        .filter(|_| !matches!(
                                            item.text,
                                            Some(ReasoningText::Plain { .. })
                                        ))
                                        .into_iter()
                                        .collect(),
                                    token_count: item.tokens,
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
                        reasoning.push(item);
                    }
                    LlmStreamEvent::ToolCalls(calls) => {
                        tool_calls = calls;
                    }
                    LlmStreamEvent::MessagePhase(phase) => {
                        // Provider revealed a native phase for the current
                        // assistant message mid-stream. Refine the streamed hint
                        // monotonically (never flip-flop, never back to None);
                        // subsequent output.message.delta events carry it. This is
                        // a hint only — it is NOT a completion signal and does not
                        // count as stream output. The completed Message.phase stays
                        // authoritative, and the hint is deliberately not derived
                        // from later tool-call presence (EVE-448 anti-pattern).
                        streamed_phase = everruns_provider::ExecutionPhase::refine_streamed_hint(
                            streamed_phase,
                            phase,
                        );
                    }
                    LlmStreamEvent::Done(metadata) => {
                        // Emit any remaining pending delta before completing,
                        // unless a post-generation guardrail must first inspect
                        // the finalized assistant text.
                        if !buffer_output_deltas
                            && !pending_delta.is_empty()
                            && let Err(e) = self
                                .event_emitter
                                .emit(EventRequest::new(
                                    session_id,
                                    streaming_event_context.clone(),
                                    OutputMessageDeltaData {
                                        turn_id: context.turn_id,
                                        message_id: output_message_id,
                                        delta: pending_delta.clone(),
                                        accumulated: text.clone(),
                                        phase: streamed_phase,
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
                        termination = StreamTermination::Completed(metadata);
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
                            termination = StreamTermination::PartialSuccess;
                            break;
                        }

                        if replay_state.should_retry(
                            &err,
                            stream_retry_metadata.attempts,
                            retry_config.max_retries,
                        ) {
                            let proposed_wait =
                                retry_config.calculate_backoff(stream_retry_metadata.attempts);
                            let Some(wait_duration) = reserve_retry_wait(
                                &retry_config,
                                &mut retry_started_at,
                                proposed_wait,
                            ) else {
                                return Err(AgentLoopError::llm_kind(
                                    err.kind(),
                                    format!(
                                        "{err}; automatic recovery time budget exhausted after {} retries; the turn is safe to resume",
                                        stream_retry_metadata.attempts
                                    ),
                                )
                                .with_retry_metadata(&stream_retry_metadata));
                            };
                            tracing::warn!(
                                session_id = %session_id,
                                turn_id = %context.turn_id,
                                attempt = stream_retry_metadata.attempts + 1,
                                max_retries = retry_config.max_retries,
                                wait_secs = wait_duration.as_secs_f64(),
                                error_code = err.code.as_deref().unwrap_or("none"),
                                error_status = err.status,
                                error = %err,
                                "ReasonAtom: transient stream error before output, retrying"
                            );
                            stream_retry_metadata.record_retry(wait_duration, None);
                            tokio::time::sleep(wait_duration).await;
                            continue 'stream_attempt;
                        }

                        // No useful output collected — treat as a real failure.
                        let llm_duration_ms = llm_start.elapsed().as_millis() as u64;
                        let event_context = EventContext::from_execution_context(context)
                            .with_span(
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
                            err.to_string(),
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
                        return Err(AgentLoopError::llm_kind(err.kind(), err.to_string()));
                    }
                }
                // Per-event heartbeat after processing the event, so accumulated_len
                // reflects the just-received tokens. Throttled to every 5s.
                if last_stream_heartbeat.elapsed().as_millis() as u64 >= 5_000
                    && let Some(ref hb) = self.stream_heartbeater
                {
                    hb.heartbeat(crate::durability::StreamProgress {
                        accumulated_len: text.len() + thinking.len(),
                        last_delta_at: last_token_at_unix,
                    })
                    .await;
                    last_stream_heartbeat = Instant::now();
                }
            }
            let (mut completion_metadata, tripped) = termination.into_parts();
            if let Some(metadata) = completion_metadata.as_mut() {
                metadata.retry_metadata =
                    merge_retry_metadata(metadata.retry_metadata.take(), &stream_retry_metadata);
            }

            break 'stream_attempt (
                text,
                thinking,
                reasoning,
                tool_calls,
                completion_metadata,
                time_to_first_token_ms,
                pending_delta,
                tripped,
            );
        };
        let (mut text, mut thinking, mut reasoning, mut tool_calls) =
            (text, thinking, reasoning, tool_calls);

        // End-of-message citation annotation seam (see knowledge/runtime-resources/citations.md). Runs
        // once on the finalized final-answer text to attach claim-level citations
        // before guardrails inspect the complete client-visible payload. Skipped
        // when a guardrail already tripped,
        // when there are no citation providers, when there is no text, or while
        // the message still carries tool calls (citations attach to answer
        // prose, not intermediate tool-calling turns). The built-in feeds do not
        // rewrite text, so streamed deltas stay valid and no buffering is needed;
        // a future feed that rewrites (e.g. to strip inline markers) must also
        // opt into delta buffering.
        let mut citation_annotations: Vec<crate::message::TextAnnotation> = Vec::new();
        if tripped.is_none()
            && !annotation_providers.is_empty()
            && !text.is_empty()
            && tool_calls.is_empty()
        {
            // Apply deterministic capability-owned response filters before
            // annotation offsets are computed against the finalized text.
            text = filter_response_text(
                &self.capability_registry,
                &resolved_capability_configs,
                text,
            );
            let collected = collect_annotations(
                &annotation_providers,
                &runtime_agent.system_prompt,
                &text,
                &messages,
                self.utility_llm_service.as_ref(),
            )
            .await;
            text = collected.text;
            citation_annotations = collected.annotations;

            // Post-generation guardrails must inspect citation metadata as well
            // as prose because annotations are persisted and rendered to clients.
            if !citation_annotations.is_empty() && !post_output_providers.is_empty() {
                let guarded_output = post_generation_guardrail_text(&text, &citation_annotations);
                let ctx = PostGenerationOutputContext {
                    system_prompt: &runtime_agent.system_prompt,
                    message_text: &guarded_output,
                    utility_llm_service: self.utility_llm_service.as_ref(),
                };
                tripped = evaluate_post_generation_guardrails(&post_output_providers, &ctx).await;
            }

            // Verification pass: stamp faithfulness verdicts on the collected
            // citations (no-op when no citation_verification capability is on).
            if tripped.is_none()
                && !citation_annotations.is_empty()
                && !citation_verifiers.is_empty()
            {
                citation_annotations = verify_annotations(
                    &citation_verifiers,
                    &text,
                    self.utility_llm_service.as_ref(),
                    citation_annotations,
                )
                .await;
            }
        }

        // Messages without citation annotations still cross the same
        // post-generation output seam once.
        if tripped.is_none()
            && citation_annotations.is_empty()
            && !post_output_providers.is_empty()
            && !text.is_empty()
        {
            let ctx = PostGenerationOutputContext {
                system_prompt: &runtime_agent.system_prompt,
                message_text: &text,
                utility_llm_service: self.utility_llm_service.as_ref(),
            };
            tripped = evaluate_post_generation_guardrails(&post_output_providers, &ctx).await;
        }

        if tripped.is_some() {
            citation_annotations.clear();
        }

        // Release buffered text only after post-generation guardrails allow it.
        // If they block, the replacement path below emits only sanitized text.
        if buffer_output_deltas
            && tripped.is_none()
            && !pending_delta.is_empty()
            && let Err(e) = self
                .event_emitter
                .emit(EventRequest::new(
                    session_id,
                    streaming_event_context.clone(),
                    OutputMessageDeltaData {
                        turn_id: context.turn_id,
                        message_id: output_message_id,
                        delta: pending_delta.clone(),
                        accumulated: text.clone(),
                        phase: streamed_phase,
                    },
                ))
                .await
        {
            tracing::warn!(
                session_id = %session_id,
                error = %e,
                "ReasonAtom: failed to emit guarded output.message.delta event"
            );
        }

        // If a streaming output guardrail tripped, emit
        // output.message.replaced and overwrite the assistant output now so
        // every downstream event (llm.generation, output.message.completed)
        // carries the replacement instead of the model's withheld tokens.
        // The original tokens are never persisted or replayed.
        if let Some(ref t) = tripped {
            let replaced_event_context = EventContext::from_execution_context(context).with_span(
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
                        message_id: output_message_id,
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

        // Finalized tool-call policy seam. This is the smallest provider-neutral
        // point where configured capabilities can normalize a complete call
        // batch before the assistant message and downstream events are built.
        if !tool_calls.is_empty() {
            self.apply_finalized_tool_call_hooks(
                session_id,
                context,
                &resolved_capability_configs,
                &runtime_agent.tools,
                &mut tool_calls,
                iteration,
            )
            .await;
        }

        let llm_duration_ms = llm_start.elapsed().as_millis() as u64;

        // Extract response_id from completion metadata for chaining and OTel
        let response_id = completion_metadata
            .as_ref()
            .and_then(|meta| meta.response_id.clone());
        let finish_reason = completion_metadata
            .as_ref()
            .and_then(|meta| meta.finish_reason.clone());

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
                        meta.cache_read_tokens.unwrap_or(0),
                        meta.cache_creation_tokens.unwrap_or(0),
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
        let event_context = EventContext::from_execution_context(context).with_span(
            trace_id.to_string(),
            Uuid::now_v7().to_string(),
            Some(reason_span_id.to_string()),
        );
        let tools_summary: Vec<ToolDefinitionSummary> =
            runtime_agent.tools.iter().map(|t| t.into()).collect();
        let finish_reasons = Some(vec![finish_reason.clone().unwrap_or_else(|| {
            if tool_calls.is_empty() {
                "stop".to_string()
            } else {
                "tool_calls".to_string()
            }
        })]);
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

        // Add compaction info if compaction was performed. Compaction is a
        // separate billable model call on the same turn, so its cost is folded
        // into the generation's actual cost — `UsageTrackingListener` reads only
        // `metadata.usage`, so that is the only path to budgets and
        // `llm_generations`. `compaction.cost_usd` keeps the split visible
        // (EVE-895).
        if let Some(info) = compaction_info {
            if let Some(compaction_cost) = info.cost_usd {
                match generation_data.metadata.usage.as_mut() {
                    Some(usage) => {
                        usage.actual_cost_usd =
                            Some(usage.actual_cost_usd.unwrap_or(0.0) + compaction_cost);
                    }
                    // The generation itself reported no usage — a provider may
                    // price compaction without returning usage on the retry.
                    // Carry the cost on a usage record of its own rather than
                    // dropping it, which is the failure this fixes.
                    None => {
                        generation_data.metadata.usage = Some(crate::events::TokenUsage {
                            input_tokens: 0,
                            output_tokens: 0,
                            cache_read_tokens: None,
                            cache_creation_tokens: None,
                            actual_cost_usd: Some(compaction_cost),
                            estimated_cost_usd: None,
                            effective_cost_usd: None,
                        });
                    }
                }
            }
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
        // Stamp the provider driver id and provider response id so the chat UI
        // can build a deep link to the provider's trace/logs for this message
        // (see ProviderTraceConfig). The resolved model carries the driver id,
        // not the concrete provider instance id, so the UI keys trace config by
        // driver. `response_id` is the provider's generation id (e.g.
        // OpenRouter's "gen-..."); absent for providers that do not return one.
        metadata.insert(
            "provider".to_string(),
            serde_json::Value::String(model_with_provider.provider_type.to_string()),
        );
        if let Some(ref rid) = response_id {
            metadata.insert(
                "response_id".to_string(),
                serde_json::Value::String(rid.clone()),
            );
        }

        // 18. Store and emit output.message.completed event with metadata and usage.
        // Apply capability-owned response filters before persisting/returning
        // the finalized assistant text.
        let text = filter_response_text(
            &self.capability_registry,
            &resolved_capability_configs,
            text,
        );
        let has_tool_calls = !tool_calls.is_empty();
        let mut assistant_message = if has_tool_calls {
            Message::assistant_with_tools(&text, tool_calls.clone())
        } else {
            Message::assistant(&text)
        }
        .with_id(output_message_id);
        // Attach citation annotations produced by the annotation seam above to
        // the message's text part (see knowledge/runtime-resources/citations.md).
        if !citation_annotations.is_empty() {
            for part in assistant_message.content.iter_mut() {
                if let crate::message::ContentPart::Text(t) = part {
                    t.annotations = std::mem::take(&mut citation_annotations);
                    break;
                }
            }
        }
        // Use the API-provided phase when available (preserving the provider's value),
        // otherwise derive from state: Commentary for intermediate iterations (with tool
        // calls), FinalAnswer for the completed response.
        let provider_type_for_reasoning = model_with_provider.provider_type.to_string();
        // Record where the phase came from. A provider-reported phase is a real
        // classification; a derived one is just `has_tool_calls` wearing a
        // classification's name, and consumers must be able to tell.
        let provider_phase = completion_metadata
            .as_ref()
            .and_then(|meta| meta.phase.as_deref())
            .and_then(everruns_provider::ExecutionPhase::from_provider_str);
        let (phase, phase_source) = match provider_phase {
            Some(phase) => (phase, everruns_provider::PhaseSource::Provider),
            None => (
                everruns_provider::ExecutionPhase::from_has_tool_calls(has_tool_calls),
                everruns_provider::PhaseSource::Derived,
            ),
        };
        assistant_message.phase = Some(phase);
        assistant_message.phase_source = Some(phase_source);
        assistant_message.metadata = Some(metadata);
        // Reasoning artifacts lead the message content, preserving their order
        // among themselves. Every current provider emits reasoning ahead of the
        // text and tool calls it produced, and all three require it replayed in
        // that position, so leading is the faithful placement.
        // Providers that stream reasoning without any replayable artifact
        // (Chat Completions `reasoning_content`) would otherwise render live
        // and vanish on reload. Persist what was shown, with no replay state,
        // so every provider's readable reasoning survives uniformly.
        if reasoning.is_empty() && !thinking.is_empty() {
            reasoning.push(
                ReasoningContentPart::opaque(provider_type_for_reasoning.clone()).with_text(
                    ReasoningText::Plain {
                        text: thinking.clone(),
                    },
                ),
            );
        }
        if !reasoning.is_empty() {
            let mut content = Vec::with_capacity(reasoning.len() + assistant_message.content.len());
            content.extend(reasoning.drain(..).map(ContentPart::Reasoning));
            content.append(&mut assistant_message.content);
            assistant_message.content = content;
        }
        // Emit output.message.completed event (this stores the message as an event with proper turn context)
        // Include token usage for tracking (child of reason span)
        let message_event_context = EventContext::from_execution_context(context).with_span(
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
            finish_reason,
            locale: resolved_locale,
            network_access: runtime_agent.network_access.clone(),
            parallel_tool_calls: runtime_agent.parallel_tool_calls,
        })
    }

    /// Finalize a partial assistant stream without making a new provider call (EVE-532).
    ///
    /// Emits `output.message.started`, `output.message.completed` from the persisted
    /// `accumulated` text, and `reason.recovered { mode: Finalize }`.
    async fn finalize_partial_stream(
        &self,
        session_id: SessionId,
        context: &ExecutionContext,
        partial: PartialStreamState,
        iteration: u32,
        runtime_agent: &crate::RuntimeAgent,
        resolved_capability_configs: &[crate::CapabilityRef],
    ) -> Result<ReasonResult> {
        let event_context = EventContext::from_execution_context(context);
        let turn_id = context.turn_id;
        let message_id = partial.message_id;

        // Signal that output is starting (keeps the streaming protocol intact).
        let _ = self
            .event_emitter
            .emit(EventRequest::new(
                session_id,
                event_context.clone(),
                OutputMessageStartedData {
                    turn_id,
                    message_id,
                    model: None,
                    iteration: Some(iteration),
                    // Recovery/finalize path reconstructs the started signal only;
                    // the streamed phase hint is unavailable here (None).
                    phase: None,
                },
            ))
            .await;

        // Build the assistant message from capability-filtered accumulated text
        // and persist it via the canonical event path.
        let accumulated = filter_response_text(
            &self.capability_registry,
            resolved_capability_configs,
            partial.accumulated,
        );
        let assistant_message = Message::assistant(&accumulated).with_id(message_id);
        let output_message_id = message_id;
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
            tool_definitions: runtime_agent.tools.clone(),
            max_iterations: runtime_agent.max_iterations,
            error: None,
            user_facing_error: None,
            error_disclosure: None,
            usage: None,
            output_message_id: Some(output_message_id),
            time_to_first_token_ms: None,
            response_id: None,
            finish_reason: Some("stop".to_string()),
            locale: None,
            network_access: None,
            // Finalize path has no tool calls, so the preference is irrelevant.
            parallel_tool_calls: None,
        })
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
            .flat_map(crate::llm_conversions::extract_image_file_ids)
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
mod tests;
