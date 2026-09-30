//! Stream events and completion metadata every chat driver produces.
//!
//! Split out of `driver_registry` (file-size ratchet); re-exported there, so
//! `driver_registry::LlmStreamEvent` and friends keep working.

pub use crate::stream_error::LlmStreamError;

use crate::provider_managed::ProviderCheckpointCandidate;
use crate::tool_types::ToolCall;

/// Events emitted during LLM streaming
///
/// `#[non_exhaustive]`: new event kinds arrive with each provider capability
/// (PDF input, native async tool calls), and a new variant must not break
/// every consumer's `match`. Consumers ignore what they do not recognize.
#[doc = include_str!("stream_contract.md")]
#[derive(Debug, Clone)]
#[non_exhaustive]
pub enum LlmStreamEvent {
    /// Text delta (incremental content)
    TextDelta(String),
    /// Incremental readable reasoning for the reasoning block currently open.
    ///
    /// Always belongs to the reasoning channel, never the assistant-text
    /// channel. `summary` marks provider-curated summary text (OpenAI
    /// Responses) as opposed to raw chain-of-thought, so consumers can label
    /// what they are showing instead of guessing.
    ReasoningDelta { delta: String, summary: bool },
    /// A reasoning block completed.
    ///
    /// Carries the whole artifact — readable text plus the opaque id,
    /// signature and encrypted payload needed to replay it verbatim. One event
    /// per block, in emission order, so interleaved thinking survives.
    ReasoningItem(crate::reasoning::ReasoningContentPart),
    /// Tool calls from the LLM
    ToolCalls(Vec<ToolCall>),
    /// Complete native async/custom call; requires a native-call coordinator.
    NativeToolCall(crate::native_async::NativeToolCall),
    /// Provider-native execution phase for the current assistant message,
    /// surfaced mid-stream before completion (EVE-774).
    ///
    /// Only emitted by providers whose stream carries a native phase ahead of
    /// the terminal `Done` metadata (OpenAI Responses exposes it on
    /// `response.output_item.added`). Consumers use it as a best-effort hint to
    /// classify streamed assistant text as commentary vs final answer; the
    /// authoritative value is still the completed `Message.phase`. Other
    /// providers never emit this and stay unclassified until completion.
    MessagePhase(crate::execution_phase::ExecutionPhase),
    /// A provider-managed compaction block started. The event deliberately
    /// carries no provider content.
    ProviderCompactionStarted,
    /// A provider-executed (hosted) tool call changed state, e.g. OpenAI's
    /// `web_search`. Informational only: the provider runs it inside this
    /// response, so the agent loop must never dispatch it.
    HostedToolCall(HostedToolCall),
    /// Streaming completed
    Done(Box<LlmCompletionMetadata>),
    /// Error during streaming
    Error(LlmStreamError),
}

/// Metadata about LLM completion
/// Contains token usage and completion information from the LLM response.
///
/// Token buckets are **disjoint** by convention (see the `TokenUsage` event): drivers normalize
/// provider wire formats at the boundary so `prompt_tokens` carries only non-cached input, with
/// `cache_read_tokens` / `cache_creation_tokens` additive on top. Inclusive providers (OpenAI
/// Responses / Chat Completions, Gemini) subtract their cached count from the reported prompt
/// total via [`disjoint_prompt_tokens`]; Anthropic / Bedrock already report disjoint buckets and
/// pass values through unchanged.
///
/// `#[non_exhaustive]`: usage and cost dimensions keep being added, so
/// construct with [`LlmCompletionMetadata::default`] and assign fields.
#[derive(Debug, Clone, Default)]
#[non_exhaustive]
pub struct LlmCompletionMetadata {
    /// Total tokens used (non-cached prompt + cache read/creation + completion)
    pub total_tokens: Option<u32>,
    /// Non-cached prompt tokens (cached reads are excluded; see struct docs)
    pub prompt_tokens: Option<u32>,
    /// Completion tokens
    pub completion_tokens: Option<u32>,
    /// Tokens read from cache (reduces cost), disjoint from `prompt_tokens`
    pub cache_read_tokens: Option<u32>,
    /// Tokens written to cache (Anthropic-specific), disjoint from `prompt_tokens`
    pub cache_creation_tokens: Option<u32>,
    /// Reasoning tokens the provider billed as part of `completion_tokens`.
    ///
    /// A *subset* of `completion_tokens`, not an addition to it: providers
    /// that report reasoning separately (OpenAI
    /// `completion_tokens_details.reasoning_tokens`) still count it in the
    /// completion total. Reported so callers can attribute the spend rather
    /// than infer it from the visible answer's length.
    pub reasoning_tokens: Option<u32>,
    /// Authoritative cost of this generation in USD, when the provider reports
    /// it inline (e.g. OpenRouter's `usage.cost`). `None` for providers that do
    /// not return a cost.
    pub provider_cost_usd: Option<f64>,
    /// Service tier the provider reports serving (OpenAI `service_tier`); prices the call.
    pub service_tier: Option<String>,
    /// Configured model used for the request.
    pub model: Option<String>,
    /// Model the provider reported serving, when its response includes one.
    pub response_model: Option<String>,
    /// Finish reason as reported, normalized; `None` if the provider sent none.
    pub finish_reason: Option<String>,
    /// Retry metadata (present if rate limit retries occurred)
    pub retry_metadata: Option<crate::llm_retry::RetryMetadata>,
    /// Provider's response ID (e.g., OpenAI response ID from response.completed).
    /// Used for `previous_response_id` chaining and OTel tracing.
    pub response_id: Option<String>,
    /// Execution phase from the provider's response (e.g., "commentary", "final_answer").
    /// When present, this value should be preserved on the assistant message and sent
    /// back as-is in subsequent requests. Only set by providers with native phase support.
    pub phase: Option<String>,
    /// The request body the driver sent, when
    /// [`crate::driver_registry::LlmCallConfig::capture_request`] asked for it.
    ///
    /// Verbatim, as serialized for the wire. `None` when the capture was not
    /// requested, or on a driver that does not support it.
    pub request_body: Option<serde_json::Value>,
    /// Provider-reported prompt-cache diagnostics, verbatim.
    ///
    /// Present only when the request opted in via
    /// [`crate::driver_registry::LlmCallConfig::cache_diagnostics`] and the provider answered with a
    /// diagnostics payload (today: Anthropic's `cache-diagnosis` beta). The
    /// shape is provider-owned, so the runtime carries it without interpreting
    /// it.
    pub cache_diagnostics: Option<serde_json::Value>,
    /// Provider-native assistant content for lossless replay on the next call.
    ///
    /// This is internal transcript state. Public message projections remove it.
    pub provider_opaque_content: Option<crate::message::ProviderOpaqueContent>,
    /// Provider-owned replay checkpoint. The engine installs it only after the
    /// completed assistant output is durable.
    pub provider_checkpoint_candidate: Option<ProviderCheckpointCandidate>,
    /// Provider-executed tool calls in this response, keyed by kind
    /// (`web_search_call`, ...). Hosted tools bill per call on top of tokens.
    pub hosted_tool_calls: std::collections::BTreeMap<String, u32>,
}

/// State of one provider-executed tool call. See [`LlmStreamEvent::HostedToolCall`].
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct HostedToolCall {
    /// Provider item id; stable across the call's state changes.
    pub id: String,
    /// Tool name as configured, e.g. `web_search`.
    pub tool: String,
    pub status: HostedToolCallStatus,
    /// Short human-readable detail once known, e.g. the search query.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub summary: Option<String>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum HostedToolCallStatus {
    InProgress,
    Completed,
    Failed,
}

/// Normalize an inclusive provider's reported prompt-token count to the disjoint
/// `TokenUsage` convention by subtracting the cached-read subset.
///
/// OpenAI (Responses & Chat Completions) and Gemini report a prompt token count
/// that *includes* cached reads; callers pass that raw count plus the provider's
/// cached-read count to get the non-cached remainder. Saturating subtraction
/// guards against a provider reporting `cache_read > reported_input`. Anthropic /
/// Bedrock already report disjoint buckets and must not call this.
///
pub fn disjoint_prompt_tokens(reported_input: u32, cache_read: Option<u32>) -> u32 {
    reported_input.saturating_sub(cache_read.unwrap_or(0))
}
