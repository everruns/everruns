// Chat Driver Abstractions
//
// This module encapsulates all abstractions needed to interact with LLM Providers:
// - ChatDriver trait and types for provider-agnostic LLM interactions
// - DriverRegistry for dynamic driver registration at startup
// - Message types for LLM calls
//
// Supports both simple text content and multipart content (text, images, audio).
//
// IMPORTANT: API keys must be provided from the database. The registry does NOT read
// from environment variables. Keys should be decrypted and passed via ProviderConfig.
//
// Design: Dependency inversion - provider crates (everruns-anthropic, everruns-openai)
// depend on core and register their drivers at startup. Core has no knowledge of
// specific provider implementations.

use crate::compact::{CompactOutputItem, CompactRequest, CompactResponse};
use crate::credential_schema::CredentialFormSchema;
use crate::error::{AgentLoopError, LlmErrorKind, Result};
use crate::tool_types::{ToolCall, ToolDefinition};
use async_trait::async_trait;
use chrono::{DateTime, Utc};
use futures::Stream;
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::pin::Pin;
use std::sync::Arc;

// ============================================================================
// ChatDriver Trait
// ============================================================================

/// Type alias for the LLM response stream
pub type LlmResponseStream = Pin<Box<dyn Stream<Item = Result<LlmStreamEvent>> + Send>>;

/// Ordered provider-owned context returned by a native compaction operation.
///
/// The runtime carries this value without interpreting or exposing its opaque
/// payload. The matching provider driver is responsible for putting the items
/// back on the wire exactly as returned.
#[derive(Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum ProviderOpaqueContext {
    /// Standalone `output` returned by OpenAI `/responses/compact`.
    OpenResponsesCompact { output: Vec<CompactOutputItem> },
}

impl std::fmt::Debug for ProviderOpaqueContext {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::OpenResponsesCompact { output } => f
                .debug_struct("OpenResponsesCompact")
                .field("item_count", &output.len())
                .finish_non_exhaustive(),
        }
    }
}

/// Structured provider error emitted inside an accepted response stream.
///
/// Providers should preserve the wire error code and HTTP status when they are
/// available. Runtime retry classification uses those fields before falling
/// back to the human-readable message for legacy drivers.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LlmStreamError {
    /// Stable machine-readable provider error code, when supplied.
    pub code: Option<String>,
    /// HTTP status associated with the stream error, when supplied.
    pub status: Option<u16>,
    /// Human-readable diagnostic text.
    pub message: String,
}

impl LlmStreamError {
    pub fn new(message: impl Into<String>) -> Self {
        Self {
            code: None,
            status: None,
            message: message.into(),
        }
    }

    /// Build a stream error while preserving provider-supplied structure.
    pub fn provider(
        code: Option<impl Into<String>>,
        status: Option<u16>,
        message: impl Into<String>,
    ) -> Self {
        Self {
            code: code.map(Into::into),
            status,
            message: message.into(),
        }
    }

    /// Map the preserved structure to Everruns' semantic provider error kind.
    pub fn kind(&self) -> LlmErrorKind {
        if let Some(code) = self.code.as_deref()
            && let Some(kind) = LlmErrorKind::from_provider_code(code)
        {
            return kind;
        }
        if let Some(status) = self.status {
            return LlmErrorKind::from_provider_status(status, &self.message);
        }
        LlmErrorKind::from_error_text(&self.message)
    }
}

impl std::error::Error for LlmStreamError {}

impl std::fmt::Display for LlmStreamError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match (&self.code, self.status) {
            (Some(code), Some(status)) => write!(f, "{code} ({status}): {}", self.message),
            (Some(code), None) => write!(f, "{code}: {}", self.message),
            (None, Some(status)) => write!(f, "({status}): {}", self.message),
            (None, None) => f.write_str(&self.message),
        }
    }
}

impl From<String> for LlmStreamError {
    fn from(message: String) -> Self {
        Self::new(message)
    }
}

impl From<&str> for LlmStreamError {
    fn from(message: &str) -> Self {
        Self::new(message)
    }
}

/// Events emitted during LLM streaming
#[derive(Debug, Clone)]
pub enum LlmStreamEvent {
    /// Text delta (incremental content)
    TextDelta(String),
    /// Thinking delta (incremental reasoning content from extended thinking models)
    ThinkingDelta(String),
    /// Cryptographic signature for thinking content (Anthropic Claude)
    /// Emitted when a thinking block completes, before the Done event
    ThinkingSignature(String),
    /// Opaque assistant reasoning response item (OpenAI Responses).
    /// Carries provider-supplied opaque/encrypted reasoning artifacts plus safe
    /// summary text and per-item metadata. Plaintext hidden reasoning content is
    /// intentionally excluded so callers can persist this without exposing
    /// chain-of-thought.
    ReasonItem {
        /// Provider name (e.g., "openai").
        provider: String,
        /// Model identifier reported by the provider, if known.
        model: Option<String>,
        /// Provider-assigned identifier for the reasoning item.
        item_id: String,
        /// Provider-encrypted reasoning context, if supplied.
        encrypted_content: Option<String>,
        /// Safe summary text segments curated by the provider.
        summary: Vec<String>,
        /// Per-item reasoning token count, when the provider reports one.
        token_count: Option<u32>,
    },
    /// Tool calls from the LLM
    ToolCalls(Vec<ToolCall>),
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
    /// Streaming completed
    Done(Box<LlmCompletionMetadata>),
    /// Error during streaming
    Error(LlmStreamError),
}

/// Model information discovered from a provider's list_models API
///
/// Represents a model available from a provider. Used for dynamic model discovery
/// to sync available models from provider APIs into the database.
///
/// The `discovered_profile` field carries structured capability/limit metadata
/// parsed from the provider's API response (e.g., Anthropic's capabilities object).
/// During model sync, this profile is merged with hardcoded profiles: hardcoded
/// values take precedence (they include cost data not available from APIs),
/// but discovered data fills gaps for models without hardcoded profiles.
#[derive(Debug, Clone)]
pub struct DiscoveredModel {
    /// Model identifier (e.g., "gpt-5.2", "claude-opus-4-5-20251101")
    pub model_id: String,
    /// Human-readable display name (if provided by API)
    pub display_name: Option<String>,
    /// When the model was created/released
    pub created_at: Option<DateTime<Utc>>,
    /// Owner or organization (e.g., "openai", "system")
    pub owned_by: Option<String>,
    /// Service capabilities advertised for this concrete model (for example,
    /// `chat` or `embeddings`). These are distinct from provider-level
    /// services: an OpenAI provider supports both, but each model does not.
    pub capabilities: Vec<String>,
    /// Structured profile built from provider API metadata (capabilities, limits).
    /// Populated by drivers that return rich model metadata (e.g., Anthropic /v1/models).
    pub discovered_profile: Option<crate::model::ModelProfile>,
}

/// Metadata about LLM completion
///
/// Contains token usage and completion information from the LLM response.
///
/// Token buckets are **disjoint** by convention (see the `TokenUsage` event): drivers
/// normalize provider wire formats at the boundary so `prompt_tokens` carries
/// only non-cached input, with `cache_read_tokens` / `cache_creation_tokens`
/// additive on top. Inclusive providers (OpenAI Responses / Chat Completions,
/// Gemini) subtract their cached count from the reported prompt total via
/// [`disjoint_prompt_tokens`]; Anthropic / Bedrock already report disjoint
/// buckets and pass values through unchanged.
///
#[derive(Debug, Clone, Default)]
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
    /// Authoritative cost of this generation in USD, when the provider reports
    /// it inline (e.g. OpenRouter's `usage.cost`). `None` for providers that do
    /// not return a cost.
    pub provider_cost_usd: Option<f64>,
    /// Model used
    pub model: Option<String>,
    /// Finish reason
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

/// Trait for LLM drivers
///
/// Implementations handle provider-specific API calls and response parsing.
///
/// # Error contract
///
/// Drivers surface provider failures as `AgentLoopError` and classify them
/// semantically at the provider boundary, where HTTP status and response body
/// are still available:
///
/// - request-too-large conditions => `AgentLoopError::request_too_large`
/// - missing/unknown model => `AgentLoopError::model_not_available`
/// - everything else => `AgentLoopError::llm_kind(LlmErrorKind::..., msg)`,
///   using `LlmErrorKind::from_provider_status` (HTTP drivers) or
///   `LlmErrorKind::from_error_text` (SDK drivers without a status). Plain
///   `AgentLoopError::llm` is reserved for unclassifiable errors; downstream
///   then falls back to string classification.
///
/// Quota/billing exhaustion (`LlmErrorKind::QuotaExhausted`) is non-transient
/// and must not be retried by driver retry loops even when the provider
/// reports it under a transient status like 429.
#[async_trait]
pub trait ChatDriver: Send + Sync {
    /// Call the LLM with streaming response
    async fn chat_completion_stream(
        &self,
        endpoint: &crate::runtime_provider::ProviderEndpoint,
        messages: Vec<LlmMessage>,
        config: &LlmCallConfig,
    ) -> Result<LlmResponseStream>;

    /// Call the LLM without streaming (convenience method)
    async fn chat_completion(
        &self,
        endpoint: &crate::runtime_provider::ProviderEndpoint,
        messages: Vec<LlmMessage>,
        config: &LlmCallConfig,
    ) -> Result<LlmResponse> {
        use futures::StreamExt;

        let mut stream = self
            .chat_completion_stream(endpoint, messages, config)
            .await?;
        let mut text = String::new();
        let mut thinking = String::new();
        let mut thinking_signature: Option<String> = None;
        let mut tool_calls = Vec::new();
        let mut metadata = LlmCompletionMetadata::default();

        while let Some(event) = stream.next().await {
            match event? {
                LlmStreamEvent::TextDelta(delta) => text.push_str(&delta),
                LlmStreamEvent::ThinkingDelta(delta) => thinking.push_str(&delta),
                LlmStreamEvent::ThinkingSignature(sig) => thinking_signature = Some(sig),
                LlmStreamEvent::ReasonItem {
                    encrypted_content, ..
                } => {
                    if let Some(sig) = encrypted_content {
                        thinking_signature = Some(sig);
                    }
                }
                LlmStreamEvent::ToolCalls(calls) => tool_calls = calls,
                // Streamed phase hint is a mid-stream refinement only; the
                // non-streaming collector relies on the terminal Done metadata.
                LlmStreamEvent::MessagePhase(_) => {}
                LlmStreamEvent::Done(meta) => metadata = *meta,
                LlmStreamEvent::Error(err) => {
                    return Err(crate::error::AgentLoopError::llm_kind(
                        err.kind(),
                        err.to_string(),
                    ));
                }
            }
        }

        Ok(LlmResponse {
            text,
            thinking: if thinking.is_empty() {
                None
            } else {
                Some(thinking)
            },
            thinking_signature,
            tool_calls: if tool_calls.is_empty() {
                None
            } else {
                Some(tool_calls)
            },
            metadata,
        })
    }

    /// List available models from the provider
    ///
    /// Returns `Ok(Some(models))` if the provider supports model listing,
    /// or `Ok(None)` if not supported (e.g., custom endpoints, proxies).
    ///
    /// Implementations should filter to chat/completion models only,
    /// excluding embedding models, TTS, whisper, etc.
    async fn list_models(
        &self,
        _endpoint: &crate::runtime_provider::ProviderEndpoint,
    ) -> Result<Option<Vec<DiscoveredModel>>> {
        // Default: not supported. Providers override if they support listing.
        Ok(None)
    }

    /// Check if this driver supports the compact endpoint
    ///
    /// The compact endpoint compresses conversation history by replacing
    /// assistant messages, tool calls, and tool results with an encrypted
    /// compaction item. User messages are kept verbatim.
    ///
    /// Returns `true` if the driver supports compaction, `false` otherwise.
    /// Currently only supported by OpenAI's Responses API.
    fn supports_compact(&self) -> bool {
        // Default: not supported
        false
    }

    /// Whether this driver persists Responses API state and can resolve tool
    /// calls that are reachable only through `previous_response_id`.
    ///
    /// Stateless and custom drivers default to `false`; they must receive a
    /// self-contained tool call/result transcript on every request.
    fn supports_stateful_responses(&self) -> bool {
        false
    }

    /// Effective context window for `model`, when the driver has authoritative
    /// model metadata that is not represented by Everruns' built-in profiles.
    ///
    /// External drivers should override this so host policy does not guess from
    /// a provider/model table that cannot describe their runtime model aliases.
    fn effective_context_window(&self, _model: &str) -> Option<usize> {
        None
    }

    /// Whether this driver can express the request-level `parallel_tool_calls`
    /// preference on the wire for `model`.
    ///
    /// Drivers that map the preference onto a request field (OpenAI/Anthropic
    /// families) return `true`; drivers whose provider API has no such control
    /// (Gemini, Bedrock) return `false`. When `false`, the preference is omitted
    /// from the request and is honored only by the local tool scheduler, so an
    /// `avoid` preference still serializes tool execution on every provider.
    ///
    /// The default is `false` (conservative: omit unless a driver opts in).
    fn supports_parallel_tool_calls(&self, _model: &str) -> bool {
        false
    }

    /// Compact a conversation to reduce context size
    ///
    /// This method compresses conversation history by calling the provider's
    /// compact endpoint. User messages are kept verbatim, while assistant
    /// messages, tool calls, and tool results are replaced by an encrypted
    /// compaction item that preserves latent context but is opaque.
    ///
    /// # Arguments
    ///
    /// * `request` - The compact request containing the model and input items
    ///
    /// # Returns
    ///
    /// Returns `Ok(Some(response))` if compaction succeeded,
    /// `Ok(None)` if compaction is not supported by this driver,
    /// or `Err` if an error occurred.
    ///
    /// The response contains the compacted output items which can be used
    /// directly as input for the next chat completion call.
    async fn compact(
        &self,
        _endpoint: &crate::runtime_provider::ProviderEndpoint,
        _request: CompactRequest,
    ) -> Result<Option<CompactResponse>> {
        // Default: not supported
        Ok(None)
    }
}

/// Implement ChatDriver for `Box<dyn ChatDriver>` to allow dynamic dispatch
#[async_trait]
impl ChatDriver for Box<dyn ChatDriver> {
    async fn chat_completion_stream(
        &self,
        endpoint: &crate::runtime_provider::ProviderEndpoint,
        messages: Vec<LlmMessage>,
        config: &LlmCallConfig,
    ) -> Result<LlmResponseStream> {
        (**self)
            .chat_completion_stream(endpoint, messages, config)
            .await
    }

    async fn chat_completion(
        &self,
        endpoint: &crate::runtime_provider::ProviderEndpoint,
        messages: Vec<LlmMessage>,
        config: &LlmCallConfig,
    ) -> Result<LlmResponse> {
        (**self).chat_completion(endpoint, messages, config).await
    }

    async fn list_models(
        &self,
        endpoint: &crate::runtime_provider::ProviderEndpoint,
    ) -> Result<Option<Vec<DiscoveredModel>>> {
        (**self).list_models(endpoint).await
    }

    fn supports_compact(&self) -> bool {
        (**self).supports_compact()
    }

    fn supports_stateful_responses(&self) -> bool {
        (**self).supports_stateful_responses()
    }

    fn effective_context_window(&self, model: &str) -> Option<usize> {
        (**self).effective_context_window(model)
    }

    fn supports_parallel_tool_calls(&self, model: &str) -> bool {
        (**self).supports_parallel_tool_calls(model)
    }

    async fn compact(
        &self,
        endpoint: &crate::runtime_provider::ProviderEndpoint,
        request: CompactRequest,
    ) -> Result<Option<CompactResponse>> {
        (**self).compact(endpoint, request).await
    }
}

// ============================================================================
// Message Types
// ============================================================================

/// Message format for LLM calls (provider-agnostic)
#[derive(Debug, Clone)]
pub struct LlmMessage {
    pub role: LlmMessageRole,
    pub content: LlmMessageContent,
    pub tool_calls: Option<Vec<ToolCall>>,
    pub tool_call_id: Option<String>,
    /// Execution phase for assistant messages.
    /// Helps models distinguish between intermediate working commentary (`Commentary`)
    /// and completed answers (`FinalAnswer`) in multi-step tool-calling flows.
    /// Only set on assistant messages. Must be preserved when replaying conversation history.
    pub phase: Option<crate::execution_phase::ExecutionPhase>,
    /// Thinking content from extended thinking models (Anthropic Claude)
    /// Must be included in subsequent API calls when thinking is enabled
    pub thinking: Option<String>,
    /// Cryptographic signature for thinking content (Anthropic Claude)
    /// Required when sending thinking back in subsequent API calls
    pub thinking_signature: Option<String>,
}

impl LlmMessage {
    /// Create a message with text content
    pub fn text(role: LlmMessageRole, content: impl Into<String>) -> Self {
        Self {
            role,
            content: LlmMessageContent::Text(content.into()),
            tool_calls: None,
            tool_call_id: None,
            phase: None,
            thinking: None,
            thinking_signature: None,
        }
    }

    /// Create a message with content parts (text, images, audio)
    pub fn parts(role: LlmMessageRole, parts: Vec<LlmContentPart>) -> Self {
        Self {
            role,
            content: LlmMessageContent::Parts(parts),
            tool_calls: None,
            tool_call_id: None,
            phase: None,
            thinking: None,
            thinking_signature: None,
        }
    }

    /// Get content as plain text string (for simple cases)
    pub fn content_as_text(&self) -> String {
        self.content.to_text()
    }

    /// Prepend a prefix to the first text content.
    ///
    /// Used by ReasonAtom to inject external actor identity (e.g. `"[Alice] "`)
    /// into user messages from external channels.
    pub fn prepend_text_prefix(&mut self, prefix: &str) {
        match &mut self.content {
            LlmMessageContent::Text(text) => {
                *text = format!("{}{}", prefix, text);
            }
            LlmMessageContent::Parts(parts) => {
                for part in parts.iter_mut() {
                    if let LlmContentPart::Text { text } = part {
                        *text = format!("{}{}", prefix, text);
                        return;
                    }
                }
                // No text part found — prepend one
                parts.insert(
                    0,
                    LlmContentPart::Text {
                        text: prefix.to_string(),
                    },
                );
            }
        }
    }
}

/// Fold every `System`-role message into a single string, joined in order with
/// blank lines.
///
/// Multiple system messages legitimately occur in one request: the agent system
/// prompt plus, e.g., `infinity_context`'s hidden-history notice or
/// `compaction`'s `[CONVERSATION_SUMMARY]`. Drivers that map the system role into
/// a dedicated top-level field (Anthropic `system`, Gemini `system_instruction`,
/// OpenResponses `instructions`) must accumulate rather than overwrite — otherwise
/// the real agent system prompt is silently dropped and only the last notice
/// survives. Returns `None` when there are no system messages.
pub fn fold_system_messages(messages: &[LlmMessage]) -> Option<String> {
    let mut system: Option<String> = None;
    for msg in messages {
        if msg.role == LlmMessageRole::System {
            let text = msg.content.to_text();
            system = Some(match system.take() {
                Some(existing) if !existing.is_empty() => format!("{existing}\n\n{text}"),
                _ => text,
            });
        }
    }
    system
}

/// Message content - either a simple string or array of content parts
#[derive(Debug, Clone)]
pub enum LlmMessageContent {
    /// Simple text content
    Text(String),
    /// Array of content parts (text, images, audio)
    Parts(Vec<LlmContentPart>),
}

impl LlmMessageContent {
    /// Convert to plain text (concatenates text parts, ignores media)
    pub fn to_text(&self) -> String {
        match self {
            LlmMessageContent::Text(s) => s.clone(),
            LlmMessageContent::Parts(parts) => parts
                .iter()
                .filter_map(|p| match p {
                    LlmContentPart::Text { text } => Some(text.clone()),
                    _ => None,
                })
                .collect::<Vec<_>>()
                .join(""),
        }
    }

    /// Check if content is simple text
    pub fn is_text(&self) -> bool {
        matches!(self, LlmMessageContent::Text(_))
    }

    /// Check if content has multiple parts
    pub fn is_parts(&self) -> bool {
        matches!(self, LlmMessageContent::Parts(_))
    }
}

impl From<String> for LlmMessageContent {
    fn from(s: String) -> Self {
        LlmMessageContent::Text(s)
    }
}

impl From<&str> for LlmMessageContent {
    fn from(s: &str) -> Self {
        LlmMessageContent::Text(s.to_string())
    }
}

/// A single content part within a message
#[derive(Debug, Clone)]
pub enum LlmContentPart {
    /// Text content
    Text { text: String },
    /// Image content (base64 data URL or HTTP URL)
    Image { url: String },
    /// Audio content (base64 data URL)
    Audio { url: String },
}

impl LlmContentPart {
    /// Create a text content part
    pub fn text(text: impl Into<String>) -> Self {
        LlmContentPart::Text { text: text.into() }
    }

    /// Create an image content part from URL (can be data URL or HTTP URL)
    pub fn image(url: impl Into<String>) -> Self {
        LlmContentPart::Image { url: url.into() }
    }

    /// Create an audio content part from URL (typically a data URL)
    pub fn audio(url: impl Into<String>) -> Self {
        LlmContentPart::Audio { url: url.into() }
    }
}

/// Message role for LLM calls
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum LlmMessageRole {
    System,
    User,
    Assistant,
    Tool,
}

// ============================================================================
// Configuration and Response Types
// ============================================================================

/// Configuration for tool_search (deferred tool loading).
///
/// When enabled, the driver groups tools into namespaces and marks them with
/// `defer_loading: true` so the model only loads full schemas on-demand.
/// This reduces token usage for agents with many tools.
#[derive(Debug, Clone, Default, serde::Serialize, serde::Deserialize)]
pub struct ToolSearchConfig {
    /// Enable tool_search for this request (requires model support)
    pub enabled: bool,
    /// Minimum number of tools before activating tool_search.
    /// Below this threshold, full schemas are sent even when enabled.
    pub threshold: usize,
}

/// Strategy for prompt caching.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[cfg_attr(feature = "openapi", derive(utoipa::ToSchema))]
#[serde(rename_all = "snake_case")]
pub enum PromptCacheStrategy {
    /// Let each driver choose the safest provider-specific behavior.
    #[default]
    Auto,
}

/// Configuration for prompt caching.
///
/// Drivers translate this into provider-specific request options when possible.
/// Unsupported providers or models should ignore it without failing the call.
#[derive(Debug, Clone, Default, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[cfg_attr(feature = "openapi", derive(utoipa::ToSchema))]
pub struct PromptCacheConfig {
    /// Enable prompt caching for this request.
    pub enabled: bool,
    /// Strategy the driver should use when enabling prompt caching.
    #[serde(default)]
    pub strategy: PromptCacheStrategy,
    /// Existing Gemini cached content resource name (`cachedContents/{id}`).
    ///
    /// When set, the Gemini driver uses explicit caching via the
    /// `cachedContent` request field. When absent, Gemini falls back to its
    /// default provider behavior (for example implicit caching on supported
    /// models).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub gemini_cached_content: Option<String>,
}

/// High-level intent presets that compile into OpenRouter provider-routing
/// controls. Presets let callers express quality, cost, privacy, and capability
/// goals without knowing every OpenRouter `provider` flag.
///
/// Multiple presets may be combined. When a preset and an explicit `provider`
/// field target the same control, the explicit field wins. Presets applied
/// earlier in the list may be overridden by later ones for the same field.
///
/// Compilation happens in `OpenRouterRoutingConfig::apply_presets()`.
#[derive(Debug, Clone, PartialEq, serde::Serialize, serde::Deserialize)]
#[cfg_attr(feature = "openapi", derive(utoipa::ToSchema))]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum OpenRouterRoutingPreset {
    /// Prefer the cheapest providers that support function-calling parameters.
    CheapestWithTools,
    /// Prefer the highest-throughput providers for quick review or triage tasks.
    LowestLatencyReview,
    /// Route only to zero-data-retention (ZDR) endpoints.
    ZdrOnly,
    /// Try BYOK-registered providers first; fall back to shared capacity.
    ByokFirst,
    /// Deny all provider-side data collection (logs and training).
    NoDataCollection,
    /// Route only to providers that support strict JSON / structured output.
    StrictJson,
    /// Route only to providers that natively support reasoning/thinking models.
    ReasoningRequired,
    /// Cap per-token provider cost. Values are USD per million tokens; `None`
    /// means no cap on that dimension.
    MaxPrice {
        /// Maximum prompt cost in USD per million tokens.
        #[serde(default, skip_serializing_if = "Option::is_none")]
        prompt_usd_per_million: Option<f64>,
        /// Maximum completion cost in USD per million tokens.
        #[serde(default, skip_serializing_if = "Option::is_none")]
        completion_usd_per_million: Option<f64>,
    },
}

/// OpenRouter model fallback and provider routing controls.
///
/// Organization-level strategy for how OpenRouter should allocate compute capacity.
///
/// Controls whether requests use OpenRouter shared credits, prefer customer-owned
/// upstream keys (BYOK), or require BYOK-only routing. Compiled into OpenRouter
/// `provider` routing controls before dispatch; not sent verbatim on the wire.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[cfg_attr(feature = "openapi", derive(utoipa::ToSchema))]
#[serde(rename_all = "snake_case")]
pub enum OpenRouterCapacityStrategy {
    /// Use OpenRouter shared capacity (credits). No routing changes. Default.
    #[default]
    SharedCapacity,
    /// Prefer providers where the org has registered its own upstream key.
    /// Falls back to shared capacity when BYOK providers are unavailable.
    /// Sets `provider.allow_fallbacks = true` unless the caller overrides it.
    ByokFirst,
    /// Require a provider where the org has its own upstream key.
    /// Routing fails if `provider.only` is not explicitly configured with at
    /// least one BYOK provider slug.
    /// Sets `provider.allow_fallbacks = false`.
    ByokOnly,
}

/// One of OpenRouter's provider-executed "server tools" (beta).
///
/// Server tools are tools OpenRouter runs server-side — it loops internally and
/// returns the final answer, so unlike client-executed function tools the agent
/// loop never dispatches them. The only client-visible artifact is
/// `usage.server_tool_use`. See
/// <https://openrouter.ai/docs/guides/features/server-tools>.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, serde::Serialize, serde::Deserialize)]
#[cfg_attr(feature = "openapi", derive(utoipa::ToSchema))]
#[serde(rename_all = "snake_case")]
pub enum OpenRouterServerToolKind {
    WebSearch,
    WebFetch,
    Datetime,
    ImageGeneration,
    ApplyPatch,
    Fusion,
    Advisor,
    Subagent,
}

impl OpenRouterServerToolKind {
    /// Every known server tool, in catalog order.
    pub const ALL: [OpenRouterServerToolKind; 8] = [
        Self::WebSearch,
        Self::WebFetch,
        Self::Datetime,
        Self::ImageGeneration,
        Self::ApplyPatch,
        Self::Fusion,
        Self::Advisor,
        Self::Subagent,
    ];

    /// Bare tool name (no prefix), e.g. `"web_search"`.
    pub fn name(&self) -> &'static str {
        match self {
            Self::WebSearch => "web_search",
            Self::WebFetch => "web_fetch",
            Self::Datetime => "datetime",
            Self::ImageGeneration => "image_generation",
            Self::ApplyPatch => "apply_patch",
            Self::Fusion => "fusion",
            Self::Advisor => "advisor",
            Self::Subagent => "subagent",
        }
    }

    /// Human-readable English display name, used for UI schema titles.
    pub fn display_name(&self) -> &'static str {
        match self {
            Self::WebSearch => "Web Search",
            Self::WebFetch => "Web Fetch",
            Self::Datetime => "Date & Time",
            Self::ImageGeneration => "Image Generation",
            Self::ApplyPatch => "Apply Patch",
            Self::Fusion => "Fusion",
            Self::Advisor => "Advisor",
            Self::Subagent => "Subagent",
        }
    }

    /// The `type` discriminator OpenRouter expects in the request `tools` array,
    /// e.g. `"openrouter:web_search"`.
    pub fn wire_type(&self) -> String {
        format!("openrouter:{}", self.name())
    }

    /// Parse a bare tool name (no `openrouter:` prefix).
    pub fn from_name(name: &str) -> Option<Self> {
        Self::ALL.into_iter().find(|kind| kind.name() == name)
    }
}

/// One activated OpenRouter server tool plus optional tool-specific parameters
/// (e.g. web_search `max_results`). Parameters are forwarded verbatim under the
/// wire entry's `parameters` field.
#[derive(Debug, Clone, PartialEq, serde::Serialize, serde::Deserialize)]
#[cfg_attr(feature = "openapi", derive(utoipa::ToSchema))]
pub struct OpenRouterServerTool {
    pub kind: OpenRouterServerToolKind,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[cfg_attr(feature = "openapi", schema(value_type = Option<Object>))]
    pub parameters: Option<serde_json::Value>,
}

impl OpenRouterServerTool {
    /// A server tool with no parameters.
    pub fn new(kind: OpenRouterServerToolKind) -> Self {
        Self {
            kind,
            parameters: None,
        }
    }

    /// A server tool carrying parameters forwarded verbatim to OpenRouter.
    pub fn with_parameters(kind: OpenRouterServerToolKind, parameters: serde_json::Value) -> Self {
        Self {
            kind,
            parameters: Some(parameters),
        }
    }
}

/// These fields mirror OpenRouter's request-level routing extensions. Drivers
/// must only forward this config to OpenRouter-compatible endpoints.
#[derive(Debug, Clone, Default, PartialEq, serde::Serialize, serde::Deserialize)]
#[cfg_attr(feature = "openapi", derive(utoipa::ToSchema))]
pub struct OpenRouterRoutingConfig {
    /// Candidate models to try in OpenRouter's fallback order.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub models: Vec<String>,
    /// OpenRouter route strategy. Currently `fallback` is the stable route
    /// value used with `models`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub route: Option<OpenRouterRoute>,
    /// Provider ordering, policy, and sorting preferences.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub provider: Option<OpenRouterProviderRouting>,
    /// Optional plugin activations (web search, file reader).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub plugins: Option<OpenRouterPluginConfig>,
    /// Org-level capacity strategy. Compiled into `provider` routing before
    /// dispatch; not forwarded verbatim. `None` and `SharedCapacity` are
    /// equivalent (no routing changes).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub capacity_strategy: Option<OpenRouterCapacityStrategy>,
    /// High-level routing quality/policy presets. Compiled into `provider`
    /// flags by `apply_presets()` before the request is serialized.
    /// Explicit `provider` fields override preset-derived values.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub presets: Vec<OpenRouterRoutingPreset>,
    /// OpenRouter server tools (beta) the model may invoke. Provider-executed;
    /// appended to the request `tools` array as `{"type":"openrouter:<name>"}`.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub server_tools: Vec<OpenRouterServerTool>,
}

impl OpenRouterRoutingConfig {
    pub fn is_empty(&self) -> bool {
        self.models.is_empty()
            && self.route.is_none()
            && self.provider.is_none()
            && self.plugins.as_ref().is_none_or(|p| p.is_empty())
            && matches!(
                self.capacity_strategy,
                None | Some(OpenRouterCapacityStrategy::SharedCapacity)
            )
            && self.presets.is_empty()
            && self.server_tools.is_empty()
    }

    /// Build an ordered model-fallback routing config.
    pub fn fallback_models(models: impl IntoIterator<Item = impl Into<String>>) -> Self {
        let models = models.into_iter().map(Into::into).collect::<Vec<_>>();
        let route = (!models.is_empty()).then_some(OpenRouterRoute::Fallback);
        Self {
            models,
            route,
            provider: None,
            plugins: None,
            capacity_strategy: None,
            presets: vec![],
            server_tools: vec![],
        }
    }

    pub fn validate_for_primary_model(
        &self,
        primary_model: &str,
    ) -> std::result::Result<(), String> {
        if self.route == Some(OpenRouterRoute::Fallback) && self.models.is_empty() {
            return Err(
                "OpenRouter fallback routing requires at least one model in `models`".to_string(),
            );
        }

        if let Some(first_model) = self.models.first()
            && first_model != primary_model
        {
            return Err(format!(
                "OpenRouter routing models[0] ('{first_model}') must match primary model ('{primary_model}')"
            ));
        }

        Ok(())
    }

    /// Apply the capacity strategy, returning a derived config with `provider`
    /// routing adjusted accordingly.
    ///
    /// - `SharedCapacity` / `None` — returns `self` unchanged.
    /// - `ByokFirst` — sets `provider.allow_fallbacks = true` when not already set.
    /// - `ByokOnly` — requires `provider.only` to list at least one provider slug;
    ///   sets `provider.allow_fallbacks = false`.
    ///
    /// Returns `Err` when the strategy constraints cannot be satisfied.
    pub fn apply_capacity_strategy(&self) -> std::result::Result<Self, String> {
        match self.capacity_strategy {
            None | Some(OpenRouterCapacityStrategy::SharedCapacity) => Ok(self.clone()),
            Some(OpenRouterCapacityStrategy::ByokFirst) => {
                let mut result = self.clone();
                let provider = result.provider.get_or_insert_with(Default::default);
                if provider.allow_fallbacks.is_none() {
                    provider.allow_fallbacks = Some(true);
                }
                Ok(result)
            }
            Some(OpenRouterCapacityStrategy::ByokOnly) => {
                let only_is_empty = self.provider.as_ref().is_none_or(|p| p.only.is_empty());
                if only_is_empty {
                    return Err(
                        "OpenRouter BYOK-only strategy requires provider.only to list at least \
                         one upstream provider slug. Configure the provider list to match the \
                         BYOK providers registered in your OpenRouter workspace."
                            .to_string(),
                    );
                }
                let mut result = self.clone();
                let provider = result.provider.get_or_insert_with(Default::default);
                provider.allow_fallbacks = Some(false);
                Ok(result)
            }
        }
    }

    /// Compile `presets` into `OpenRouterProviderRouting` flags and merge with
    /// any explicit `provider` overrides. Returns a derived config with the
    /// `presets` list cleared and `provider` reflecting the merged result.
    ///
    /// Explicit `provider` fields always win over preset-derived values. When
    /// multiple presets target the same provider field, later presets in the
    /// list override earlier ones.
    ///
    /// Returns `Err` if any preset values are invalid (e.g. negative `MaxPrice` values).
    pub fn apply_presets(&self) -> std::result::Result<Self, String> {
        if self.presets.is_empty() {
            return Ok(self.clone());
        }

        let mut derived = OpenRouterProviderRouting::default();

        for preset in &self.presets {
            match preset {
                OpenRouterRoutingPreset::CheapestWithTools => {
                    derived.require_parameters = Some(true);
                    derived.sort = Some(OpenRouterProviderSort::Simple(
                        OpenRouterProviderSortBy::Price,
                    ));
                }
                OpenRouterRoutingPreset::LowestLatencyReview => {
                    derived.sort = Some(OpenRouterProviderSort::Simple(
                        OpenRouterProviderSortBy::Throughput,
                    ));
                }
                OpenRouterRoutingPreset::ZdrOnly => {
                    derived.zdr = Some(true);
                }
                OpenRouterRoutingPreset::ByokFirst => {
                    if derived.allow_fallbacks.is_none() {
                        derived.allow_fallbacks = Some(true);
                    }
                }
                OpenRouterRoutingPreset::NoDataCollection => {
                    derived.data_collection = Some(OpenRouterDataCollection::Deny);
                }
                OpenRouterRoutingPreset::StrictJson
                | OpenRouterRoutingPreset::ReasoningRequired => {
                    derived.require_parameters = Some(true);
                }
                OpenRouterRoutingPreset::MaxPrice {
                    prompt_usd_per_million,
                    completion_usd_per_million,
                } => {
                    if prompt_usd_per_million.is_some_and(|v| v < 0.0)
                        || completion_usd_per_million.is_some_and(|v| v < 0.0)
                    {
                        return Err(
                            "MaxPrice preset values must be non-negative USD per million tokens"
                                .to_string(),
                        );
                    }
                    if prompt_usd_per_million.is_some() || completion_usd_per_million.is_some() {
                        let mp = derived.max_price.get_or_insert_with(Default::default);
                        if let Some(p) = prompt_usd_per_million {
                            mp.prompt = Some(p / 1_000_000.0);
                        }
                        if let Some(c) = completion_usd_per_million {
                            mp.completion = Some(c / 1_000_000.0);
                        }
                    }
                }
            }
        }

        // Explicit provider fields override preset-derived values.
        let merged = merge_provider_routing(derived, self.provider.clone().unwrap_or_default());

        let mut result = self.clone();
        result.presets = vec![];
        result.provider = if merged.is_empty() {
            None
        } else {
            Some(merged)
        };
        Ok(result)
    }
}

/// Merge preset-derived provider routing with explicit provider overrides.
/// Explicit fields always win; preset-derived fields fill gaps where explicit
/// fields are absent (None / empty Vec).
fn merge_provider_routing(
    derived: OpenRouterProviderRouting,
    explicit: OpenRouterProviderRouting,
) -> OpenRouterProviderRouting {
    OpenRouterProviderRouting {
        order: if !explicit.order.is_empty() {
            explicit.order
        } else {
            derived.order
        },
        only: if !explicit.only.is_empty() {
            explicit.only
        } else {
            derived.only
        },
        ignore: if !explicit.ignore.is_empty() {
            explicit.ignore
        } else {
            derived.ignore
        },
        allow_fallbacks: explicit.allow_fallbacks.or(derived.allow_fallbacks),
        require_parameters: explicit.require_parameters.or(derived.require_parameters),
        data_collection: explicit.data_collection.or(derived.data_collection),
        zdr: explicit.zdr.or(derived.zdr),
        enforce_distillable_text: explicit
            .enforce_distillable_text
            .or(derived.enforce_distillable_text),
        quantizations: if !explicit.quantizations.is_empty() {
            explicit.quantizations
        } else {
            derived.quantizations
        },
        sort: explicit.sort.or(derived.sort),
        max_price: explicit.max_price.or(derived.max_price),
    }
}

/// OpenRouter route strategy.
#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[cfg_attr(feature = "openapi", derive(utoipa::ToSchema))]
#[serde(rename_all = "snake_case")]
pub enum OpenRouterRoute {
    Fallback,
}

/// OpenRouter provider routing preferences.
#[derive(Debug, Clone, Default, PartialEq, serde::Serialize, serde::Deserialize)]
#[cfg_attr(feature = "openapi", derive(utoipa::ToSchema))]
pub struct OpenRouterProviderRouting {
    /// Provider slugs to try first, in order.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub order: Vec<String>,
    /// Restrict routing to these provider slugs.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub only: Vec<String>,
    /// Provider slugs to skip.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub ignore: Vec<String>,
    /// Whether OpenRouter may fall back outside the ordered/allowed providers.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub allow_fallbacks: Option<bool>,
    /// Require routed providers to support all request parameters.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub require_parameters: Option<bool>,
    /// Restrict routing by provider data-retention policy.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub data_collection: Option<OpenRouterDataCollection>,
    /// Restrict routing to zero-data-retention endpoints.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub zdr: Option<bool>,
    /// Restrict routing to distillable-text endpoints.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub enforce_distillable_text: Option<bool>,
    /// Restrict routing to provider quantization levels.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub quantizations: Vec<String>,
    /// Sort provider endpoints by price, throughput, or latency.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub sort: Option<OpenRouterProviderSort>,
    /// Maximum accepted per-unit provider price.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub max_price: Option<OpenRouterMaxPrice>,
}

impl OpenRouterProviderRouting {
    pub fn is_empty(&self) -> bool {
        self.order.is_empty()
            && self.only.is_empty()
            && self.ignore.is_empty()
            && self.allow_fallbacks.is_none()
            && self.require_parameters.is_none()
            && self.data_collection.is_none()
            && self.zdr.is_none()
            && self.enforce_distillable_text.is_none()
            && self.quantizations.is_empty()
            && self.sort.is_none()
            && self.max_price.is_none()
    }
}

/// OpenRouter provider data-retention preference.
#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[cfg_attr(feature = "openapi", derive(utoipa::ToSchema))]
#[serde(rename_all = "snake_case")]
pub enum OpenRouterDataCollection {
    Allow,
    Deny,
}

/// OpenRouter provider sort preference.
#[derive(Debug, Clone, PartialEq, serde::Serialize, serde::Deserialize)]
#[cfg_attr(feature = "openapi", derive(utoipa::ToSchema))]
#[serde(untagged)]
pub enum OpenRouterProviderSort {
    Simple(OpenRouterProviderSortBy),
    Advanced(OpenRouterProviderSortOptions),
}

/// OpenRouter provider sorting dimension.
#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[cfg_attr(feature = "openapi", derive(utoipa::ToSchema))]
#[serde(rename_all = "snake_case")]
pub enum OpenRouterProviderSortBy {
    Price,
    Throughput,
    Latency,
}

/// OpenRouter advanced provider sort options.
#[derive(Debug, Clone, PartialEq, serde::Serialize, serde::Deserialize)]
#[cfg_attr(feature = "openapi", derive(utoipa::ToSchema))]
pub struct OpenRouterProviderSortOptions {
    pub by: OpenRouterProviderSortBy,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub partition: Option<OpenRouterSortPartition>,
}

/// How OpenRouter sorts endpoints when multiple fallback models are present.
#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[cfg_attr(feature = "openapi", derive(utoipa::ToSchema))]
#[serde(rename_all = "snake_case")]
pub enum OpenRouterSortPartition {
    Model,
    None,
}

/// Maximum accepted OpenRouter provider pricing, expressed in dollars per
/// million prompt/completion tokens or per request/image where supported.
#[derive(Debug, Clone, Default, PartialEq, serde::Serialize, serde::Deserialize)]
#[cfg_attr(feature = "openapi", derive(utoipa::ToSchema))]
pub struct OpenRouterMaxPrice {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub prompt: Option<f64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub completion: Option<f64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub request: Option<f64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub image: Option<f64>,
}

/// OpenRouter web-search plugin configuration.
///
/// Instructs OpenRouter to retrieve and inject web search results before the
/// model sees the prompt. Only sent when the resolved provider type is
/// OpenRouter.
#[derive(Debug, Clone, Default, PartialEq, serde::Serialize, serde::Deserialize)]
#[cfg_attr(feature = "openapi", derive(utoipa::ToSchema))]
pub struct OpenRouterWebSearchPlugin {
    /// Maximum number of search results to include.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub max_results: Option<u32>,
    /// Custom search prompt hint passed to the web-search step.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub search_prompt: Option<String>,
}

/// OpenRouter file-reader plugin configuration.
///
/// Instructs OpenRouter to read and attach file contents before the model
/// sees the prompt. Only sent when the resolved provider type is OpenRouter.
#[derive(Debug, Clone, Default, PartialEq, serde::Serialize, serde::Deserialize)]
#[cfg_attr(feature = "openapi", derive(utoipa::ToSchema))]
pub struct OpenRouterFilePlugin {}

/// OpenRouter plugin configuration bundling optional plugin activations.
///
/// Any `None` plugin is omitted from the wire request. When all plugins are
/// `None`, no `plugins` field is emitted.
#[derive(Debug, Clone, Default, PartialEq, serde::Serialize, serde::Deserialize)]
#[cfg_attr(feature = "openapi", derive(utoipa::ToSchema))]
pub struct OpenRouterPluginConfig {
    /// Web-search plugin.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub web: Option<OpenRouterWebSearchPlugin>,
    /// File-reader plugin.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub file: Option<OpenRouterFilePlugin>,
}

impl OpenRouterPluginConfig {
    pub fn is_empty(&self) -> bool {
        self.web.is_none() && self.file.is_none()
    }
}

/// Metadata key consumed by the OpenRouter driver as `HTTP-Referer`.
pub const OPENROUTER_HTTP_REFERER_METADATA_KEY: &str = "openrouter.http_referer";
/// Metadata key consumed by the OpenRouter driver as `X-Title`.
pub const OPENROUTER_X_TITLE_METADATA_KEY: &str = "openrouter.x_title";

/// Configuration for an LLM call
#[derive(Debug, Clone)]
pub struct LlmCallConfig {
    pub model: String,
    pub temperature: Option<f32>,
    pub max_tokens: Option<u32>,
    pub tools: Vec<ToolDefinition>,
    /// Reasoning effort level (for models that support it: low, medium, high)
    pub reasoning_effort: Option<String>,
    /// Speed (service tier) for this call: "flex", "default", or "priority".
    /// Serialized as OpenAI `service_tier`; omitted when `None` so the
    /// provider keeps its default ("auto") routing.
    pub speed: Option<String>,
    /// Verbosity for this call: "low", "medium", or "high". Serialized as
    /// OpenAI `verbosity`; omitted when `None` so the provider keeps its
    /// default ("medium") output length.
    pub verbosity: Option<String>,
    /// Metadata to send with the API request for tracking and debugging.
    /// Keys and values are strings. Both OpenAI and Anthropic support metadata fields.
    /// Typically includes: session_id, agent_id, org_id, turn_id, exec_id.
    pub metadata: HashMap<String, String>,
    /// Previous response ID for stateful continuation (OpenAI Responses API).
    /// When set, the provider can skip re-encoding cached context.
    pub previous_response_id: Option<String>,
    /// Standalone, ordered native compact output for this request.
    ///
    /// This is mutually exclusive with `previous_response_id`. Provider
    /// drivers must serialize it as the request input without transcript-delta
    /// trimming or structural pruning.
    pub provider_opaque_context: Option<ProviderOpaqueContext>,
    /// Tool search configuration for deferred tool loading
    pub tool_search: Option<ToolSearchConfig>,
    /// Prompt caching configuration for provider-specific cache controls.
    pub prompt_cache: Option<PromptCacheConfig>,
    /// OpenRouter-only model fallback and provider routing controls.
    pub openrouter_routing: Option<OpenRouterRoutingConfig>,
    /// Request-level parallel tool calling preference (EVE-598).
    ///
    /// Serialized onto the provider request when `Some(_)`: OpenAI sets
    /// `parallel_tool_calls`; Anthropic maps `Some(false)` →
    /// `tool_choice.disable_parallel_tool_use = true`. `None` preserves
    /// provider defaults (no field sent).
    pub parallel_tool_calls: Option<bool>,
    /// Number of trailing messages that are volatile (regenerated every turn)
    /// and must not anchor a message-level prompt-cache breakpoint.
    ///
    /// `ReasonAtom` sets this to the count of live `<facts>` messages it appends
    /// at the conversation tail. Drivers that place a message cache breakpoint
    /// on the last block (Anthropic) skip this many trailing messages so the
    /// breakpoint lands on the last *stable* block — otherwise a tail that
    /// changes each turn would evict the conversation-history cache. `0` (the
    /// default) preserves the previous behavior exactly.
    pub volatile_suffix_len: usize,
}

impl LlmCallConfig {
    /// Resolve the effective wire value for `parallel_tool_calls`, gated by
    /// whether the driver/model can express it on the request.
    ///
    /// Returns `None` (omit the field, keep the provider default) when the
    /// preference is unset or `supported` is `false`. Drivers call this with
    /// `self.supports_parallel_tool_calls(&config.model)` so the preference is
    /// only serialized where the provider has a control for it. The local tool
    /// scheduler honors the preference independently, so `Some(false)` still
    /// serializes execution even when this returns `None`.
    pub fn resolved_parallel_tool_calls(&self, supported: bool) -> Option<bool> {
        if supported {
            self.parallel_tool_calls
        } else {
            None
        }
    }
}

// The `From<&RuntimeAgent>` adapter for LlmCallConfig lives in
// everruns-core (`llm_conversions`), since RuntimeAgent is a core domain type.

/// Response from an LLM call (non-streaming)
#[derive(Debug, Clone)]
pub struct LlmResponse {
    pub text: String,
    /// Thinking content from extended thinking models (e.g., Claude with thinking enabled)
    pub thinking: Option<String>,
    /// Cryptographic signature for thinking content (Anthropic Claude)
    pub thinking_signature: Option<String>,
    pub tool_calls: Option<Vec<ToolCall>>,
    pub metadata: LlmCompletionMetadata,
}

/// Builder for LlmCallConfig with fluent API
///
/// Chain methods like `reasoning_effort()`, `temperature()`, etc. and call
/// `build()` to get the final config. To start from a core `RuntimeAgent`, use
/// `everruns_core::llm_conversions::llm_call_config_builder_from_agent`.
pub struct LlmCallConfigBuilder {
    config: LlmCallConfig,
}

impl LlmCallConfigBuilder {
    /// Construct a builder wrapping an existing config.
    pub fn from_config(config: LlmCallConfig) -> Self {
        Self { config }
    }

    /// Set reasoning effort level (for models that support it: low, medium, high)
    pub fn reasoning_effort(mut self, effort: impl Into<String>) -> Self {
        self.config.reasoning_effort = Some(effort.into());
        self
    }

    /// Set speed (service tier): "flex", "default", or "priority"
    pub fn speed(mut self, speed: impl Into<String>) -> Self {
        self.config.speed = Some(speed.into());
        self
    }

    /// Set verbosity: "low", "medium", or "high"
    pub fn verbosity(mut self, verbosity: impl Into<String>) -> Self {
        self.config.verbosity = Some(verbosity.into());
        self
    }

    /// Set the model
    pub fn model(mut self, model: impl Into<String>) -> Self {
        self.config.model = model.into();
        self
    }

    /// Set temperature
    pub fn temperature(mut self, temp: f32) -> Self {
        self.config.temperature = Some(temp);
        self
    }

    /// Set max tokens
    pub fn max_tokens(mut self, tokens: u32) -> Self {
        self.config.max_tokens = Some(tokens);
        self
    }

    /// Set tools
    pub fn tools(mut self, tools: Vec<ToolDefinition>) -> Self {
        self.config.tools = tools;
        self
    }

    /// Set metadata for API tracking
    ///
    /// This metadata is sent to the LLM provider for tracking and debugging.
    /// Typically includes session_id, agent_id, org_id, turn_id, exec_id.
    pub fn metadata(mut self, metadata: HashMap<String, String>) -> Self {
        self.config.metadata = metadata;
        self
    }

    /// Add a single metadata key-value pair
    pub fn with_metadata(mut self, key: impl Into<String>, value: impl Into<String>) -> Self {
        self.config.metadata.insert(key.into(), value.into());
        self
    }

    /// Set previous response ID for stateful continuation
    pub fn previous_response_id(mut self, id: Option<String>) -> Self {
        self.config.previous_response_id = id;
        self
    }

    /// Set standalone provider-owned compact context for the request.
    pub fn provider_opaque_context(mut self, context: Option<ProviderOpaqueContext>) -> Self {
        self.config.provider_opaque_context = context;
        self
    }

    /// Set tool_search configuration
    pub fn tool_search(mut self, config: ToolSearchConfig) -> Self {
        self.config.tool_search = Some(config);
        self
    }

    /// Set prompt caching configuration
    pub fn prompt_cache(mut self, config: PromptCacheConfig) -> Self {
        self.config.prompt_cache = Some(config);
        self
    }

    /// Set OpenRouter model fallback and provider routing controls.
    pub fn openrouter_routing(mut self, config: OpenRouterRoutingConfig) -> Self {
        self.config.openrouter_routing = (!config.is_empty()).then_some(config);
        self
    }

    /// Set the request-level parallel tool calling preference (EVE-598).
    pub fn parallel_tool_calls(mut self, parallel_tool_calls: Option<bool>) -> Self {
        self.config.parallel_tool_calls = parallel_tool_calls;
        self
    }

    /// Set the number of trailing volatile messages that must not anchor a
    /// message-level prompt-cache breakpoint (see
    /// [`LlmCallConfig::volatile_suffix_len`]).
    pub fn volatile_suffix_len(mut self, len: usize) -> Self {
        self.config.volatile_suffix_len = len;
        self
    }

    /// Build the configuration
    pub fn build(self) -> LlmCallConfig {
        self.config
    }
}

// The Message->LlmMessage adapters (plain, with-images, and image-file
// helpers) live in everruns-core (`llm_conversions`): they depend on core
// domain types (Message, ContentPart, ResolvedImage).

// ============================================================================
// Driver Factory Types
// ============================================================================

pub use crate::provider::DriverId;

/// Extra provider-specific authentication/metadata beyond an API key.
///
/// Built-in providers ignore this; embedder-defined ([`DriverId::External`])
/// providers use it to carry OAuth tokens, account ids, or arbitrary extras
/// their driver factory needs.
#[derive(Clone, Default, PartialEq, Eq)]
pub struct ProviderMetadata {
    /// OAuth refresh token, when the provider authenticates via OAuth.
    pub refresh_token: Option<String>,
    /// Provider-side account identifier, when required.
    pub account_id: Option<String>,
    /// Arbitrary extra fields the driver factory understands.
    pub extra: Option<serde_json::Value>,
}

impl std::fmt::Debug for ProviderMetadata {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("ProviderMetadata")
            .field(
                "refresh_token",
                &self.refresh_token.as_ref().map(|_| "<configured>"),
            )
            .field("account_id", &self.account_id)
            .field("extra", &self.extra.as_ref().map(|_| "<configured>"))
            .finish()
    }
}

/// Configuration for creating an LLM provider
#[derive(Clone)]
pub struct ProviderConfig {
    /// Runtime service identity selected by the model.
    pub provider: crate::runtime_provider::ProviderKey,
    /// Type of provider
    pub provider_type: DriverId,
    /// API key for authentication
    pub api_key: Option<String>,
    /// Base URL override (optional)
    pub base_url: Option<String>,
    /// Extra provider-specific metadata (OAuth tokens, account ids, etc.).
    pub metadata: ProviderMetadata,
}

impl ProviderConfig {
    /// Create a new provider config
    pub fn new(provider_type: DriverId) -> Self {
        let provider = crate::runtime_provider::ProviderKey::new(provider_type.as_str());
        Self {
            provider,
            provider_type,
            api_key: None,
            base_url: None,
            metadata: ProviderMetadata::default(),
        }
    }

    /// Configure a runtime provider id independently from its hosted
    /// integration kind.
    pub fn for_provider(
        provider: impl Into<crate::runtime_provider::ProviderKey>,
        provider_type: DriverId,
    ) -> Self {
        Self {
            provider: provider.into(),
            provider_type,
            api_key: None,
            base_url: None,
            metadata: ProviderMetadata::default(),
        }
    }

    /// Set the API key
    pub fn with_api_key(mut self, api_key: impl Into<String>) -> Self {
        self.api_key = Some(api_key.into());
        self
    }

    /// Set the base URL
    pub fn with_base_url(mut self, base_url: impl Into<String>) -> Self {
        self.base_url = Some(base_url.into());
        self
    }

    /// Set provider-specific metadata.
    pub fn with_metadata(mut self, metadata: ProviderMetadata) -> Self {
        self.metadata = metadata;
        self
    }
}

impl std::fmt::Debug for ProviderConfig {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("ProviderConfig")
            .field("provider", &self.provider)
            .field("provider_type", &self.provider_type)
            .field("auth", &self.api_key.as_ref().map(|_| "<configured>"))
            .field("base_url", &self.base_url.as_ref().map(|_| "<configured>"))
            .field(
                "metadata",
                &self.metadata.extra.as_ref().map(|_| "<configured>"),
            )
            .finish()
    }
}

/// Everything a [`DriverFactory`] receives to build a driver instance.
///
/// Replaces the old `(api_key, base_url)` factory arguments so that
/// embedder-defined providers can receive richer auth via [`ProviderMetadata`]
/// without changing the factory signature again.
#[derive(Clone)]
pub struct DriverConfig {
    /// Runtime service identity.
    pub provider: crate::runtime_provider::ProviderKey,
    /// Provider type being created.
    pub provider_type: DriverId,
    /// Raw credential document, when one is configured. `None` for keyless
    /// providers (LlmSim, or external providers that authenticate via
    /// [`ProviderMetadata`]). For single-key drivers this is the API key
    /// verbatim; multi-field drivers should read [`DriverConfig::credentials`]
    /// instead of parsing this string.
    pub api_key: Option<String>,
    /// Typed credential fields parsed from the stored credential document (see
    /// [`crate::credential_schema::parse_credential_document`]). Multi-field
    /// drivers (Bedrock AWS keys, MAI Entra OAuth) read their declared fields
    /// from here instead of hand-parsing JSON out of `api_key`. Empty for
    /// keyless providers.
    pub credentials: std::collections::BTreeMap<String, String>,
    /// Base URL override, when configured.
    pub base_url: Option<String>,
    /// Extra provider-specific metadata.
    pub metadata: ProviderMetadata,
}

impl DriverConfig {
    /// Build a driver config from a resolved [`ProviderConfig`], parsing the
    /// credential document into the typed [`DriverConfig::credentials`] map.
    /// This is the single point where the stored credential string becomes
    /// typed fields, so every driver-creation path (server, worker, sync, dev)
    /// gets the same typed view.
    pub fn from_provider_config(config: &ProviderConfig) -> Self {
        Self {
            provider: config.provider.clone(),
            provider_type: config.provider_type.clone(),
            credentials: crate::credential_schema::parse_credential_document(
                config.api_key.as_deref(),
            ),
            api_key: config.api_key.clone(),
            base_url: config.base_url.clone(),
            metadata: config.metadata.clone(),
        }
    }

    /// A declared credential field's non-empty value, if present.
    pub fn credential(&self, name: &str) -> Option<&str> {
        self.credentials
            .get(name)
            .map(String::as_str)
            .filter(|s| !s.is_empty())
    }
}

impl std::fmt::Debug for DriverConfig {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("DriverConfig")
            .field("provider", &self.provider)
            .field("provider_type", &self.provider_type)
            .field("auth", &self.api_key.as_ref().map(|_| "<configured>"))
            .field(
                "credential_fields",
                &self.credentials.keys().collect::<Vec<_>>(),
            )
            .field("base_url", &self.base_url.as_ref().map(|_| "<configured>"))
            .finish()
    }
}

// The `From<&ResolvedModel>` adapter for ProviderConfig lives in
// everruns-core (`llm_conversions`), since ResolvedModel is a core domain type.

/// Boxed chat driver for dynamic dispatch
pub type BoxedChatDriver = Box<dyn ChatDriver>;

// ============================================================================
// EmbeddingsDriver Trait
// ============================================================================

/// Request to embed a batch of text strings into dense vectors.
#[derive(Debug, Clone)]
pub struct EmbedRequest {
    /// Texts to embed. All texts in a batch share the same model.
    pub texts: Vec<String>,
    /// Provider-side model id (e.g. `text-embedding-3-small`).
    pub model: String,
}

/// Response from an embedding request.
#[derive(Debug, Clone)]
pub struct EmbedResponse {
    /// One float vector per input text, in the same order.
    pub embeddings: Vec<Vec<f32>>,
    /// Total tokens consumed (for usage tracking). `None` if the provider
    /// does not report token counts.
    pub usage_tokens: Option<u32>,
    /// Actual cost of this call in USD, as reported by the provider inline
    /// (OpenAI-compatible gateways report `usage.cost`). `None` for providers
    /// that do not return a cost — direct OpenAI does not, same as the chat
    /// path (EVE-894).
    pub actual_cost_usd: Option<f64>,
}

/// Error returned by [`EmbeddingsDriver::embed`].
#[derive(Debug, thiserror::Error)]
pub enum EmbeddingsDriverError {
    #[error("embeddings provider returned an error: {0}")]
    Provider(String),
    #[error("embeddings request failed: {0}")]
    Transport(String),
}

/// Driver trait for text embedding services.
///
/// Implementors call their provider's embedding API and return dense float
/// vectors. Used by knowledge-base hybrid retrieval (see knowledge/runtime-resources/knowledge-bases.md
/// and knowledge/foundations/providers.md phase 6).
#[async_trait]
pub trait EmbeddingsDriver: Send + Sync {
    /// Embed a batch of texts and return one vector per input.
    async fn embed(
        &self,
        endpoint: &crate::runtime_provider::ProviderEndpoint,
        request: EmbedRequest,
    ) -> std::result::Result<EmbedResponse, EmbeddingsDriverError>;
}

#[async_trait]
impl EmbeddingsDriver for Box<dyn EmbeddingsDriver> {
    async fn embed(
        &self,
        endpoint: &crate::runtime_provider::ProviderEndpoint,
        request: EmbedRequest,
    ) -> std::result::Result<EmbedResponse, EmbeddingsDriverError> {
        (**self).embed(endpoint, request).await
    }
}

/// Boxed embeddings driver for dynamic dispatch.
pub type BoxedEmbeddingsDriver = Box<dyn EmbeddingsDriver>;

/// Factory function type for creating embeddings drivers.
pub type EmbeddingsDriverFactory =
    Arc<dyn Fn(&DriverConfig) -> BoxedEmbeddingsDriver + Send + Sync>;

// ============================================================================
// Driver Registry
// ============================================================================

/// Factory function type for creating chat drivers.
///
/// Receives a [`DriverConfig`] (provider type, optional key/base URL, and
/// provider metadata) and returns a boxed driver.
pub type DriverFactory = Arc<dyn Fn(&DriverConfig) -> BoxedChatDriver + Send + Sync>;

/// A typed service a provider driver can offer (see knowledge/foundations/providers.md).
///
/// Declared in code by each driver, never stored in the database. Only `Chat`
/// has a driver trait today; the set is additive and new kinds gain factories
/// on [`DriverDescriptor`] when their first consumer lands.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ServiceKind {
    /// Chat completion ([`ChatDriver`]).
    Chat,
    /// Text embeddings (planned: knowledge-base hybrid retrieval).
    Embeddings,
    /// Realtime voice sessions (server-side adapter using provider credentials).
    Realtime,
    /// Image generation.
    Images,
    /// Search-result reranking.
    Rerank,
}

impl std::fmt::Display for ServiceKind {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let s = match self {
            ServiceKind::Chat => "chat",
            ServiceKind::Embeddings => "embeddings",
            ServiceKind::Realtime => "realtime",
            ServiceKind::Images => "images",
            ServiceKind::Rerank => "rerank",
        };
        f.write_str(s)
    }
}

/// Wire flavor of a driver's interactive OAuth connect flow.
///
/// A driver may let an org admin connect a provider by authorizing in the
/// browser instead of pasting an API key. The flow always yields a long-lived
/// credential that lands in `providers.credentials_encrypted`, exactly like a
/// hand-entered key — so runtime resolution is unchanged and non-admin users
/// are unaffected (see knowledge/foundations/providers.md "OAuth provider connection").
///
/// Only OpenRouter's PKCE flavor exists today. Adding OAuth to another driver
/// means a new variant here (which the server matches on) plus a
/// [`DriverOAuthConfig`] on that driver's descriptor — never a parallel set of
/// endpoints.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DriverOAuthFlow {
    /// OpenRouter one-click PKCE
    /// (<https://openrouter.ai/docs/guides/overview/auth/oauth>): redirect the
    /// admin to `authorize_url?callback_url=..&code_challenge=..&code_challenge_method=S256`,
    /// then POST JSON `{code, code_verifier, code_challenge_method}` to
    /// `token_url`; the `key` field of the response is the user-controlled API
    /// key to store. No client registration or secret is required (public PKCE
    /// client).
    OpenRouterPkce,
}

/// A driver's declared OAuth connect flow.
///
/// Presence of this on a [`DriverDescriptor`] is what makes "Connect with
/// {provider}" available; absence means credentials must be entered manually.
#[derive(Debug, Clone)]
pub struct DriverOAuthConfig {
    /// Authorization endpoint the admin's browser is redirected to.
    pub authorize_url: String,
    /// Endpoint that exchanges the returned authorization code for a credential.
    pub token_url: String,
    /// Wire flavor of the two steps above.
    pub flow: DriverOAuthFlow,
}

impl DriverOAuthConfig {
    /// OpenRouter's one-click PKCE connect flow.
    pub fn openrouter() -> Self {
        Self {
            authorize_url: "https://openrouter.ai/auth".to_string(),
            token_url: "https://openrouter.ai/api/v1/auth/keys".to_string(),
            flow: DriverOAuthFlow::OpenRouterPkce,
        }
    }
}

/// A registered provider driver: identity, declared services, the credential
/// shape its providers must supply, and per-service factories.
///
/// The descriptor is the code-side unit of the providers domain model
/// (knowledge/foundations/providers.md): one descriptor per driver id, instantiated as many
/// org-scoped providers.
#[derive(Clone)]
pub struct DriverDescriptor {
    /// Driver id (also the registry key).
    pub id: DriverId,
    /// Human-readable driver name (e.g. "OpenAI", "AWS Bedrock").
    pub display_name: String,
    /// Services this driver's providers can power. Declared, not stored.
    pub services: Vec<ServiceKind>,
    /// Credential fields a provider instance must supply.
    pub credential_schema: CredentialFormSchema,
    /// Optional interactive OAuth connect flow. `Some` makes "Connect with
    /// {provider}" available as an alternative to entering a key by hand.
    pub oauth: Option<DriverOAuthConfig>,
    /// Chat service factory. `None` for drivers that only offer other services.
    pub chat: Option<DriverFactory>,
    /// Embeddings service factory. `None` for drivers that do not support embeddings.
    pub embeddings: Option<EmbeddingsDriverFactory>,
}

impl DriverDescriptor {
    /// Descriptor for a chat-only driver with the default credential schema
    /// for the driver id (a single required `api_key` field for real
    /// providers; empty for `LlmSim` and `External`, which may authenticate
    /// via [`ProviderMetadata`]) and a display name derived from the id.
    pub fn chat_only<F>(id: impl Into<DriverId>, factory: F) -> Self
    where
        F: Fn(&DriverConfig) -> BoxedChatDriver + Send + Sync + 'static,
    {
        let id = id.into();
        Self {
            display_name: default_display_name(&id),
            credential_schema: default_credential_schema(&id),
            services: vec![ServiceKind::Chat],
            oauth: None,
            chat: Some(Arc::new(factory)),
            embeddings: None,
            id,
        }
    }

    /// Whether the driver declares the given service.
    pub fn supports(&self, service: ServiceKind) -> bool {
        self.services.contains(&service)
    }
}

impl std::fmt::Debug for DriverDescriptor {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("DriverDescriptor")
            .field("id", &self.id)
            .field("display_name", &self.display_name)
            .field("services", &self.services)
            .field("oauth", &self.oauth.is_some())
            .field("chat", &self.chat.is_some())
            .field("embeddings", &self.embeddings.is_some())
            .finish()
    }
}

fn default_display_name(id: &DriverId) -> String {
    id.as_str().replace(['_', '-'], " ")
}

fn default_credential_schema(id: &DriverId) -> CredentialFormSchema {
    if id == &DriverId::LlmSim {
        CredentialFormSchema::empty()
    } else {
        CredentialFormSchema::api_key(String::new())
    }
}

/// Registry for LLM drivers
///
/// Enables dependency inversion: provider crates (everruns-anthropic, everruns-openai)
/// register their drivers at startup. The core has no direct knowledge of implementations.
///
/// # Example
///
/// ```ignore
/// use everruns_core::{DriverRegistry, DriverId};
/// use everruns_anthropic::register_driver;
/// use everruns_openai::register_driver as register_openai;
///
/// let mut registry = DriverRegistry::new();
/// everruns_anthropic::register_driver(&mut registry);
/// everruns_openai::register_driver(&mut registry);
///
/// // Later, create a driver from config
/// let driver = registry.create_chat_driver(&config)?;
/// ```
#[derive(Clone, Default)]
pub struct DriverRegistry {
    descriptors: HashMap<DriverId, DriverDescriptor>,
    providers: crate::runtime_provider::RuntimeProviderRegistry,
}

impl DriverRegistry {
    /// Create a new empty registry
    pub fn new() -> Self {
        Self {
            descriptors: HashMap::new(),
            providers: crate::runtime_provider::RuntimeProviderRegistry::new(),
        }
    }

    /// Register an application-supplied runtime provider directly.
    pub fn register_provider(
        &mut self,
        provider: crate::runtime_provider::RuntimeProvider,
    ) -> Result<()> {
        self.providers.register(provider)
    }

    /// Explicitly replace an application-supplied runtime provider.
    pub fn replace_provider(
        &mut self,
        provider: crate::runtime_provider::RuntimeProvider,
    ) -> Option<Arc<crate::runtime_provider::RuntimeProvider>> {
        self.providers.replace(provider)
    }

    /// Look up a directly registered runtime provider by service identity.
    pub fn provider(
        &self,
        id: &crate::runtime_provider::ProviderKey,
    ) -> Option<Arc<crate::runtime_provider::RuntimeProvider>> {
        self.providers.get(id)
    }

    /// Register a full driver descriptor.
    ///
    /// Panics if a descriptor is already registered for the same driver id —
    /// silent overwrites hide double-registration bugs. Use
    /// [`Self::register_descriptor_or_replace`] to overwrite intentionally.
    pub fn register_descriptor(&mut self, descriptor: DriverDescriptor) {
        if self.descriptors.contains_key(&descriptor.id) {
            panic!(
                "driver already registered for provider '{}'; \
                 use register_descriptor_or_replace to overwrite intentionally",
                descriptor.id
            );
        }
        self.descriptors.insert(descriptor.id.clone(), descriptor);
    }

    /// Register a full driver descriptor, replacing any existing one.
    pub fn register_descriptor_or_replace(&mut self, descriptor: DriverDescriptor) {
        self.descriptors.insert(descriptor.id.clone(), descriptor);
    }

    /// Register a driver factory for a provider type.
    ///
    /// Panics if a factory is already registered for `provider_type` — silent
    /// overwrites hide double-registration bugs. Use
    /// [`Self::register_or_replace`] to overwrite intentionally.
    pub fn register<F>(&mut self, provider_type: impl Into<DriverId>, factory: F)
    where
        F: Fn(&DriverConfig) -> BoxedChatDriver + Send + Sync + 'static,
    {
        self.register_descriptor(DriverDescriptor::chat_only(provider_type, factory));
    }

    /// Register a driver factory, replacing any existing one for the provider.
    ///
    /// Use when overwriting is intentional (e.g. swapping in an `LlmSim` driver
    /// for tests). Prefer [`Self::register`] otherwise so duplicates surface.
    pub fn register_or_replace<F>(&mut self, provider_type: impl Into<DriverId>, factory: F)
    where
        F: Fn(&DriverConfig) -> BoxedChatDriver + Send + Sync + 'static,
    {
        self.register_descriptor_or_replace(DriverDescriptor::chat_only(provider_type, factory));
    }

    /// Register a driver factory for an embedder-defined external provider,
    /// keyed by its canonical id. The id is normalized to lowercase (via
    /// [`DriverId::external`]) so it matches parsed lookups regardless of
    /// the casing stored in the database or sent on the wire.
    pub fn register_external<F>(&mut self, id: impl AsRef<str>, factory: F)
    where
        F: Fn(&DriverConfig) -> BoxedChatDriver + Send + Sync + 'static,
    {
        let mut descriptor = DriverDescriptor::chat_only(DriverId::external(id), factory);
        descriptor.credential_schema = CredentialFormSchema::empty();
        self.register_descriptor(descriptor);
    }

    /// Create an LLM driver based on configuration
    ///
    /// API keys must be provided in the config for real providers. This function does NOT fall back to
    /// environment variables. Keys should be decrypted from the database and passed here.
    /// Exception: `LlmSim` and `External` providers do not require an API key
    /// (external providers may authenticate via [`ProviderMetadata`]).
    ///
    /// Returns `DriverNotRegistered` error if no driver is registered for the provider type.
    pub fn create_chat_driver(&self, config: &ProviderConfig) -> Result<BoxedChatDriver> {
        if let Some(provider) = self.providers.get(&config.provider) {
            return Ok((*provider).clone().into_boxed_driver());
        }
        let descriptor = self.descriptors.get(&config.provider_type).ok_or_else(|| {
            AgentLoopError::driver_not_registered(config.provider_type.to_string())
        })?;
        let requires_api_key = descriptor
            .credential_schema
            .fields
            .iter()
            .any(|field| field.name == "api_key" && field.required && field.group.is_none());
        if requires_api_key && config.api_key.is_none() {
            return Err(AgentLoopError::llm(
                "API key is required. Configure the API key in provider settings.",
            ));
        }

        // Look up the descriptor and its chat factory for this provider type
        let factory = descriptor.chat.as_ref().ok_or_else(|| {
            AgentLoopError::llm(format!(
                "Provider driver '{}' does not implement the chat service.",
                config.provider_type
            ))
        })?;

        // Create the driver using the factory
        let driver_config = DriverConfig::from_provider_config(config);
        Ok(factory(&driver_config))
    }

    /// Check if a driver is registered for a provider type
    pub fn has_driver(&self, provider_type: &DriverId) -> bool {
        self.descriptors.contains_key(provider_type)
    }

    /// Get the registered descriptor for a provider type.
    pub fn descriptor(&self, provider_type: &DriverId) -> Option<&DriverDescriptor> {
        self.descriptors.get(provider_type)
    }

    /// Whether the registered driver declares the given service.
    pub fn supports(&self, provider_type: &DriverId, service: ServiceKind) -> bool {
        self.descriptors
            .get(provider_type)
            .is_some_and(|d| d.supports(service))
    }

    /// Driver ids whose descriptors declare the given service.
    pub fn providers_for(&self, service: ServiceKind) -> Vec<DriverId> {
        self.descriptors
            .values()
            .filter(|d| d.supports(service))
            .map(|d| d.id.clone())
            .collect()
    }

    /// Get the list of registered provider types
    pub fn registered_providers(&self) -> Vec<DriverId> {
        self.descriptors.keys().cloned().collect()
    }

    /// Runtime provider ids registered directly by an application.
    pub fn registered_provider_ids(&self) -> Vec<String> {
        self.providers.ids()
    }

    /// Create an embeddings driver based on configuration.
    ///
    /// API keys must be provided in the config for real providers. Exception:
    /// `LlmSim` and `External` providers do not require an API key.
    ///
    /// Returns an error if the driver is not registered or does not implement
    /// the embeddings service.
    pub fn create_embeddings_driver(
        &self,
        config: &ProviderConfig,
    ) -> std::result::Result<BoxedEmbeddingsDriver, EmbeddingsDriverError> {
        let requires_api_key = config.provider_type != DriverId::LlmSim;
        if requires_api_key && config.api_key.is_none() {
            return Err(EmbeddingsDriverError::Provider(
                "API key is required. Configure the API key in provider settings.".to_string(),
            ));
        }
        let descriptor = self.descriptors.get(&config.provider_type).ok_or_else(|| {
            EmbeddingsDriverError::Provider(format!(
                "No driver registered for provider '{}'",
                config.provider_type
            ))
        })?;
        let factory = descriptor.embeddings.as_ref().ok_or_else(|| {
            EmbeddingsDriverError::Provider(format!(
                "Provider driver '{}' does not implement the embeddings service.",
                config.provider_type
            ))
        })?;
        let driver_config = DriverConfig::from_provider_config(config);
        Ok(factory(&driver_config))
    }
}

/// Maximum tool result size in bytes before truncation (64 KiB).
/// Defense-in-depth backstop for tool results that bypass ActAtom hooks
/// (e.g. client-submitted or stored events). The primary hard limit is
/// enforced by `OutputHardLimitHook` (EVE-225) at tool execution time.
const MAX_TOOL_RESULT_BYTES: usize = 64 * 1024;

const TRUNCATION_SUFFIX: &str =
    "\n\n[Output truncated — exceeded 64 KiB limit. Try quiet flags, pipes, or redirect to file.]";

pub fn truncate_tool_result(text: String) -> String {
    if text.len() <= MAX_TOOL_RESULT_BYTES {
        return text;
    }
    let content_budget = MAX_TOOL_RESULT_BYTES.saturating_sub(TRUNCATION_SUFFIX.len());
    let mut end = content_budget;
    while end > 0 && !text.is_char_boundary(end) {
        end -= 1;
    }
    let mut truncated = text[..end].to_string();
    truncated.push_str(TRUNCATION_SUFFIX);
    truncated
}

// ============================================================================
// Tests
// ============================================================================

#[cfg(test)]
mod tests {
    use super::*;
    use crate::runtime_provider::ProviderEndpoint;

    #[test]
    fn test_disjoint_prompt_tokens_subtracts_cached_subset() {
        // Inclusive providers report a prompt count that includes cached reads;
        // normalization yields the non-cached remainder.
        assert_eq!(disjoint_prompt_tokens(1000, Some(800)), 200);
        // No cache reported => prompt count passes through unchanged.
        assert_eq!(disjoint_prompt_tokens(1000, None), 1000);
        assert_eq!(disjoint_prompt_tokens(1000, Some(0)), 1000);
        // Saturating: a provider reporting cache > input never underflows.
        assert_eq!(disjoint_prompt_tokens(800, Some(1000)), 0);
    }

    #[test]
    fn test_chat_driver_defaults_are_conservative_and_boxed_capabilities_forward() {
        // Default trait impl is conservative: drivers opt in.
        struct DefaultDriver;
        #[async_trait]
        impl ChatDriver for DefaultDriver {
            async fn chat_completion_stream(
                &self,
                _endpoint: &ProviderEndpoint,
                _messages: Vec<LlmMessage>,
                _config: &LlmCallConfig,
            ) -> Result<LlmResponseStream> {
                unreachable!()
            }
        }
        assert!(!DefaultDriver.supports_parallel_tool_calls("any-model"));
        assert!(!DefaultDriver.supports_stateful_responses());

        struct StatefulDriver;
        #[async_trait]
        impl ChatDriver for StatefulDriver {
            async fn chat_completion_stream(
                &self,
                _endpoint: &ProviderEndpoint,
                _messages: Vec<LlmMessage>,
                _config: &LlmCallConfig,
            ) -> Result<LlmResponseStream> {
                unreachable!()
            }

            fn supports_stateful_responses(&self) -> bool {
                true
            }
        }
        let boxed: BoxedChatDriver = Box::new(StatefulDriver);
        assert!(boxed.supports_stateful_responses());
    }

    #[test]
    fn test_fold_system_messages_none_when_absent() {
        let messages = vec![
            LlmMessage::text(LlmMessageRole::User, "hi"),
            LlmMessage::text(LlmMessageRole::Assistant, "ok"),
        ];
        assert_eq!(fold_system_messages(&messages), None);
    }

    #[test]
    fn test_fold_system_messages_single() {
        let messages = vec![
            LlmMessage::text(LlmMessageRole::System, "AGENT-PROMPT"),
            LlmMessage::text(LlmMessageRole::User, "hi"),
        ];
        assert_eq!(
            fold_system_messages(&messages),
            Some("AGENT-PROMPT".to_string())
        );
    }

    #[test]
    fn test_fold_system_messages_accumulates_in_order() {
        // The agent system prompt plus a later notice/summary System message
        // (infinity_context / compaction) must both survive, in order — the
        // later one must not overwrite the real agent system prompt.
        let messages = vec![
            LlmMessage::text(LlmMessageRole::System, "A"),
            LlmMessage::text(LlmMessageRole::User, "hi"),
            LlmMessage::text(LlmMessageRole::Assistant, "ok"),
            LlmMessage::text(LlmMessageRole::System, "B"),
        ];
        assert_eq!(fold_system_messages(&messages), Some("A\n\nB".to_string()));
    }

    #[test]
    fn test_fold_system_messages_concatenates_parts() {
        let messages = vec![LlmMessage::parts(
            LlmMessageRole::System,
            vec![
                LlmContentPart::text("foo"),
                LlmContentPart::image("data:image/png;base64,xxx"),
                LlmContentPart::text("bar"),
            ],
        )];
        assert_eq!(fold_system_messages(&messages), Some("foobar".to_string()));
    }

    #[test]
    fn test_openrouter_fallback_models_empty_is_empty() {
        let routing = OpenRouterRoutingConfig::fallback_models(std::iter::empty::<String>());

        assert!(routing.is_empty());
        assert_eq!(routing.route, None);
    }

    #[test]
    fn test_openrouter_routing_validates_primary_model() {
        let routing = OpenRouterRoutingConfig::fallback_models([
            "openai/gpt-5-mini",
            "anthropic/claude-sonnet-4.5",
        ]);

        assert!(
            routing
                .validate_for_primary_model("openai/gpt-5-mini")
                .is_ok()
        );
        let err = routing
            .validate_for_primary_model("anthropic/claude-sonnet-4.5")
            .unwrap_err();
        assert!(err.contains("models[0]"));
    }

    #[test]
    fn test_openrouter_routing_rejects_fallback_without_models() {
        let routing = OpenRouterRoutingConfig {
            route: Some(OpenRouterRoute::Fallback),
            ..Default::default()
        };

        let err = routing
            .validate_for_primary_model("openai/gpt-5-mini")
            .unwrap_err();
        assert!(err.contains("requires at least one model"));
    }

    #[test]
    fn test_openrouter_routing_serializes_request_fields() {
        let routing = OpenRouterRoutingConfig {
            models: vec![
                "openai/gpt-5-mini".to_string(),
                "anthropic/claude-sonnet-4.5".to_string(),
            ],
            route: Some(OpenRouterRoute::Fallback),
            provider: Some(OpenRouterProviderRouting {
                order: vec!["anthropic".to_string(), "openai".to_string()],
                allow_fallbacks: Some(false),
                require_parameters: Some(true),
                data_collection: Some(OpenRouterDataCollection::Deny),
                zdr: Some(true),
                sort: Some(OpenRouterProviderSort::Advanced(
                    OpenRouterProviderSortOptions {
                        by: OpenRouterProviderSortBy::Throughput,
                        partition: Some(OpenRouterSortPartition::None),
                    },
                )),
                max_price: Some(OpenRouterMaxPrice {
                    prompt: Some(1.0),
                    completion: Some(2.0),
                    ..Default::default()
                }),
                ..Default::default()
            }),
            ..Default::default()
        };

        let json = serde_json::to_value(routing).unwrap();

        assert_eq!(
            json,
            serde_json::json!({
                "models": [
                    "openai/gpt-5-mini",
                    "anthropic/claude-sonnet-4.5"
                ],
                "route": "fallback",
                "provider": {
                    "order": ["anthropic", "openai"],
                    "allow_fallbacks": false,
                    "require_parameters": true,
                    "data_collection": "deny",
                    "zdr": true,
                    "sort": {
                        "by": "throughput",
                        "partition": "none"
                    },
                    "max_price": {
                        "prompt": 1.0,
                        "completion": 2.0
                    }
                }
            })
        );
    }

    #[test]
    fn test_provider_type_parsing() {
        assert_eq!("openai".parse::<DriverId>().unwrap(), DriverId::OpenAI);
        assert_eq!(
            "openrouter".parse::<DriverId>().unwrap(),
            DriverId::OpenRouter
        );
        assert_eq!(
            "openai_completions".parse::<DriverId>().unwrap(),
            DriverId::OpenAICompletions
        );
        assert_eq!(
            "azure_openai".parse::<DriverId>().unwrap(),
            DriverId::AzureOpenAI
        );
        assert_eq!(
            "anthropic".parse::<DriverId>().unwrap(),
            DriverId::Anthropic
        );
        assert_eq!("gemini".parse::<DriverId>().unwrap(), DriverId::Gemini);
        // Unknown ids parse to External rather than erroring.
        assert_eq!(
            "ollama".parse::<DriverId>().unwrap(),
            DriverId::external("ollama")
        );
        assert_eq!(
            "custom".parse::<DriverId>().unwrap(),
            DriverId::external("custom")
        );
    }

    #[test]
    fn test_external_provider_id_is_case_insensitive() {
        // Built-in matching and external normalization are both case-folding,
        // so the same id in different casing resolves to one provider.
        assert_eq!("OpenAI".parse::<DriverId>().unwrap(), DriverId::OpenAI);
        assert_eq!(
            "Ollama".parse::<DriverId>().unwrap(),
            "ollama".parse::<DriverId>().unwrap()
        );
        assert_eq!(DriverId::external("OpenAI-Codex").as_str(), "openai-codex");
        // Registration and parsed lookup agree regardless of casing.
        assert_eq!(
            DriverId::external("MyProvider"),
            "myprovider".parse::<DriverId>().unwrap()
        );
    }

    #[test]
    fn test_provider_type_display() {
        assert_eq!(DriverId::OpenAI.to_string(), "openai");
        assert_eq!(DriverId::OpenRouter.to_string(), "openrouter");
        assert_eq!(DriverId::AzureOpenAI.to_string(), "azure_openai");
        assert_eq!(
            DriverId::OpenAICompletions.to_string(),
            "openai_completions"
        );
        assert_eq!(DriverId::Anthropic.to_string(), "anthropic");
        assert_eq!(DriverId::Gemini.to_string(), "gemini");
    }

    #[test]
    fn test_provider_config_builder() {
        let config = ProviderConfig::new(DriverId::Anthropic)
            .with_api_key("test-key")
            .with_base_url("https://custom.api.com");

        assert_eq!(config.provider_type, DriverId::Anthropic);
        assert_eq!(config.api_key, Some("test-key".to_string()));
        assert_eq!(config.base_url, Some("https://custom.api.com".to_string()));
    }

    #[test]
    fn provider_config_debug_redacts_runtime_values() {
        let config = ProviderConfig::new(DriverId::OpenAI)
            .with_api_key("secret-key")
            .with_base_url("https://user:password@example.test/v1?token=secret")
            .with_metadata(ProviderMetadata {
                refresh_token: Some("refresh-secret".into()),
                account_id: Some("account-1".into()),
                extra: Some(serde_json::json!({ "client_secret": "metadata-secret" })),
            });
        let debug = format!("{config:?}");
        for secret in [
            "secret-key",
            "password",
            "token=secret",
            "refresh-secret",
            "metadata-secret",
        ] {
            assert!(!debug.contains(secret), "debug output exposed {secret}");
        }
    }

    #[test]
    fn test_driver_registry_requires_api_key() {
        // Register a mock factory
        let mut registry = DriverRegistry::new();
        registry.register(DriverId::OpenAI, |_config| {
            // Return a mock driver - just need something that compiles
            struct MockDriver;
            #[async_trait]
            impl ChatDriver for MockDriver {
                async fn chat_completion_stream(
                    &self,
                    _endpoint: &ProviderEndpoint,
                    _messages: Vec<LlmMessage>,
                    _config: &LlmCallConfig,
                ) -> Result<LlmResponseStream> {
                    unimplemented!()
                }
            }
            Box::new(MockDriver)
        });

        // Driver without API key should fail
        let config = ProviderConfig::new(DriverId::OpenAI);
        let result = registry.create_chat_driver(&config);
        assert!(result.is_err());

        // Driver with API key should succeed
        let config_with_key = ProviderConfig::new(DriverId::OpenAI).with_api_key("test-key");
        let result = registry.create_chat_driver(&config_with_key);
        assert!(result.is_ok());
    }

    #[test]
    fn test_driver_registry_returns_error_for_unregistered_provider() {
        let registry = DriverRegistry::new();
        let config = ProviderConfig::new(DriverId::Anthropic).with_api_key("test-key");

        let result = registry.create_chat_driver(&config);

        // Should fail with DriverNotRegistered error
        if let Err(AgentLoopError::DriverNotRegistered(provider)) = result {
            assert_eq!(provider, "anthropic");
        } else {
            panic!("Expected DriverNotRegistered error");
        }
    }

    #[test]
    fn test_driver_registry_registration() {
        let mut registry = DriverRegistry::new();

        assert!(!registry.has_driver(&DriverId::OpenAI));
        assert!(!registry.has_driver(&DriverId::Anthropic));

        registry.register(DriverId::OpenAI, |_config| {
            struct MockDriver;
            #[async_trait]
            impl ChatDriver for MockDriver {
                async fn chat_completion_stream(
                    &self,
                    _endpoint: &ProviderEndpoint,
                    _messages: Vec<LlmMessage>,
                    _config: &LlmCallConfig,
                ) -> Result<LlmResponseStream> {
                    unimplemented!()
                }
            }
            Box::new(MockDriver)
        });

        assert!(registry.has_driver(&DriverId::OpenAI));
        assert!(!registry.has_driver(&DriverId::Anthropic));
    }

    #[test]
    fn test_register_external_and_create_driver_without_api_key() {
        struct MockDriver;
        #[async_trait]
        impl ChatDriver for MockDriver {
            async fn chat_completion_stream(
                &self,
                _endpoint: &ProviderEndpoint,
                _messages: Vec<LlmMessage>,
                _config: &LlmCallConfig,
            ) -> Result<LlmResponseStream> {
                unimplemented!()
            }
        }

        let mut registry = DriverRegistry::new();
        registry.register_external("openai-codex", |config| {
            // External providers may authenticate via metadata, not an api_key.
            assert_eq!(config.provider_type, DriverId::external("openai-codex"));
            Box::new(MockDriver)
        });

        assert!(registry.has_driver(&DriverId::external("openai-codex")));

        // No api_key required for external providers.
        let config = ProviderConfig::new(DriverId::external("openai-codex")).with_metadata(
            ProviderMetadata {
                refresh_token: Some("rt".into()),
                ..Default::default()
            },
        );
        assert!(registry.create_chat_driver(&config).is_ok());
    }

    #[test]
    fn test_register_defaults_to_chat_only_descriptor() {
        struct MockDriver;
        #[async_trait]
        impl ChatDriver for MockDriver {
            async fn chat_completion_stream(
                &self,
                _endpoint: &ProviderEndpoint,
                _messages: Vec<LlmMessage>,
                _config: &LlmCallConfig,
            ) -> Result<LlmResponseStream> {
                unimplemented!()
            }
        }

        let mut registry = DriverRegistry::new();
        registry.register(DriverId::Anthropic, |_config| Box::new(MockDriver));

        let descriptor = registry.descriptor(&DriverId::Anthropic).unwrap();
        assert_eq!(descriptor.display_name, "anthropic");
        assert_eq!(descriptor.services, vec![ServiceKind::Chat]);
        assert!(descriptor.chat.is_some());
        // Default credential shape is a single required api_key field.
        assert_eq!(descriptor.credential_schema.fields.len(), 1);
        assert_eq!(descriptor.credential_schema.fields[0].name, "api_key");
        assert!(descriptor.credential_schema.fields[0].required);

        // Keyless drivers default to an empty schema.
        registry.register(DriverId::LlmSim, |_config| Box::new(MockDriver));
        let sim = registry.descriptor(&DriverId::LlmSim).unwrap();
        assert!(sim.credential_schema.fields.is_empty());
    }

    #[test]
    fn test_descriptor_services_and_lookup() {
        struct MockDriver;
        #[async_trait]
        impl ChatDriver for MockDriver {
            async fn chat_completion_stream(
                &self,
                _endpoint: &ProviderEndpoint,
                _messages: Vec<LlmMessage>,
                _config: &LlmCallConfig,
            ) -> Result<LlmResponseStream> {
                unimplemented!()
            }
        }

        let mut registry = DriverRegistry::new();
        registry.register_descriptor(DriverDescriptor {
            services: vec![ServiceKind::Chat, ServiceKind::Realtime],
            ..DriverDescriptor::chat_only(DriverId::OpenAI, |_config| Box::new(MockDriver))
        });
        registry.register(DriverId::Anthropic, |_config| Box::new(MockDriver));

        assert!(registry.supports(&DriverId::OpenAI, ServiceKind::Chat));
        assert!(registry.supports(&DriverId::OpenAI, ServiceKind::Realtime));
        assert!(!registry.supports(&DriverId::Anthropic, ServiceKind::Realtime));
        assert!(!registry.supports(&DriverId::Gemini, ServiceKind::Chat));

        let realtime = registry.providers_for(ServiceKind::Realtime);
        assert_eq!(realtime, vec![DriverId::OpenAI]);
        let mut chat = registry.providers_for(ServiceKind::Chat);
        chat.sort_by_key(|p| p.to_string());
        assert_eq!(chat, vec![DriverId::Anthropic, DriverId::OpenAI]);
    }

    #[test]
    fn test_create_chat_driver_fails_without_chat_factory() {
        let mut registry = DriverRegistry::new();
        registry.register_descriptor(DriverDescriptor {
            id: DriverId::external("embeddings-only"),
            display_name: "Embeddings Only".to_string(),
            services: vec![ServiceKind::Embeddings],
            credential_schema: CredentialFormSchema::empty(),
            oauth: None,
            chat: None,
            embeddings: None,
        });

        let config = ProviderConfig::new(DriverId::external("embeddings-only"));
        let err = match registry.create_chat_driver(&config) {
            Ok(_) => panic!("expected error for missing chat factory"),
            Err(err) => err,
        };
        assert!(
            err.to_string()
                .contains("does not implement the chat service"),
            "unexpected error: {err}"
        );
    }

    #[test]
    #[should_panic(expected = "already registered")]
    fn test_register_duplicate_panics() {
        struct MockDriver;
        #[async_trait]
        impl ChatDriver for MockDriver {
            async fn chat_completion_stream(
                &self,
                _endpoint: &ProviderEndpoint,
                _messages: Vec<LlmMessage>,
                _config: &LlmCallConfig,
            ) -> Result<LlmResponseStream> {
                unimplemented!()
            }
        }

        let mut registry = DriverRegistry::new();
        registry.register(DriverId::OpenAI, |_config| Box::new(MockDriver));
        // Second registration for the same provider must panic.
        registry.register(DriverId::OpenAI, |_config| Box::new(MockDriver));
    }

    #[test]
    fn test_register_or_replace_overwrites() {
        struct MockDriver;
        #[async_trait]
        impl ChatDriver for MockDriver {
            async fn chat_completion_stream(
                &self,
                _endpoint: &ProviderEndpoint,
                _messages: Vec<LlmMessage>,
                _config: &LlmCallConfig,
            ) -> Result<LlmResponseStream> {
                unimplemented!()
            }
        }

        let mut registry = DriverRegistry::new();
        registry.register(DriverId::LlmSim, |_config| Box::new(MockDriver));
        // Replacing intentionally must not panic.
        registry.register_or_replace(DriverId::LlmSim, |_config| Box::new(MockDriver));
        assert!(registry.has_driver(&DriverId::LlmSim));
    }

    #[test]
    fn test_prepend_text_prefix_simple_text() {
        let mut msg = LlmMessage::text(LlmMessageRole::User, "Hello bot");
        msg.prepend_text_prefix("[Alice] ");
        assert_eq!(msg.content_as_text(), "[Alice] Hello bot");
    }

    #[test]
    fn test_prepend_text_prefix_parts() {
        let mut msg = LlmMessage::parts(
            LlmMessageRole::User,
            vec![
                LlmContentPart::Text {
                    text: "Hello".to_string(),
                },
                LlmContentPart::Image {
                    url: "data:image/png;base64,abc".to_string(),
                },
            ],
        );
        msg.prepend_text_prefix("[Bob] ");
        match &msg.content {
            LlmMessageContent::Parts(parts) => {
                if let LlmContentPart::Text { text } = &parts[0] {
                    assert_eq!(text, "[Bob] Hello");
                } else {
                    panic!("Expected text part");
                }
            }
            _ => panic!("Expected parts content"),
        }
    }

    #[test]
    fn test_prepend_text_prefix_parts_no_text() {
        let mut msg = LlmMessage::parts(
            LlmMessageRole::User,
            vec![LlmContentPart::Image {
                url: "data:image/png;base64,abc".to_string(),
            }],
        );
        msg.prepend_text_prefix("[Eve] ");
        match &msg.content {
            LlmMessageContent::Parts(parts) => {
                assert_eq!(parts.len(), 2);
                if let LlmContentPart::Text { text } = &parts[0] {
                    assert_eq!(text, "[Eve] ");
                } else {
                    panic!("Expected prepended text part");
                }
            }
            _ => panic!("Expected parts content"),
        }
    }

    #[test]
    fn test_openrouter_plugin_config_is_empty() {
        assert!(OpenRouterPluginConfig::default().is_empty());
        assert!(
            !OpenRouterPluginConfig {
                web: Some(OpenRouterWebSearchPlugin::default()),
                file: None,
            }
            .is_empty()
        );
        assert!(
            !OpenRouterPluginConfig {
                web: None,
                file: Some(OpenRouterFilePlugin {}),
            }
            .is_empty()
        );
    }

    #[test]
    fn test_openrouter_routing_is_empty_with_plugins() {
        let with_plugins = OpenRouterRoutingConfig {
            plugins: Some(OpenRouterPluginConfig {
                web: Some(OpenRouterWebSearchPlugin::default()),
                file: None,
            }),
            ..Default::default()
        };
        assert!(!with_plugins.is_empty());

        let empty_plugins = OpenRouterRoutingConfig {
            plugins: Some(OpenRouterPluginConfig::default()),
            ..Default::default()
        };
        assert!(empty_plugins.is_empty());
    }

    #[test]
    fn test_openrouter_web_search_plugin_serialization() {
        let plugin = OpenRouterWebSearchPlugin {
            max_results: Some(10),
            search_prompt: Some("search for Rust crates".to_string()),
        };
        let json = serde_json::to_value(&plugin).unwrap();
        assert_eq!(json["max_results"], 10);
        assert_eq!(json["search_prompt"], "search for Rust crates");
    }

    #[test]
    fn test_openrouter_web_search_plugin_omits_none_fields() {
        let plugin = OpenRouterWebSearchPlugin::default();
        let json = serde_json::to_value(&plugin).unwrap();
        assert!(json.get("max_results").is_none());
        assert!(json.get("search_prompt").is_none());
    }

    #[test]
    fn test_capacity_strategy_shared_capacity_is_noop() {
        let base = OpenRouterRoutingConfig {
            models: vec!["openai/gpt-5-mini".to_string()],
            capacity_strategy: Some(OpenRouterCapacityStrategy::SharedCapacity),
            ..Default::default()
        };
        let result = base.apply_capacity_strategy().unwrap();
        assert_eq!(
            result.capacity_strategy,
            Some(OpenRouterCapacityStrategy::SharedCapacity)
        );
        assert!(result.provider.is_none());
    }

    #[test]
    fn test_capacity_strategy_none_is_noop() {
        let base = OpenRouterRoutingConfig {
            models: vec!["openai/gpt-5-mini".to_string()],
            capacity_strategy: None,
            ..Default::default()
        };
        let result = base.apply_capacity_strategy().unwrap();
        assert!(result.provider.is_none());
    }

    #[test]
    fn test_capacity_strategy_byok_first_sets_allow_fallbacks() {
        let base = OpenRouterRoutingConfig {
            models: vec!["openai/gpt-5-mini".to_string()],
            capacity_strategy: Some(OpenRouterCapacityStrategy::ByokFirst),
            ..Default::default()
        };
        let result = base.apply_capacity_strategy().unwrap();
        let provider = result.provider.as_ref().expect("provider set by ByokFirst");
        assert_eq!(provider.allow_fallbacks, Some(true));
    }

    #[test]
    fn test_capacity_strategy_byok_first_preserves_explicit_allow_fallbacks() {
        // If allow_fallbacks was already set explicitly, ByokFirst must not override it.
        let base = OpenRouterRoutingConfig {
            models: vec!["openai/gpt-5-mini".to_string()],
            capacity_strategy: Some(OpenRouterCapacityStrategy::ByokFirst),
            provider: Some(OpenRouterProviderRouting {
                allow_fallbacks: Some(false),
                ..Default::default()
            }),
            ..Default::default()
        };
        let result = base.apply_capacity_strategy().unwrap();
        let provider = result.provider.as_ref().unwrap();
        assert_eq!(provider.allow_fallbacks, Some(false));
    }

    #[test]
    fn test_capacity_strategy_byok_only_requires_provider_only() {
        let base = OpenRouterRoutingConfig {
            models: vec!["openai/gpt-5-mini".to_string()],
            capacity_strategy: Some(OpenRouterCapacityStrategy::ByokOnly),
            ..Default::default()
        };
        let err = base.apply_capacity_strategy().unwrap_err();
        assert!(
            err.contains("provider.only"),
            "error should mention provider.only: {err}"
        );
    }

    #[test]
    fn test_capacity_strategy_byok_only_disables_fallbacks() {
        let base = OpenRouterRoutingConfig {
            models: vec!["openai/gpt-5-mini".to_string()],
            capacity_strategy: Some(OpenRouterCapacityStrategy::ByokOnly),
            provider: Some(OpenRouterProviderRouting {
                only: vec!["my-byok-provider".to_string()],
                ..Default::default()
            }),
            ..Default::default()
        };
        let result = base.apply_capacity_strategy().unwrap();
        let provider = result.provider.as_ref().unwrap();
        assert_eq!(provider.allow_fallbacks, Some(false));
        assert_eq!(provider.only, vec!["my-byok-provider"]);
    }

    #[test]
    fn test_capacity_strategy_byok_only_not_empty_in_is_empty() {
        let with_strategy = OpenRouterRoutingConfig {
            capacity_strategy: Some(OpenRouterCapacityStrategy::ByokOnly),
            ..Default::default()
        };
        assert!(!with_strategy.is_empty());

        let byok_first = OpenRouterRoutingConfig {
            capacity_strategy: Some(OpenRouterCapacityStrategy::ByokFirst),
            ..Default::default()
        };
        assert!(!byok_first.is_empty());

        let shared = OpenRouterRoutingConfig {
            capacity_strategy: Some(OpenRouterCapacityStrategy::SharedCapacity),
            ..Default::default()
        };
        assert!(shared.is_empty());
    }

    // -------------------------------------------------------------------------

    // OpenRouterRoutingPreset tests

    // -------------------------------------------------------------------------

    #[test]
    fn test_preset_no_presets_is_noop() {
        let base = OpenRouterRoutingConfig {
            models: vec!["openai/gpt-5-mini".to_string()],
            ..Default::default()
        };
        let result = base.apply_presets().unwrap();
        assert_eq!(result, base);
    }

    #[test]
    fn test_preset_cheapest_with_tools_sets_require_parameters_and_sort_price() {
        let base = OpenRouterRoutingConfig {
            presets: vec![OpenRouterRoutingPreset::CheapestWithTools],
            ..Default::default()
        };
        let result = base.apply_presets().unwrap();
        assert!(result.presets.is_empty(), "presets cleared after apply");
        let provider = result.provider.expect("provider set by preset");
        assert_eq!(provider.require_parameters, Some(true));
        assert_eq!(
            provider.sort,
            Some(OpenRouterProviderSort::Simple(
                OpenRouterProviderSortBy::Price
            ))
        );
    }

    #[test]
    fn test_preset_lowest_latency_review_sets_sort_throughput() {
        let base = OpenRouterRoutingConfig {
            presets: vec![OpenRouterRoutingPreset::LowestLatencyReview],
            ..Default::default()
        };
        let result = base.apply_presets().unwrap();
        let provider = result.provider.expect("provider set by preset");
        assert_eq!(
            provider.sort,
            Some(OpenRouterProviderSort::Simple(
                OpenRouterProviderSortBy::Throughput
            ))
        );
    }

    #[test]
    fn test_preset_zdr_only_sets_zdr() {
        let base = OpenRouterRoutingConfig {
            presets: vec![OpenRouterRoutingPreset::ZdrOnly],
            ..Default::default()
        };
        let result = base.apply_presets().unwrap();
        let provider = result.provider.expect("provider set");
        assert_eq!(provider.zdr, Some(true));
    }

    #[test]
    fn test_preset_byok_first_sets_allow_fallbacks() {
        let base = OpenRouterRoutingConfig {
            presets: vec![OpenRouterRoutingPreset::ByokFirst],
            ..Default::default()
        };
        let result = base.apply_presets().unwrap();
        let provider = result.provider.expect("provider set");
        assert_eq!(provider.allow_fallbacks, Some(true));
    }

    #[test]
    fn test_preset_no_data_collection_sets_data_collection_deny() {
        let base = OpenRouterRoutingConfig {
            presets: vec![OpenRouterRoutingPreset::NoDataCollection],
            ..Default::default()
        };
        let result = base.apply_presets().unwrap();
        let provider = result.provider.expect("provider set");
        assert_eq!(
            provider.data_collection,
            Some(OpenRouterDataCollection::Deny)
        );
    }

    #[test]
    fn test_preset_strict_json_sets_require_parameters() {
        let base = OpenRouterRoutingConfig {
            presets: vec![OpenRouterRoutingPreset::StrictJson],
            ..Default::default()
        };
        let result = base.apply_presets().unwrap();
        let provider = result.provider.expect("provider set");
        assert_eq!(provider.require_parameters, Some(true));
    }

    #[test]
    fn test_preset_reasoning_required_sets_require_parameters() {
        let base = OpenRouterRoutingConfig {
            presets: vec![OpenRouterRoutingPreset::ReasoningRequired],
            ..Default::default()
        };
        let result = base.apply_presets().unwrap();
        let provider = result.provider.expect("provider set");
        assert_eq!(provider.require_parameters, Some(true));
    }

    #[test]
    fn test_preset_max_price_converts_usd_per_million() {
        let base = OpenRouterRoutingConfig {
            presets: vec![OpenRouterRoutingPreset::MaxPrice {
                prompt_usd_per_million: Some(5.0),
                completion_usd_per_million: Some(15.0),
            }],
            ..Default::default()
        };
        let result = base.apply_presets().unwrap();
        let provider = result.provider.expect("provider set");
        let max_price = provider.max_price.expect("max_price set");
        // 5.0 USD/M → 5.0 / 1_000_000 per token
        let prompt = max_price.prompt.expect("prompt set");
        assert!((prompt - 5.0 / 1_000_000.0).abs() < f64::EPSILON);
        let completion = max_price.completion.expect("completion set");
        assert!((completion - 15.0 / 1_000_000.0).abs() < f64::EPSILON);
    }

    #[test]
    fn test_preset_max_price_rejects_negative_values() {
        let base = OpenRouterRoutingConfig {
            presets: vec![OpenRouterRoutingPreset::MaxPrice {
                prompt_usd_per_million: Some(-1.0),
                completion_usd_per_million: None,
            }],
            ..Default::default()
        };
        let err = base.apply_presets().unwrap_err();
        assert!(
            err.contains("non-negative"),
            "error should mention non-negative: {err}"
        );
    }

    #[test]
    fn test_preset_max_price_both_none_no_provider_field() {
        let base = OpenRouterRoutingConfig {
            presets: vec![OpenRouterRoutingPreset::MaxPrice {
                prompt_usd_per_million: None,
                completion_usd_per_million: None,
            }],
            ..Default::default()
        };
        let result = base.apply_presets().unwrap();
        assert!(
            result.provider.is_none(),
            "MaxPrice with no dimensions should not produce a provider field"
        );
    }

    #[test]
    fn test_preset_explicit_provider_overrides_preset() {
        let base = OpenRouterRoutingConfig {
            presets: vec![OpenRouterRoutingPreset::CheapestWithTools],
            provider: Some(OpenRouterProviderRouting {
                // Caller explicitly wants throughput sort, overriding Price preset
                sort: Some(OpenRouterProviderSort::Simple(
                    OpenRouterProviderSortBy::Throughput,
                )),
                ..Default::default()
            }),
            ..Default::default()
        };
        let result = base.apply_presets().unwrap();
        let provider = result.provider.expect("provider set");
        // Explicit sort wins
        assert_eq!(
            provider.sort,
            Some(OpenRouterProviderSort::Simple(
                OpenRouterProviderSortBy::Throughput
            ))
        );
        // But preset-derived require_parameters still set (not overridden by explicit)
        assert_eq!(provider.require_parameters, Some(true));
    }

    #[test]
    fn test_preset_multiple_presets_combined() {
        let base = OpenRouterRoutingConfig {
            presets: vec![
                OpenRouterRoutingPreset::ZdrOnly,
                OpenRouterRoutingPreset::NoDataCollection,
                OpenRouterRoutingPreset::LowestLatencyReview,
            ],
            ..Default::default()
        };
        let result = base.apply_presets().unwrap();
        let provider = result.provider.expect("provider set");
        assert_eq!(provider.zdr, Some(true));
        assert_eq!(
            provider.data_collection,
            Some(OpenRouterDataCollection::Deny)
        );
        assert_eq!(
            provider.sort,
            Some(OpenRouterProviderSort::Simple(
                OpenRouterProviderSortBy::Throughput
            ))
        );
    }

    #[test]
    fn test_preset_later_preset_overrides_sort() {
        let base = OpenRouterRoutingConfig {
            presets: vec![
                OpenRouterRoutingPreset::CheapestWithTools, // sets Price sort
                OpenRouterRoutingPreset::LowestLatencyReview, // overrides to Throughput
            ],
            ..Default::default()
        };
        let result = base.apply_presets().unwrap();
        let provider = result.provider.expect("provider set");
        // Later preset wins for sort
        assert_eq!(
            provider.sort,
            Some(OpenRouterProviderSort::Simple(
                OpenRouterProviderSortBy::Throughput
            ))
        );
        // require_parameters still set by CheapestWithTools
        assert_eq!(provider.require_parameters, Some(true));
    }

    #[test]
    fn test_preset_non_empty_in_is_empty() {
        let with_preset = OpenRouterRoutingConfig {
            presets: vec![OpenRouterRoutingPreset::ZdrOnly],
            ..Default::default()
        };
        assert!(!with_preset.is_empty());

        let without = OpenRouterRoutingConfig::default();
        assert!(without.is_empty());
    }
}
