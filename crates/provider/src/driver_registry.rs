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
    OpenResponsesCompact {
        output: Vec<CompactOutputItem>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        reasoning_state: Option<crate::reasoning_updates::ReasoningState>,
    },
}

impl std::fmt::Debug for ProviderOpaqueContext {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::OpenResponsesCompact { output, .. } => f
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
    /// Provider-reported prompt-cache diagnostics, verbatim.
    ///
    /// Present only when the request opted in via
    /// [`LlmCallConfig::cache_diagnostics`] and the provider answered with a
    /// diagnostics payload (today: Anthropic's `cache-diagnosis` beta). The
    /// shape is provider-owned, so the runtime carries it without interpreting
    /// it.
    pub cache_diagnostics: Option<serde_json::Value>,
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
        let mut reasoning: Vec<crate::reasoning::ReasoningContentPart> = Vec::new();
        let mut tool_calls = Vec::new();
        let mut metadata = LlmCompletionMetadata::default();

        while let Some(event) = stream.next().await {
            match event? {
                LlmStreamEvent::TextDelta(delta) => text.push_str(&delta),
                // Deltas are a live-rendering concern; the terminal
                // `ReasoningItem` carries the durable artifact.
                LlmStreamEvent::ReasoningDelta { .. } => {}
                LlmStreamEvent::ReasoningItem(item) => reasoning.push(item),
                LlmStreamEvent::ToolCalls(calls) => tool_calls = calls,
                LlmStreamEvent::NativeToolCall(_) => {
                    return Err(crate::error::AgentLoopError::config(
                        "native async/custom calls require a streaming coordinator",
                    ));
                }
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
            reasoning,
            tool_calls: if tool_calls.is_empty() {
                None
            } else {
                Some(tool_calls)
            },
            metadata,
        })
    }

    /// Whether this driver can complete without SSE on the wire.
    ///
    /// When `false` (the default), [`Self::chat_completion_non_streaming`]
    /// falls back to collecting [`Self::chat_completion_stream`], so callers
    /// still wait for one full response but the provider call streams
    /// underneath. Drivers with a native `stream: false` JSON endpoint
    /// return `true` and issue a single request/response call instead.
    fn supports_native_non_streaming(&self) -> bool {
        false
    }

    /// Call the LLM and wait for the full response without SSE.
    ///
    /// This is the non-streaming counterpart to
    /// [`Self::chat_completion_stream`]: no `LlmStreamEvent`s reach the
    /// caller. The default collects the stream; drivers with a native
    /// non-streaming endpoint override this to use it.
    async fn chat_completion_non_streaming(
        &self,
        endpoint: &crate::runtime_provider::ProviderEndpoint,
        messages: Vec<LlmMessage>,
        config: &LlmCallConfig,
    ) -> Result<LlmResponse> {
        self.chat_completion(endpoint, messages, config).await
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

    fn supports_native_non_streaming(&self) -> bool {
        (**self).supports_native_non_streaming()
    }

    async fn chat_completion_non_streaming(
        &self,
        endpoint: &crate::runtime_provider::ProviderEndpoint,
        messages: Vec<LlmMessage>,
        config: &LlmCallConfig,
    ) -> Result<LlmResponse> {
        (**self)
            .chat_completion_non_streaming(endpoint, messages, config)
            .await
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
    /// Provider reasoning artifacts for this assistant turn, in emission order.
    ///
    /// Drivers replay these verbatim in the position the provider issued them:
    /// each keeps its own signature, id and encrypted payload, so interleaved
    /// thinking and per-call thought signatures survive a round trip. Empty for
    /// messages without reasoning.
    pub reasoning: Vec<crate::reasoning::ReasoningContentPart>,
    /// Astra effort transition immediately before this message. Other protocols
    /// ignore it; it is never rendered as conversation text.
    pub configuration_update: Option<crate::model::ReasoningEffort>,
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
            reasoning: Vec::new(),
            configuration_update: None,
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
            reasoning: Vec::new(),
            configuration_update: None,
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
    /// File content, e.g. a PDF document (base64 data URL or file URL)
    File {
        url: String,
        filename: Option<String>,
    },
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

    /// Create a file content part from URL (typically a data URL)
    pub fn file(url: impl Into<String>, filename: Option<String>) -> Self {
        LlmContentPart::File {
            url: url.into(),
            filename,
        }
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

/// Per-request prompt-cache diagnostics controls.
///
/// Anthropic's `cache-diagnosis` beta fingerprints each request and, on the
/// next one, reports where the prompt prefix diverged (model, system prompt,
/// tools, or message history) instead of leaving a silent cache miss. Drivers
/// that have no diagnostics protocol ignore this without failing the call.
#[derive(Debug, Clone, Default, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[cfg_attr(feature = "openapi", derive(utoipa::ToSchema))]
pub struct CacheDiagnosticsConfig {
    /// Opt this request into diagnostics.
    pub enabled: bool,
    /// Provider response id of the request to compare this one against.
    ///
    /// `None` opts in without a prior request: Anthropic requires the field to
    /// be present and explicitly `null` on the first turn, so drivers must
    /// serialize it rather than skip it.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub previous_message_id: Option<String>,
}

/// Configuration for an LLM call
#[derive(Debug, Clone)]
pub struct LlmCallConfig {
    /// Durable Astra baseline and effective effort; absent for other modes.
    pub reasoning_state: Option<crate::reasoning_updates::ReasoningState>,
    pub model: String,
    pub temperature: Option<f32>,
    pub max_tokens: Option<u32>,
    pub tools: Vec<ToolDefinition>,
    /// Reasoning effort for models that support it.
    ///
    /// `None` means unset — the provider keeps its default. `Some(None)` is the
    /// caller explicitly asking for no reasoning, which drivers honor by
    /// omitting the reasoning request fields rather than sending a default.
    pub reasoning_effort: Option<crate::model::ReasoningEffort>,
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
    /// Driver-namespaced opaque per-call options (`"<driver-id>/<option>"`, e.g.
    /// `"openrouter/routing"`). Each entry's shape is owned by the driver crate
    /// named in the key; this crate never interprets the values.
    pub driver_options: HashMap<String, serde_json::Value>,
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
    /// Extra HTTP headers to attach to every provider request made for this
    /// call.
    ///
    /// Merged case-insensitively over the driver's protocol headers and the
    /// provider's configured/auth headers, so a caller value replaces an
    /// existing header instead of appending a second copy. Connection-level
    /// headers are dropped (see
    /// [`merge_request_headers`](crate::driver_helpers::merge_request_headers)).
    pub extra_headers: Vec<(String, String)>,
    /// Prompt-cache diagnostics requested for this call.
    pub cache_diagnostics: Option<CacheDiagnosticsConfig>,
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
    /// Provider reasoning artifacts, in emission order.
    pub reasoning: Vec<crate::reasoning::ReasoningContentPart>,
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

    /// Set reasoning effort for models that support it.
    pub fn reasoning_effort(mut self, effort: crate::model::ReasoningEffort) -> Self {
        self.config.reasoning_effort = Some(effort);
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

    /// Set a driver-namespaced opaque per-call option (`"<driver-id>/<option>"`).
    /// The value's shape is owned by the driver crate named in the key; this
    /// crate passes it through untouched.
    pub fn driver_option(mut self, key: impl Into<String>, value: serde_json::Value) -> Self {
        self.config.driver_options.insert(key.into(), value);
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

    /// Replace the extra HTTP headers sent with this call.
    pub fn extra_headers(mut self, headers: Vec<(String, String)>) -> Self {
        self.config.extra_headers = headers;
        self
    }

    /// Add one extra HTTP header to send with this call.
    pub fn extra_header(mut self, name: impl Into<String>, value: impl Into<String>) -> Self {
        self.config.extra_headers.push((name.into(), value.into()));
        self
    }

    /// Request provider prompt-cache diagnostics for this call.
    pub fn cache_diagnostics(mut self, config: CacheDiagnosticsConfig) -> Self {
        self.config.cache_diagnostics = Some(config);
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
    /// Connection-level request options (extra headers, diagnostics opt-in)
    /// applied to every call made through this provider.
    pub request_options: crate::provider::ProviderRequestOptions,
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
            request_options: Default::default(),
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
            request_options: Default::default(),
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

    /// Set the connection-level request options.
    pub fn with_request_options(
        mut self,
        request_options: crate::provider::ProviderRequestOptions,
    ) -> Self {
        self.request_options = request_options;
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

/// A fully constructed driver whose provider selection is valid but whose
/// credential document is not yet configured.
///
/// Hosts must be able to assemble a turn context so setup commands can repair
/// provider configuration. The gate therefore sits at the first operation
/// that could reach the provider, rather than at driver construction time.
struct CredentialGateDriver {
    inner: BoxedChatDriver,
    message: String,
}

impl CredentialGateDriver {
    fn error(&self) -> AgentLoopError {
        AgentLoopError::llm_kind(LlmErrorKind::Authentication, self.message.clone())
    }
}

#[async_trait]
impl ChatDriver for CredentialGateDriver {
    async fn chat_completion_stream(
        &self,
        _endpoint: &crate::runtime_provider::ProviderEndpoint,
        _messages: Vec<LlmMessage>,
        _config: &LlmCallConfig,
    ) -> Result<LlmResponseStream> {
        Err(self.error())
    }

    async fn list_models(
        &self,
        _endpoint: &crate::runtime_provider::ProviderEndpoint,
    ) -> Result<Option<Vec<DiscoveredModel>>> {
        Err(self.error())
    }

    fn supports_compact(&self) -> bool {
        self.inner.supports_compact()
    }

    fn supports_stateful_responses(&self) -> bool {
        self.inner.supports_stateful_responses()
    }

    fn effective_context_window(&self, model: &str) -> Option<usize> {
        self.inner.effective_context_window(model)
    }

    fn supports_parallel_tool_calls(&self, model: &str) -> bool {
        self.inner.supports_parallel_tool_calls(model)
    }

    async fn compact(
        &self,
        _endpoint: &crate::runtime_provider::ProviderEndpoint,
        _request: CompactRequest,
    ) -> Result<Option<CompactResponse>> {
        Err(self.error())
    }
}

/// Applies a provider connection's [`ProviderRequestOptions`] to every call made
/// through it, by rewriting the per-call [`LlmCallConfig`] before delegating.
///
/// This sits above the wire drivers on purpose: the options are expressed in
/// terms drivers already understand (`extra_headers`, `cache_diagnostics`), so
/// no driver needs to know that a connection can carry them, and a driver that
/// implements neither simply ignores the config it is handed.
struct RequestOptionsDriver {
    inner: BoxedChatDriver,
    options: crate::provider::ProviderRequestOptions,
}

impl RequestOptionsDriver {
    /// Wrap `driver` when `options` change anything; otherwise hand it back
    /// unchanged so the common path adds no indirection.
    fn wrap(
        driver: BoxedChatDriver,
        options: &crate::provider::ProviderRequestOptions,
    ) -> BoxedChatDriver {
        if options.is_empty() {
            return driver;
        }
        Box::new(Self {
            inner: driver,
            options: options.clone(),
        })
    }

    fn apply(&self, config: &LlmCallConfig) -> LlmCallConfig {
        let mut config = config.clone();
        config.extra_headers.extend(self.options.header_pairs());
        if self.options.cache_diagnostics {
            config.cache_diagnostics = Some(CacheDiagnosticsConfig {
                enabled: true,
                // Chain to the previous generation of this turn so the provider
                // can report *where* the prompt prefix diverged. `None` on the
                // first call opts in without a comparison point.
                previous_message_id: config.previous_response_id.clone(),
            });
        }
        config
    }
}

#[async_trait]
impl ChatDriver for RequestOptionsDriver {
    async fn chat_completion_stream(
        &self,
        endpoint: &crate::runtime_provider::ProviderEndpoint,
        messages: Vec<LlmMessage>,
        config: &LlmCallConfig,
    ) -> Result<LlmResponseStream> {
        self.inner
            .chat_completion_stream(endpoint, messages, &self.apply(config))
            .await
    }

    async fn chat_completion(
        &self,
        endpoint: &crate::runtime_provider::ProviderEndpoint,
        messages: Vec<LlmMessage>,
        config: &LlmCallConfig,
    ) -> Result<LlmResponse> {
        self.inner
            .chat_completion(endpoint, messages, &self.apply(config))
            .await
    }

    fn supports_native_non_streaming(&self) -> bool {
        self.inner.supports_native_non_streaming()
    }

    async fn chat_completion_non_streaming(
        &self,
        endpoint: &crate::runtime_provider::ProviderEndpoint,
        messages: Vec<LlmMessage>,
        config: &LlmCallConfig,
    ) -> Result<LlmResponse> {
        self.inner
            .chat_completion_non_streaming(endpoint, messages, &self.apply(config))
            .await
    }

    async fn list_models(
        &self,
        endpoint: &crate::runtime_provider::ProviderEndpoint,
    ) -> Result<Option<Vec<DiscoveredModel>>> {
        self.inner.list_models(endpoint).await
    }

    fn supports_compact(&self) -> bool {
        self.inner.supports_compact()
    }

    fn supports_stateful_responses(&self) -> bool {
        self.inner.supports_stateful_responses()
    }

    fn effective_context_window(&self, model: &str) -> Option<usize> {
        self.inner.effective_context_window(model)
    }

    fn supports_parallel_tool_calls(&self, model: &str) -> bool {
        self.inner.supports_parallel_tool_calls(model)
    }

    async fn compact(
        &self,
        endpoint: &crate::runtime_provider::ProviderEndpoint,
        request: CompactRequest,
    ) -> Result<Option<CompactResponse>> {
        self.inner.compact(endpoint, request).await
    }
}

/// A typed service a provider driver can offer (see knowledge/foundations/providers.md).
///
/// Declared in code by each driver, never stored in the database. Only `Chat`
/// has a driver trait today; the set is additive and new kinds gain factories
/// on [`DriverDescriptor`] when their first consumer lands.
///
/// Defined in `everruns-model-profiles` (profile data is keyed by service
/// kind) and re-exported here for source compatibility.
pub use everruns_model_profiles::ServiceKind;

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
    /// This function does not fall back to environment variables. Keys should
    /// be decrypted by the host and passed here. A selected provider with
    /// missing credentials still produces a driver so commands can inspect the
    /// turn and repair configuration; that driver rejects every provider
    /// operation locally before network I/O.
    ///
    /// Returns `DriverNotRegistered` error if no driver is registered for the provider type.
    pub fn create_chat_driver(&self, config: &ProviderConfig) -> Result<BoxedChatDriver> {
        if let Some(provider) = self.providers.get(&config.provider) {
            return Ok(RequestOptionsDriver::wrap(
                (*provider).clone().into_boxed_driver(),
                &config.request_options,
            ));
        }
        let descriptor = self.descriptors.get(&config.provider_type).ok_or_else(|| {
            AgentLoopError::driver_not_registered(config.provider_type.to_string())
        })?;
        // Look up the descriptor and its chat factory for this provider type
        let factory = descriptor.chat.as_ref().ok_or_else(|| {
            AgentLoopError::llm(format!(
                "Provider driver '{}' does not implement the chat service.",
                config.provider_type
            ))
        })?;

        // Create the driver using the factory
        let driver_config = DriverConfig::from_provider_config(config);
        let driver = factory(&driver_config);
        let mut credential_fields = driver_config.credentials.clone();
        if let Some(serde_json::Value::Object(extra)) = &driver_config.metadata.extra {
            for (name, value) in extra {
                if let Some(value) = value.as_str() {
                    credential_fields
                        .entry(name.clone())
                        .or_insert_with(|| value.to_string());
                }
            }
        }
        let credential_errors = descriptor.credential_schema.validate(&credential_fields);
        if credential_errors.is_empty() {
            Ok(RequestOptionsDriver::wrap(driver, &config.request_options))
        } else {
            let message = if descriptor.credential_schema.fields.len() == 1
                && descriptor.credential_schema.fields[0].name == "api_key"
            {
                "API key is required. Configure the API key in provider settings.".to_string()
            } else {
                format!(
                    "Provider credentials are required. Configure provider settings: {}",
                    credential_errors.join(" ")
                )
            };
            Ok(Box::new(CredentialGateDriver {
                inner: driver,
                message,
            }))
        }
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

    /// A call config with nothing set beyond the model, so a test can assert on
    /// exactly what a wrapper adds.
    fn bare_call_config() -> LlmCallConfig {
        LlmCallConfig {
            model: "claude-opus-4-8".to_string(),
            temperature: None,
            max_tokens: None,
            tools: vec![],
            reasoning_effort: None,
            speed: None,
            verbosity: None,
            metadata: HashMap::new(),
            previous_response_id: None,
            provider_opaque_context: None,
            tool_search: None,
            prompt_cache: None,
            driver_options: Default::default(),
            parallel_tool_calls: None,
            volatile_suffix_len: 0,
            extra_headers: Vec::new(),
            cache_diagnostics: None,
            reasoning_state: None,
        }
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
        assert!(debug.contains("ProviderConfig"));
        assert!(debug.contains("openai"));
        assert!(debug.contains("<configured>"));
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
    fn system_messages_fold_only_system_text_in_transcript_order() {
        use LlmMessageRole::{Assistant, System, Tool, User};
        for (messages, expected) in [
            (vec![], None),
            (
                vec![
                    LlmMessage::text(User, "user"),
                    LlmMessage::text(Assistant, "answer"),
                    LlmMessage::text(Tool, "result"),
                ],
                None,
            ),
            (vec![LlmMessage::text(System, "")], Some("")),
            (
                vec![
                    LlmMessage::text(System, "rules"),
                    LlmMessage::text(User, "question"),
                ],
                Some("rules"),
            ),
            (
                vec![
                    LlmMessage::text(System, "first"),
                    LlmMessage::text(User, "question"),
                    LlmMessage::text(System, "second"),
                    LlmMessage::text(Assistant, "answer"),
                    LlmMessage::text(System, "third"),
                ],
                Some("first\n\nsecond\n\nthird"),
            ),
            (
                vec![
                    LlmMessage::parts(
                        System,
                        vec![
                            LlmContentPart::text("foo"),
                            LlmContentPart::image("image"),
                            LlmContentPart::audio("audio"),
                            LlmContentPart::text("bar"),
                        ],
                    ),
                    LlmMessage::text(System, "next"),
                ],
                Some("foobar\n\nnext"),
            ),
        ] {
            assert_eq!(fold_system_messages(&messages).as_deref(), expected);
        }
    }

    #[test]
    fn prefix_preserves_all_media_and_changes_only_the_first_text_part() {
        let mut plain = LlmMessage::text(LlmMessageRole::User, "Hello");
        plain.prepend_text_prefix("[Alice] ");
        assert!(
            matches!(plain.content, LlmMessageContent::Text(ref text) if text == "[Alice] Hello")
        );
        for (parts, expected) in [
            (vec![], vec![("text", "[Alice] ")]),
            (
                vec![
                    LlmContentPart::image("image"),
                    LlmContentPart::audio("audio"),
                ],
                vec![("text", "[Alice] "), ("image", "image"), ("audio", "audio")],
            ),
            (
                vec![
                    LlmContentPart::text("Hello"),
                    LlmContentPart::image("image"),
                ],
                vec![("text", "[Alice] Hello"), ("image", "image")],
            ),
            (
                vec![
                    LlmContentPart::image("image"),
                    LlmContentPart::text("Hello"),
                    LlmContentPart::audio("audio"),
                    LlmContentPart::text("later"),
                ],
                vec![
                    ("image", "image"),
                    ("text", "[Alice] Hello"),
                    ("audio", "audio"),
                    ("text", "later"),
                ],
            ),
            (
                vec![LlmContentPart::text(""), LlmContentPart::text("later")],
                vec![("text", "[Alice] "), ("text", "later")],
            ),
        ] {
            let mut message = LlmMessage::parts(LlmMessageRole::Tool, parts);
            message.tool_call_id = Some("call-1".into());
            message.prepend_text_prefix("[Alice] ");
            let LlmMessageContent::Parts(parts) = &message.content else {
                panic!("parts must remain parts")
            };
            let actual: Vec<_> = parts
                .iter()
                .map(|part| match part {
                    LlmContentPart::Text { text } => ("text", text.as_str()),
                    LlmContentPart::Image { url } => ("image", url.as_str()),
                    LlmContentPart::Audio { url } => ("audio", url.as_str()),
                    LlmContentPart::File { url, .. } => ("file", url.as_str()),
                })
                .collect();
            assert_eq!(actual, expected);
            assert_eq!(message.role, LlmMessageRole::Tool);
            assert_eq!(message.tool_call_id.as_deref(), Some("call-1"));
        }
    }
    struct FixtureDriver(&'static str);

    #[async_trait]
    impl ChatDriver for FixtureDriver {
        async fn chat_completion_stream(
            &self,
            _: &ProviderEndpoint,
            _: Vec<LlmMessage>,
            _: &LlmCallConfig,
        ) -> Result<LlmResponseStream> {
            Ok(Box::pin(futures::stream::iter([
                Ok(LlmStreamEvent::TextDelta(self.0.into())),
                Ok(LlmStreamEvent::Done(Box::default())),
            ])))
        }
        async fn list_models(&self, _: &ProviderEndpoint) -> Result<Option<Vec<DiscoveredModel>>> {
            Ok(Some(vec![DiscoveredModel {
                model_id: self.0.into(),
                display_name: None,
                created_at: None,
                owned_by: None,
                capabilities: vec!["chat".into()],
                discovered_profile: None,
            }]))
        }
        async fn compact(
            &self,
            _: &ProviderEndpoint,
            request: CompactRequest,
        ) -> Result<Option<CompactResponse>> {
            Ok(Some(CompactResponse {
                output: vec![crate::compact::CompactOutputItem::Compaction {
                    encrypted_content: request.model,
                }],
                usage: None,
            }))
        }
        fn supports_compact(&self) -> bool {
            true
        }
        fn supports_stateful_responses(&self) -> bool {
            true
        }
        fn effective_context_window(&self, model: &str) -> Option<usize> {
            (model == "known").then_some(12345)
        }
        fn supports_parallel_tool_calls(&self, model: &str) -> bool {
            model == "known"
        }
    }

    fn compact_fixture() -> CompactRequest {
        CompactRequest {
            reasoning_state: None,
            model: "compact-model".into(),
            input: vec![],
            previous_response_id: None,
            instructions: None,
        }
    }

    #[tokio::test]
    async fn default_and_boxed_drivers_preserve_optional_operations_and_model_capabilities() {
        struct DefaultDriver;
        #[async_trait]
        impl ChatDriver for DefaultDriver {
            async fn chat_completion_stream(
                &self,
                _: &ProviderEndpoint,
                _: Vec<LlmMessage>,
                _: &LlmCallConfig,
            ) -> Result<LlmResponseStream> {
                Ok(Box::pin(futures::stream::empty()))
            }
        }
        let endpoint = ProviderEndpoint::default();
        assert!(!DefaultDriver.supports_compact());
        assert!(!DefaultDriver.supports_stateful_responses());
        assert!(!DefaultDriver.supports_parallel_tool_calls("known"));
        assert_eq!(DefaultDriver.effective_context_window("known"), None);
        assert!(
            DefaultDriver
                .list_models(&endpoint)
                .await
                .unwrap()
                .is_none()
        );
        assert!(
            DefaultDriver
                .compact(&endpoint, compact_fixture())
                .await
                .unwrap()
                .is_none()
        );
        let boxed: BoxedChatDriver = Box::new(FixtureDriver("boxed"));
        assert!(boxed.supports_compact());
        assert!(boxed.supports_stateful_responses());
        for (model, expected) in [("known", true), ("unknown", false)] {
            assert_eq!(boxed.supports_parallel_tool_calls(model), expected);
            assert_eq!(
                boxed.effective_context_window(model),
                expected.then_some(12345)
            );
        }
        assert_eq!(
            boxed
                .chat_completion(&endpoint, vec![], &bare_call_config())
                .await
                .unwrap()
                .text,
            "boxed"
        );
    }

    #[tokio::test]
    async fn registry_replacement_changes_factory_and_preserves_other_descriptors() {
        let mut registry = DriverRegistry::new();
        assert!(registry.registered_providers().is_empty());
        registry.register(DriverId::LlmSim, |_| Box::new(FixtureDriver("first")));
        registry.register_descriptor(DriverDescriptor {
            display_name: "OpenAI custom".into(),
            services: vec![ServiceKind::Chat, ServiceKind::Realtime],
            ..DriverDescriptor::chat_only(DriverId::OpenAI, |_| Box::new(FixtureDriver("other")))
        });
        let config = ProviderConfig::new(DriverId::LlmSim);
        let endpoint = ProviderEndpoint::default();
        assert_eq!(
            registry
                .create_chat_driver(&config)
                .unwrap()
                .chat_completion(&endpoint, vec![], &bare_call_config())
                .await
                .unwrap()
                .text,
            "first"
        );
        registry.register_or_replace(DriverId::LlmSim, |_| Box::new(FixtureDriver("replacement")));
        assert_eq!(
            registry
                .create_chat_driver(&config)
                .unwrap()
                .chat_completion(&endpoint, vec![], &bare_call_config())
                .await
                .unwrap()
                .text,
            "replacement"
        );
        assert!(registry.has_driver(&DriverId::LlmSim));
        assert!(!registry.has_driver(&DriverId::Anthropic));
        assert_eq!(
            registry.providers_for(ServiceKind::Realtime),
            vec![DriverId::OpenAI]
        );
        let mut chat = registry.providers_for(ServiceKind::Chat);
        chat.sort_by_key(|id| id.to_string());
        assert_eq!(chat, vec![DriverId::LlmSim, DriverId::OpenAI]);
        assert!(registry.supports(&DriverId::OpenAI, ServiceKind::Realtime));
        assert!(!registry.supports(&DriverId::LlmSim, ServiceKind::Realtime));
        assert!(!registry.supports(&DriverId::Gemini, ServiceKind::Chat));
        assert_eq!(
            registry.descriptor(&DriverId::OpenAI).unwrap().display_name,
            "OpenAI custom"
        );
        assert_eq!(
            registry
                .create_chat_driver(
                    &ProviderConfig::new(DriverId::OpenAI).with_api_key("synthetic-key")
                )
                .unwrap()
                .chat_completion(&endpoint, vec![], &bare_call_config())
                .await
                .unwrap()
                .text,
            "other"
        );
        let defaults = DriverDescriptor::chat_only(DriverId::Anthropic, |_| {
            Box::new(FixtureDriver("default"))
        });
        assert_eq!(defaults.display_name, "anthropic");
        let sim = registry.descriptor(&DriverId::LlmSim).unwrap();
        assert!(sim.credential_schema.fields.is_empty());
        assert_eq!(sim.services, vec![ServiceKind::Chat]);
        assert!(sim.chat.is_some());
        let real = registry.descriptor(&DriverId::OpenAI).unwrap();
        assert_eq!(real.credential_schema.fields.len(), 1);
        assert_eq!(real.credential_schema.fields[0].name, "api_key");
        assert!(real.credential_schema.fields[0].required);
        assert!(registry.descriptor(&DriverId::Gemini).is_none());
    }

    #[test]
    #[should_panic(expected = "already registered")]
    fn duplicate_registration_rejects_an_existing_driver() {
        let mut registry = DriverRegistry::new();
        registry.register(DriverId::OpenAI, |_| Box::new(FixtureDriver("first")));
        registry.register(DriverId::OpenAI, |_| Box::new(FixtureDriver("second")));
    }

    #[tokio::test]
    async fn factory_receives_complete_config_and_external_metadata_auth_remains_keyless() {
        let seen = Arc::new(std::sync::Mutex::new(Vec::new()));
        let capture = seen.clone();
        let mut registry = DriverRegistry::new();
        registry.register_external("CUSTOM", move |config| {
            capture.lock().unwrap().push(config.clone());
            Box::new(FixtureDriver("external"))
        });
        let metadata = ProviderMetadata {
            refresh_token: Some("refresh".into()),
            account_id: Some("account".into()),
            extra: Some(serde_json::json!({"region":"west"})),
        };
        for key in [None, Some("synthetic-key")] {
            let mut config =
                ProviderConfig::for_provider("connection", DriverId::external("custom"))
                    .with_base_url("https://gateway.example/v1")
                    .with_metadata(metadata.clone());
            if let Some(key) = key {
                config = config.with_api_key(key);
            }
            let response = registry
                .create_chat_driver(&config)
                .unwrap()
                .chat_completion(&ProviderEndpoint::default(), vec![], &bare_call_config())
                .await
                .unwrap();
            assert_eq!(response.text, "external");
            let received = seen.lock().unwrap().pop().unwrap();
            assert_eq!(received.provider.as_str(), "connection");
            assert_eq!(received.provider_type, DriverId::external("custom"));
            assert_eq!(received.api_key.as_deref(), key);
            assert_eq!(received.credential("api_key"), key);
            assert_eq!(received.credentials.len(), usize::from(key.is_some()));
            assert_eq!(
                received.base_url.as_deref(),
                Some("https://gateway.example/v1")
            );
            assert_eq!(received.metadata, metadata);
        }
        assert!(
            registry
                .descriptor(&DriverId::external("custom"))
                .unwrap()
                .credential_schema
                .fields
                .is_empty()
        );
    }

    #[test]
    fn registry_distinguishes_missing_driver_from_missing_chat_service() {
        let mut registry = DriverRegistry::new();
        assert!(
            matches!(registry.create_chat_driver(&ProviderConfig::new(DriverId::Anthropic)), Err(AgentLoopError::DriverNotRegistered(id)) if id == "anthropic")
        );
        registry.register_descriptor(DriverDescriptor {
            id: DriverId::external("embeddings-only"),
            display_name: "Embeddings Only".into(),
            services: vec![ServiceKind::Embeddings],
            credential_schema: CredentialFormSchema::empty(),
            oauth: None,
            chat: None,
            embeddings: None,
        });
        match registry
            .create_chat_driver(&ProviderConfig::new(DriverId::external("embeddings-only")))
        {
            Err(AgentLoopError::Llm(error)) => assert_eq!(
                error.message,
                "Provider driver 'embeddings-only' does not implement the chat service."
            ),
            _ => panic!("expected a missing-chat-service error"),
        }
    }

    #[tokio::test]
    async fn credential_gate_rejects_every_io_operation_before_dispatch() {
        struct ForbiddenDriver;
        #[async_trait]
        impl ChatDriver for ForbiddenDriver {
            async fn chat_completion_stream(
                &self,
                _: &ProviderEndpoint,
                _: Vec<LlmMessage>,
                _: &LlmCallConfig,
            ) -> Result<LlmResponseStream> {
                panic!("unauthenticated stream dispatch")
            }
            async fn list_models(
                &self,
                _: &ProviderEndpoint,
            ) -> Result<Option<Vec<DiscoveredModel>>> {
                panic!("unauthenticated model dispatch")
            }
            async fn compact(
                &self,
                _: &ProviderEndpoint,
                _: CompactRequest,
            ) -> Result<Option<CompactResponse>> {
                panic!("unauthenticated compact dispatch")
            }
        }
        let mut registry = DriverRegistry::new();
        registry.register(DriverId::OpenAI, |config| {
            if config.api_key.is_some() {
                Box::new(FixtureDriver("authenticated"))
            } else {
                Box::new(ForbiddenDriver)
            }
        });
        let driver = registry
            .create_chat_driver(&ProviderConfig::new(DriverId::OpenAI))
            .unwrap();
        let endpoint = ProviderEndpoint::default();
        let stream_error = match driver
            .chat_completion_stream(&endpoint, vec![], &bare_call_config())
            .await
        {
            Err(error) => error,
            Ok(_) => panic!("expected authentication error"),
        };
        for error in [
            stream_error,
            driver
                .chat_completion(&endpoint, vec![], &bare_call_config())
                .await
                .unwrap_err(),
            driver.list_models(&endpoint).await.unwrap_err(),
            driver
                .compact(&endpoint, compact_fixture())
                .await
                .unwrap_err(),
        ] {
            assert_eq!(error.llm_error_kind(), Some(LlmErrorKind::Authentication));
            assert_eq!(
                error.to_string(),
                "LLM error: API key is required. Configure the API key in provider settings."
            );
        }
        let driver = registry
            .create_chat_driver(
                &ProviderConfig::new(DriverId::OpenAI).with_api_key("synthetic-key"),
            )
            .unwrap();
        assert_eq!(
            driver
                .chat_completion(&endpoint, vec![], &bare_call_config())
                .await
                .unwrap()
                .text,
            "authenticated"
        );
        assert_eq!(
            driver.list_models(&endpoint).await.unwrap().unwrap()[0].model_id,
            "authenticated"
        );
        assert_eq!(
            serde_json::to_value(
                driver
                    .compact(&endpoint, compact_fixture())
                    .await
                    .unwrap()
                    .unwrap()
                    .output
            )
            .unwrap(),
            serde_json::json!([{"type":"compaction","encrypted_content":"compact-model"}])
        );
    }

    #[tokio::test]
    async fn request_options_preserve_calls_and_apply_headers_and_diagnostics_independently() {
        struct CapturingDriver(Arc<std::sync::Mutex<Vec<LlmCallConfig>>>);
        impl CapturingDriver {
            fn capture(
                &self,
                endpoint: &ProviderEndpoint,
                messages: &[LlmMessage],
                config: &LlmCallConfig,
            ) {
                assert_eq!(
                    endpoint.url("probe").as_deref(),
                    Some("https://gateway.example/v1/probe")
                );
                assert_eq!(messages.len(), 1);
                assert_eq!(messages[0].role, LlmMessageRole::User);
                assert_eq!(messages[0].content_as_text(), "request text");
                self.0.lock().unwrap().push(config.clone());
            }
        }
        #[async_trait]
        impl ChatDriver for CapturingDriver {
            async fn chat_completion_stream(
                &self,
                endpoint: &ProviderEndpoint,
                messages: Vec<LlmMessage>,
                config: &LlmCallConfig,
            ) -> Result<LlmResponseStream> {
                self.capture(endpoint, &messages, config);
                FixtureDriver("stream")
                    .chat_completion_stream(endpoint, messages, config)
                    .await
            }
            async fn chat_completion(
                &self,
                endpoint: &ProviderEndpoint,
                messages: Vec<LlmMessage>,
                config: &LlmCallConfig,
            ) -> Result<LlmResponse> {
                self.capture(endpoint, &messages, config);
                FixtureDriver("completion")
                    .chat_completion(endpoint, messages, config)
                    .await
            }
        }
        let provider = crate::Provider::new("fixture", FixtureDriver("endpoint"))
            .base_url("https://gateway.example/v1");
        for (headers, diagnostics) in [(false, false), (true, false), (false, true), (true, true)] {
            let seen = Arc::new(std::sync::Mutex::new(Vec::new()));
            let options = crate::provider::ProviderRequestOptions {
                headers: if headers {
                    vec![crate::provider::ProviderRequestHeader {
                        name: "x-base".into(),
                        value: "connection".into(),
                    }]
                } else {
                    vec![]
                },
                cache_diagnostics: diagnostics,
            };
            let driver =
                RequestOptionsDriver::wrap(Box::new(CapturingDriver(seen.clone())), &options);
            let mut config = bare_call_config();
            config.model = "requested-model".into();
            config.temperature = Some(0.25);
            config.max_tokens = Some(42);
            config
                .metadata
                .insert("session_id".into(), "session-one".into());
            config.previous_response_id = Some("response-one".into());
            config.extra_headers = vec![("x-base".into(), "original".into())];
            config.cache_diagnostics = Some(CacheDiagnosticsConfig {
                enabled: false,
                previous_message_id: Some("existing".into()),
            });
            let mut stream = driver
                .chat_completion_stream(
                    provider.endpoint(),
                    vec![LlmMessage::text(LlmMessageRole::User, "request text")],
                    &config,
                )
                .await
                .unwrap();
            use futures::StreamExt;
            assert!(
                matches!(stream.next().await.unwrap().unwrap(), LlmStreamEvent::TextDelta(text) if text == "stream")
            );
            assert!(matches!(
                stream.next().await.unwrap().unwrap(),
                LlmStreamEvent::Done(_)
            ));
            assert!(stream.next().await.is_none());
            assert_eq!(
                driver
                    .chat_completion(
                        provider.endpoint(),
                        vec![LlmMessage::text(LlmMessageRole::User, "request text")],
                        &config
                    )
                    .await
                    .unwrap()
                    .text,
                "completion"
            );
            let mut expected_headers = vec![("x-base".into(), "original".into())];
            if headers {
                expected_headers.push(("x-base".into(), "connection".into()));
            }
            let observed = seen.lock().unwrap();
            assert_eq!(observed.len(), 2);
            for received in observed.iter() {
                assert_eq!(received.extra_headers, expected_headers);
                let diagnostic = received.cache_diagnostics.as_ref().unwrap();
                assert_eq!(diagnostic.enabled, diagnostics);
                assert_eq!(
                    diagnostic.previous_message_id.as_deref(),
                    Some(if diagnostics {
                        "response-one"
                    } else {
                        "existing"
                    })
                );
                assert_eq!(received.model, "requested-model");
                assert_eq!(received.temperature, Some(0.25));
                assert_eq!(received.max_tokens, Some(42));
                assert_eq!(received.metadata, config.metadata);
                assert_eq!(received.previous_response_id, config.previous_response_id);
            }
            assert_eq!(
                config.extra_headers,
                vec![("x-base".into(), "original".into())]
            );
            assert!(!config.cache_diagnostics.as_ref().unwrap().enabled);
            assert_eq!(
                config
                    .cache_diagnostics
                    .as_ref()
                    .unwrap()
                    .previous_message_id
                    .as_deref(),
                Some("existing")
            );
        }
        let options = crate::provider::ProviderRequestOptions {
            headers: vec![],
            cache_diagnostics: true,
        };
        let wrapped = RequestOptionsDriver::wrap(Box::new(FixtureDriver("forwarded")), &options);
        assert!(wrapped.supports_compact());
        assert!(wrapped.supports_stateful_responses());
        for (model, expected) in [("known", true), ("unknown", false)] {
            assert_eq!(wrapped.supports_parallel_tool_calls(model), expected);
            assert_eq!(
                wrapped.effective_context_window(model),
                expected.then_some(12345)
            );
        }
        assert_eq!(
            wrapped
                .list_models(provider.endpoint())
                .await
                .unwrap()
                .unwrap()[0]
                .model_id,
            "forwarded"
        );
        assert_eq!(
            serde_json::to_value(
                wrapped
                    .compact(provider.endpoint(), compact_fixture())
                    .await
                    .unwrap()
                    .unwrap()
                    .output
            )
            .unwrap(),
            serde_json::json!([{"type":"compaction","encrypted_content":"compact-model"}])
        );
    }
}
