// LLM Driver Abstractions
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

use crate::error::{AgentLoopError, Result};
use crate::openresponses_protocol::{CompactRequest, CompactResponse};
use crate::runtime_agent::RuntimeAgent;
use crate::tool_types::{ToolCall, ToolDefinition};
use async_trait::async_trait;
use chrono::{DateTime, Utc};
use futures::Stream;
use std::collections::HashMap;
use std::pin::Pin;
use std::sync::Arc;

// ============================================================================
// ChatDriver Trait
// ============================================================================

/// Type alias for the LLM response stream
pub type LlmResponseStream = Pin<Box<dyn Stream<Item = Result<LlmStreamEvent>> + Send>>;

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
    /// Streaming completed
    Done(Box<LlmCompletionMetadata>),
    /// Error during streaming
    Error(String),
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
    /// Structured profile built from provider API metadata (capabilities, limits).
    /// Populated by drivers that return rich model metadata (e.g., Anthropic /v1/models).
    pub discovered_profile: Option<crate::llm_models::LlmModelProfile>,
}

/// Metadata about LLM completion
///
/// Contains token usage and completion information from the LLM response.
/// Cache token fields are provider-specific:
/// - OpenAI: `cache_read_tokens` from prompt_tokens_details.cached_tokens
/// - Anthropic: `cache_read_tokens` from cache_read_input_tokens,
///   `cache_creation_tokens` from cache_creation_input_tokens
#[derive(Debug, Clone, Default)]
pub struct LlmCompletionMetadata {
    /// Total tokens used
    pub total_tokens: Option<u32>,
    /// Prompt tokens
    pub prompt_tokens: Option<u32>,
    /// Completion tokens
    pub completion_tokens: Option<u32>,
    /// Tokens read from cache (reduces cost)
    pub cache_read_tokens: Option<u32>,
    /// Tokens written to cache (Anthropic-specific)
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
        messages: Vec<LlmMessage>,
        config: &LlmCallConfig,
    ) -> Result<LlmResponseStream>;

    /// Call the LLM without streaming (convenience method)
    async fn chat_completion(
        &self,
        messages: Vec<LlmMessage>,
        config: &LlmCallConfig,
    ) -> Result<LlmResponse> {
        use futures::StreamExt;

        let mut stream = self.chat_completion_stream(messages, config).await?;
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
                LlmStreamEvent::Done(meta) => metadata = *meta,
                LlmStreamEvent::Error(err) => return Err(crate::error::AgentLoopError::llm(err)),
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
    async fn list_models(&self) -> Result<Option<Vec<DiscoveredModel>>> {
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
    async fn compact(&self, _request: CompactRequest) -> Result<Option<CompactResponse>> {
        // Default: not supported
        Ok(None)
    }
}

/// Implement ChatDriver for `Box<dyn ChatDriver>` to allow dynamic dispatch
#[async_trait]
impl ChatDriver for Box<dyn ChatDriver> {
    async fn chat_completion_stream(
        &self,
        messages: Vec<LlmMessage>,
        config: &LlmCallConfig,
    ) -> Result<LlmResponseStream> {
        (**self).chat_completion_stream(messages, config).await
    }

    async fn chat_completion(
        &self,
        messages: Vec<LlmMessage>,
        config: &LlmCallConfig,
    ) -> Result<LlmResponse> {
        (**self).chat_completion(messages, config).await
    }

    async fn list_models(&self) -> Result<Option<Vec<DiscoveredModel>>> {
        (**self).list_models().await
    }

    fn supports_compact(&self) -> bool {
        (**self).supports_compact()
    }

    async fn compact(&self, request: CompactRequest) -> Result<Option<CompactResponse>> {
        (**self).compact(request).await
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
    pub phase: Option<crate::message::ExecutionPhase>,
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

/// OpenRouter model fallback and provider routing controls.
///
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
}

impl OpenRouterRoutingConfig {
    pub fn is_empty(&self) -> bool {
        self.models.is_empty() && self.route.is_none() && self.provider.is_none()
    }

    /// Build an ordered model-fallback routing config.
    pub fn fallback_models(models: impl IntoIterator<Item = impl Into<String>>) -> Self {
        let models = models.into_iter().map(Into::into).collect::<Vec<_>>();
        let route = (!models.is_empty()).then_some(OpenRouterRoute::Fallback);
        Self {
            models,
            route,
            provider: None,
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

/// Configuration for an LLM call
#[derive(Debug, Clone)]
pub struct LlmCallConfig {
    pub model: String,
    pub temperature: Option<f32>,
    pub max_tokens: Option<u32>,
    pub tools: Vec<ToolDefinition>,
    /// Reasoning effort level (for models that support it: low, medium, high)
    pub reasoning_effort: Option<String>,
    /// Metadata to send with the API request for tracking and debugging.
    /// Keys and values are strings. Both OpenAI and Anthropic support metadata fields.
    /// Typically includes: session_id, agent_id, org_id, turn_id, exec_id.
    pub metadata: HashMap<String, String>,
    /// Previous response ID for stateful continuation (OpenAI Responses API).
    /// When set, the provider can skip re-encoding cached context.
    pub previous_response_id: Option<String>,
    /// Tool search configuration for deferred tool loading
    pub tool_search: Option<ToolSearchConfig>,
    /// Prompt caching configuration for provider-specific cache controls.
    pub prompt_cache: Option<PromptCacheConfig>,
    /// OpenRouter-only model fallback and provider routing controls.
    pub openrouter_routing: Option<OpenRouterRoutingConfig>,
}

impl From<&RuntimeAgent> for LlmCallConfig {
    fn from(runtime_agent: &RuntimeAgent) -> Self {
        Self {
            model: runtime_agent.model.clone(),
            temperature: runtime_agent.temperature,
            max_tokens: runtime_agent.max_tokens,
            tools: runtime_agent.tools.clone(),
            reasoning_effort: None, // Set by ReasonAtom from user message controls
            metadata: HashMap::new(), // Set by ReasonAtom with session/agent context
            previous_response_id: None,
            tool_search: runtime_agent.tool_search.clone(),
            prompt_cache: runtime_agent.prompt_cache.clone(),
            openrouter_routing: None,
        }
    }
}

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
/// Use `from(&runtime_agent)` to start building from a RuntimeAgent, then chain
/// methods like `reasoning_effort()`, `temperature()`, etc. Call `build()`
/// to get the final config.
///
/// # Example
///
/// ```ignore
/// use everruns_core::llm::LlmCallConfigBuilder;
/// use everruns_core::runtime_agent::RuntimeAgent;
///
/// let runtime_agent = RuntimeAgent::new("You are helpful", "gpt-4o");
/// let llm_config = LlmCallConfigBuilder::from(&runtime_agent)
///     .reasoning_effort("high")
///     .temperature(0.7)
///     .build();
/// ```
pub struct LlmCallConfigBuilder {
    config: LlmCallConfig,
}

impl LlmCallConfigBuilder {
    /// Start building from a RuntimeAgent
    pub fn from(runtime_agent: &RuntimeAgent) -> Self {
        Self {
            config: LlmCallConfig::from(runtime_agent),
        }
    }

    /// Set reasoning effort level (for models that support it: low, medium, high)
    pub fn reasoning_effort(mut self, effort: impl Into<String>) -> Self {
        self.config.reasoning_effort = Some(effort.into());
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

    /// Build the configuration
    pub fn build(self) -> LlmCallConfig {
        self.config
    }
}

// ============================================================================
// Conversion from Message
// ============================================================================

impl From<&crate::message::Message> for LlmMessage {
    /// Convert a Message to LlmMessage (text-only, images become placeholders)
    ///
    /// This conversion is suitable for messages without images or when image
    /// resolution is not available. For multimodal messages, use
    /// `LlmMessage::from_message_with_images()` instead.
    fn from(msg: &crate::message::Message) -> Self {
        let role = match msg.role {
            crate::message::MessageRole::System => LlmMessageRole::System,
            crate::message::MessageRole::User => LlmMessageRole::User,
            crate::message::MessageRole::Agent => LlmMessageRole::Assistant,
            crate::message::MessageRole::ToolResult => LlmMessageRole::Tool,
        };

        // Convert tool calls from ContentPart format to ToolCall format
        let tool_calls: Vec<ToolCall> = msg
            .tool_calls()
            .into_iter()
            .map(|tc| ToolCall {
                id: tc.id.clone(),
                name: tc.name.clone(),
                arguments: tc.arguments.clone(),
            })
            .collect();

        LlmMessage {
            role,
            content: LlmMessageContent::Text(msg.content_to_llm_string()),
            tool_calls: if tool_calls.is_empty() {
                None
            } else {
                Some(tool_calls)
            },
            tool_call_id: msg.tool_call_id().map(|s| s.to_string()),
            phase: msg.phase,
            thinking: msg.thinking.clone(),
            thinking_signature: msg.thinking_signature.clone(),
        }
    }
}

// ============================================================================
// Message Conversion with Images
// ============================================================================

use crate::traits::ResolvedImage;
use uuid::Uuid;

impl LlmMessage {
    /// Convert a Message to LlmMessage with resolved images
    ///
    /// This method handles multimodal messages by converting:
    /// - `text` content parts → `LlmContentPart::Text`
    /// - `image` content parts → `LlmContentPart::Image` (data URL)
    /// - `image_file` content parts → `LlmContentPart::Image` (resolved to data URL)
    /// - `tool_call` content parts → extracted to `tool_calls` field
    /// - `tool_result` content parts → text representation
    ///
    /// # Provider-specific formatting
    ///
    /// The `LlmContentPart::Image` uses data URLs which are converted by each provider:
    /// - **OpenAI**: `{ "type": "image_url", "image_url": { "url": "data:..." } }`
    /// - **Anthropic**: `{ "type": "image", "source": { "type": "base64", ... } }`
    ///
    /// # Arguments
    ///
    /// * `msg` - The message to convert
    /// * `resolved_images` - Pre-resolved images keyed by image_id
    pub fn from_message_with_images(
        msg: &crate::message::Message,
        resolved_images: &HashMap<Uuid, ResolvedImage>,
    ) -> Self {
        use crate::message::{ContentPart, MessageRole};

        let role = match msg.role {
            MessageRole::System => LlmMessageRole::System,
            MessageRole::User => LlmMessageRole::User,
            MessageRole::Agent => LlmMessageRole::Assistant,
            MessageRole::ToolResult => LlmMessageRole::Tool,
        };

        // Convert content parts to LlmContentParts
        let mut parts: Vec<LlmContentPart> = Vec::new();
        let mut tool_calls: Vec<ToolCall> = Vec::new();

        for part in &msg.content {
            match part {
                ContentPart::Text(t) => {
                    parts.push(LlmContentPart::Text {
                        text: t.text.clone(),
                    });
                }
                ContentPart::Image(img) => {
                    // Convert inline image to data URL
                    if let Some(url) = &img.url {
                        parts.push(LlmContentPart::Image { url: url.clone() });
                    } else if let (Some(base64), Some(media_type)) = (&img.base64, &img.media_type)
                    {
                        let data_url = format!("data:{};base64,{}", media_type, base64);
                        parts.push(LlmContentPart::Image { url: data_url });
                    }
                }
                ContentPart::ImageFile(img_file) => {
                    // Resolve image_file to actual image data
                    if let Some(resolved) = resolved_images.get(&img_file.image_id.uuid()) {
                        parts.push(LlmContentPart::Image {
                            url: resolved.to_data_url(),
                        });
                    } else {
                        // Image not found - add placeholder text
                        parts.push(LlmContentPart::Text {
                            text: format!("[Image not found: {}]", img_file.image_id),
                        });
                    }
                }
                ContentPart::ToolCall(tc) => {
                    // Extract tool calls to separate field (don't include in content)
                    tool_calls.push(ToolCall {
                        id: tc.id.clone(),
                        name: tc.name.clone(),
                        arguments: tc.arguments.clone(),
                    });
                }
                ContentPart::ToolResult(tr) => {
                    // Convert tool result to text representation
                    let text = if let Some(err) = &tr.error {
                        format!("Tool error: {}", err)
                    } else if let Some(res) = &tr.result {
                        serde_json::to_string(res).unwrap_or_else(|_| "{}".to_string())
                    } else {
                        "{}".to_string()
                    };
                    // Primary hard limit enforced by OutputHardLimitHook (EVE-225)
                    // at tool execution time. This backstop catches tool results
                    // that bypass ActAtom hooks (client-submitted, stored events).
                    let text = truncate_tool_result(text);
                    parts.push(LlmContentPart::Text { text });
                }
            }
        }

        // Determine content format
        let content = if parts.len() == 1 && matches!(&parts[0], LlmContentPart::Text { .. }) {
            // Single text part - use simple Text format
            if let LlmContentPart::Text { text } = &parts[0] {
                LlmMessageContent::Text(text.clone())
            } else {
                LlmMessageContent::Parts(parts)
            }
        } else if parts.is_empty() {
            // No content parts - use empty text
            LlmMessageContent::Text(String::new())
        } else {
            // Multiple parts or non-text - use Parts format
            LlmMessageContent::Parts(parts)
        };

        LlmMessage {
            role,
            content,
            tool_calls: if tool_calls.is_empty() {
                None
            } else {
                Some(tool_calls)
            },
            tool_call_id: msg.tool_call_id().map(|s| s.to_string()),
            phase: msg.phase,
            thinking: msg.thinking.clone(),
            thinking_signature: msg.thinking_signature.clone(),
        }
    }

    /// Check if a message contains image_file references that need resolution
    pub fn message_has_image_files(msg: &crate::message::Message) -> bool {
        msg.content.iter().any(|p| p.is_image_file())
    }

    /// Extract all image_file IDs from a message
    pub fn extract_image_file_ids(msg: &crate::message::Message) -> Vec<Uuid> {
        msg.content
            .iter()
            .filter_map(|p| match p {
                crate::message::ContentPart::ImageFile(f) => Some(f.image_id.uuid()),
                _ => None,
            })
            .collect()
    }
}

// ============================================================================
// Driver Factory Types
// ============================================================================

/// Provider type enumeration matching the database/contracts.
///
/// Built-in variants are compiled into everruns. [`ProviderType::External`]
/// (or [`ProviderType::external`]) identifies providers an embedder defines and
/// registers itself, keyed by their canonical wire id.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub enum ProviderType {
    /// OpenAI using Open Responses API (<https://www.openresponses.org/>)
    /// This is the recommended API for new projects.
    OpenAI,
    /// OpenRouter using the OpenAI-compatible Responses API.
    OpenRouter,
    /// Azure OpenAI using the Azure-hosted OpenAI v1 API.
    AzureOpenAI,
    /// OpenAI using Chat Completions API (for backward compatibility)
    /// Use this if you need the legacy /v1/chat/completions endpoint.
    OpenAICompletions,
    Anthropic,
    /// Google Gemini API
    Gemini,
    /// LLM simulator for testing (uses llmsim crate)
    LlmSim,
    /// AWS Bedrock Runtime (ConverseStream API)
    Bedrock,
    /// Embedder-defined provider identified by its canonical wire id.
    External(Arc<str>),
}

impl ProviderType {
    /// Construct an external provider type from its canonical id.
    ///
    /// The id is normalized to lowercase so registration and lookup match
    /// case-insensitively, consistent with built-in parsing.
    ///
    /// ```
    /// use everruns_core::llm_driver_registry::ProviderType;
    /// let p = ProviderType::external("OpenAI-Codex");
    /// assert_eq!(p.as_str(), "openai-codex");
    /// ```
    pub fn external(id: impl Into<Arc<str>>) -> Self {
        let id: Arc<str> = id.into();
        // Avoid reallocating when the id is already lowercase.
        if id.bytes().any(|b| b.is_ascii_uppercase()) {
            ProviderType::External(Arc::from(id.to_lowercase().as_str()))
        } else {
            ProviderType::External(id)
        }
    }

    /// Canonical string identifier for this provider.
    pub fn as_str(&self) -> &str {
        match self {
            ProviderType::OpenAI => "openai",
            ProviderType::OpenRouter => "openrouter",
            ProviderType::AzureOpenAI => "azure_openai",
            ProviderType::OpenAICompletions => "openai_completions",
            ProviderType::Anthropic => "anthropic",
            ProviderType::Gemini => "gemini",
            ProviderType::LlmSim => "llmsim",
            ProviderType::Bedrock => "bedrock",
            ProviderType::External(id) => id.as_ref(),
        }
    }
}

impl std::str::FromStr for ProviderType {
    // Parsing never fails: unknown ids become `External`.
    type Err = std::convert::Infallible;

    fn from_str(s: &str) -> std::result::Result<Self, Self::Err> {
        // Normalize once: built-in matching and the External id share the same
        // lowercased form so casing variance never produces duplicate externals.
        let lower = s.to_lowercase();
        Ok(match lower.as_str() {
            "openai" => ProviderType::OpenAI,
            "openrouter" => ProviderType::OpenRouter,
            "azure_openai" => ProviderType::AzureOpenAI,
            "openai_completions" => ProviderType::OpenAICompletions,
            "anthropic" => ProviderType::Anthropic,
            "gemini" => ProviderType::Gemini,
            "llmsim" => ProviderType::LlmSim,
            "bedrock" => ProviderType::Bedrock,
            _ => ProviderType::External(Arc::from(lower.as_str())),
        })
    }
}

impl std::fmt::Display for ProviderType {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(self.as_str())
    }
}

/// Extra provider-specific authentication/metadata beyond an API key.
///
/// Built-in providers ignore this; embedder-defined ([`ProviderType::External`])
/// providers use it to carry OAuth tokens, account ids, or arbitrary extras
/// their driver factory needs.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct ProviderMetadata {
    /// OAuth refresh token, when the provider authenticates via OAuth.
    pub refresh_token: Option<String>,
    /// Provider-side account identifier, when required.
    pub account_id: Option<String>,
    /// Arbitrary extra fields the driver factory understands.
    pub extra: Option<serde_json::Value>,
}

/// Configuration for creating an LLM provider
#[derive(Debug, Clone)]
pub struct ProviderConfig {
    /// Type of provider
    pub provider_type: ProviderType,
    /// API key for authentication
    pub api_key: Option<String>,
    /// Base URL override (optional)
    pub base_url: Option<String>,
    /// Extra provider-specific metadata (OAuth tokens, account ids, etc.).
    pub metadata: ProviderMetadata,
}

impl ProviderConfig {
    /// Create a new provider config
    pub fn new(provider_type: ProviderType) -> Self {
        Self {
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

/// Everything a [`DriverFactory`] receives to build a driver instance.
///
/// Replaces the old `(api_key, base_url)` factory arguments so that
/// embedder-defined providers can receive richer auth via [`ProviderMetadata`]
/// without changing the factory signature again.
#[derive(Debug, Clone)]
pub struct DriverConfig {
    /// Provider type being created.
    pub provider_type: ProviderType,
    /// API key, when one is configured. `None` for keyless providers (LlmSim,
    /// or external providers that authenticate via [`ProviderMetadata`]).
    pub api_key: Option<String>,
    /// Base URL override, when configured.
    pub base_url: Option<String>,
    /// Extra provider-specific metadata.
    pub metadata: ProviderMetadata,
}

impl From<crate::llm_models::LlmProviderType> for ProviderType {
    fn from(provider_type: crate::llm_models::LlmProviderType) -> Self {
        use crate::llm_models::LlmProviderType;
        match provider_type {
            LlmProviderType::Openai => ProviderType::OpenAI,
            LlmProviderType::Openrouter => ProviderType::OpenRouter,
            LlmProviderType::AzureOpenai => ProviderType::AzureOpenAI,
            LlmProviderType::OpenaiCompletions => ProviderType::OpenAICompletions,
            LlmProviderType::Anthropic => ProviderType::Anthropic,
            LlmProviderType::Gemini => ProviderType::Gemini,
            LlmProviderType::LlmSim => ProviderType::LlmSim,
            LlmProviderType::Bedrock => ProviderType::Bedrock,
            LlmProviderType::External(id) => ProviderType::External(id),
        }
    }
}

impl From<&crate::traits::ModelWithProvider> for ProviderConfig {
    fn from(model: &crate::traits::ModelWithProvider) -> Self {
        Self {
            provider_type: model.provider_type.clone().into(),
            api_key: model.api_key.clone(),
            base_url: model.base_url.clone(),
            metadata: model.provider_metadata.clone().unwrap_or_default(),
        }
    }
}

/// Boxed LLM driver for dynamic dispatch
pub type BoxedChatDriver = Box<dyn ChatDriver>;

// ============================================================================
// Driver Registry
// ============================================================================

/// Factory function type for creating LLM drivers.
///
/// Receives a [`DriverConfig`] (provider type, optional key/base URL, and
/// provider metadata) and returns a boxed driver.
pub type DriverFactory = Arc<dyn Fn(&DriverConfig) -> BoxedChatDriver + Send + Sync>;

/// Registry for LLM drivers
///
/// Enables dependency inversion: provider crates (everruns-anthropic, everruns-openai)
/// register their drivers at startup. The core has no direct knowledge of implementations.
///
/// # Example
///
/// ```ignore
/// use everruns_core::llm_drivers::{DriverRegistry, ProviderType};
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
    factories: HashMap<ProviderType, DriverFactory>,
}

impl DriverRegistry {
    /// Create a new empty registry
    pub fn new() -> Self {
        Self {
            factories: HashMap::new(),
        }
    }

    /// Register a driver factory for a provider type.
    ///
    /// Panics if a factory is already registered for `provider_type` — silent
    /// overwrites hide double-registration bugs. Use
    /// [`Self::register_or_replace`] to overwrite intentionally.
    pub fn register<F>(&mut self, provider_type: impl Into<ProviderType>, factory: F)
    where
        F: Fn(&DriverConfig) -> BoxedChatDriver + Send + Sync + 'static,
    {
        let provider_type = provider_type.into();
        if self.factories.contains_key(&provider_type) {
            panic!(
                "driver already registered for provider '{provider_type}'; \
                 use register_or_replace to overwrite intentionally"
            );
        }
        self.factories.insert(provider_type, Arc::new(factory));
    }

    /// Register a driver factory, replacing any existing one for the provider.
    ///
    /// Use when overwriting is intentional (e.g. swapping in an `LlmSim` driver
    /// for tests). Prefer [`Self::register`] otherwise so duplicates surface.
    pub fn register_or_replace<F>(&mut self, provider_type: impl Into<ProviderType>, factory: F)
    where
        F: Fn(&DriverConfig) -> BoxedChatDriver + Send + Sync + 'static,
    {
        self.factories
            .insert(provider_type.into(), Arc::new(factory));
    }

    /// Register a driver factory for an embedder-defined external provider,
    /// keyed by its canonical id. The id is normalized to lowercase (via
    /// [`ProviderType::external`]) so it matches parsed lookups regardless of
    /// the casing stored in the database or sent on the wire.
    pub fn register_external<F>(&mut self, id: impl Into<Arc<str>>, factory: F)
    where
        F: Fn(&DriverConfig) -> BoxedChatDriver + Send + Sync + 'static,
    {
        self.register(ProviderType::external(id), factory);
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
        // API key is required for real built-in providers, but not for LlmSim
        // (testing) or External providers (which may use metadata-based auth).
        let requires_api_key = !matches!(
            config.provider_type,
            ProviderType::LlmSim | ProviderType::External(_)
        );
        if requires_api_key && config.api_key.is_none() {
            return Err(AgentLoopError::llm(
                "API key is required. Configure the API key in provider settings.",
            ));
        }

        // Look up the factory for this provider type
        let factory = self.factories.get(&config.provider_type).ok_or_else(|| {
            AgentLoopError::driver_not_registered(config.provider_type.to_string())
        })?;

        // Create the driver using the factory
        let driver_config = DriverConfig {
            provider_type: config.provider_type.clone(),
            api_key: config.api_key.clone(),
            base_url: config.base_url.clone(),
            metadata: config.metadata.clone(),
        };
        Ok(factory(&driver_config))
    }

    /// Check if a driver is registered for a provider type
    pub fn has_driver(&self, provider_type: &ProviderType) -> bool {
        self.factories.contains_key(provider_type)
    }

    /// Get the list of registered provider types
    pub fn registered_providers(&self) -> Vec<ProviderType> {
        self.factories.keys().cloned().collect()
    }
}

/// Maximum tool result size in bytes before truncation (64 KiB).
/// Defense-in-depth backstop for tool results that bypass ActAtom hooks
/// (e.g. client-submitted or stored events). The primary hard limit is
/// enforced by `OutputHardLimitHook` (EVE-225) at tool execution time.
const MAX_TOOL_RESULT_BYTES: usize = 64 * 1024;

const TRUNCATION_SUFFIX: &str =
    "\n\n[Output truncated — exceeded 64 KiB limit. Try quiet flags, pipes, or redirect to file.]";

fn truncate_tool_result(text: String) -> String {
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

    #[test]
    fn test_llm_call_config_builder_from_runtime_agent() {
        let runtime_agent = RuntimeAgent::new("You are helpful", "gpt-4o");
        let llm_config = LlmCallConfigBuilder::from(&runtime_agent).build();

        assert_eq!(llm_config.model, "gpt-4o");
        assert!(llm_config.reasoning_effort.is_none());
        assert!(llm_config.temperature.is_none());
        assert!(llm_config.max_tokens.is_none());
        assert!(llm_config.tools.is_empty());
        assert!(llm_config.metadata.is_empty());
    }

    #[test]
    fn test_llm_call_config_builder_with_metadata() {
        let runtime_agent = RuntimeAgent::new("You are helpful", "gpt-4o");
        let llm_config = LlmCallConfigBuilder::from(&runtime_agent)
            .with_metadata("session_id", "session_abc123")
            .with_metadata("agent_id", "agent_xyz789")
            .build();

        assert_eq!(
            llm_config.metadata.get("session_id"),
            Some(&"session_abc123".to_string())
        );
        assert_eq!(
            llm_config.metadata.get("agent_id"),
            Some(&"agent_xyz789".to_string())
        );
    }

    #[test]
    fn test_llm_call_config_builder_with_metadata_hashmap() {
        let runtime_agent = RuntimeAgent::new("You are helpful", "gpt-4o");
        let mut metadata = HashMap::new();
        metadata.insert("key1".to_string(), "value1".to_string());
        metadata.insert("key2".to_string(), "value2".to_string());

        let llm_config = LlmCallConfigBuilder::from(&runtime_agent)
            .metadata(metadata)
            .build();

        assert_eq!(llm_config.metadata.get("key1"), Some(&"value1".to_string()));
        assert_eq!(llm_config.metadata.get("key2"), Some(&"value2".to_string()));
    }

    #[test]
    fn test_llm_call_config_builder_with_reasoning_effort() {
        let runtime_agent = RuntimeAgent::new("You are helpful", "gpt-4o");
        let llm_config = LlmCallConfigBuilder::from(&runtime_agent)
            .reasoning_effort("high")
            .build();

        assert_eq!(llm_config.reasoning_effort, Some("high".to_string()));
    }

    #[test]
    fn test_llm_call_config_builder_with_all_options() {
        let runtime_agent = RuntimeAgent::new("You are helpful", "gpt-4o");
        let llm_config = LlmCallConfigBuilder::from(&runtime_agent)
            .model("claude-3-opus")
            .reasoning_effort("medium")
            .temperature(0.7)
            .max_tokens(1000)
            .build();

        assert_eq!(llm_config.model, "claude-3-opus");
        assert_eq!(llm_config.reasoning_effort, Some("medium".to_string()));
        assert_eq!(llm_config.temperature, Some(0.7));
        assert_eq!(llm_config.max_tokens, Some(1000));
    }

    #[test]
    fn test_llm_call_config_builder_with_openrouter_routing() {
        let runtime_agent = RuntimeAgent::new("You are helpful", "openai/gpt-5-mini");
        let routing = OpenRouterRoutingConfig::fallback_models([
            "openai/gpt-5-mini",
            "anthropic/claude-sonnet-4.5",
        ]);

        let llm_config = LlmCallConfigBuilder::from(&runtime_agent)
            .openrouter_routing(routing.clone())
            .build();

        assert_eq!(llm_config.openrouter_routing, Some(routing));
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
        assert_eq!(
            "openai".parse::<ProviderType>().unwrap(),
            ProviderType::OpenAI
        );
        assert_eq!(
            "openrouter".parse::<ProviderType>().unwrap(),
            ProviderType::OpenRouter
        );
        assert_eq!(
            "openai_completions".parse::<ProviderType>().unwrap(),
            ProviderType::OpenAICompletions
        );
        assert_eq!(
            "azure_openai".parse::<ProviderType>().unwrap(),
            ProviderType::AzureOpenAI
        );
        assert_eq!(
            "anthropic".parse::<ProviderType>().unwrap(),
            ProviderType::Anthropic
        );
        assert_eq!(
            "gemini".parse::<ProviderType>().unwrap(),
            ProviderType::Gemini
        );
        // Unknown ids parse to External rather than erroring.
        assert_eq!(
            "ollama".parse::<ProviderType>().unwrap(),
            ProviderType::external("ollama")
        );
        assert_eq!(
            "custom".parse::<ProviderType>().unwrap(),
            ProviderType::external("custom")
        );
    }

    #[test]
    fn test_external_provider_id_is_case_insensitive() {
        // Built-in matching and external normalization are both case-folding,
        // so the same id in different casing resolves to one provider.
        assert_eq!(
            "OpenAI".parse::<ProviderType>().unwrap(),
            ProviderType::OpenAI
        );
        assert_eq!(
            "Ollama".parse::<ProviderType>().unwrap(),
            "ollama".parse::<ProviderType>().unwrap()
        );
        assert_eq!(
            ProviderType::external("OpenAI-Codex").as_str(),
            "openai-codex"
        );
        // Registration and parsed lookup agree regardless of casing.
        assert_eq!(
            ProviderType::external("MyProvider"),
            "myprovider".parse::<ProviderType>().unwrap()
        );
    }

    #[test]
    fn test_provider_type_display() {
        assert_eq!(ProviderType::OpenAI.to_string(), "openai");
        assert_eq!(ProviderType::OpenRouter.to_string(), "openrouter");
        assert_eq!(ProviderType::AzureOpenAI.to_string(), "azure_openai");
        assert_eq!(
            ProviderType::OpenAICompletions.to_string(),
            "openai_completions"
        );
        assert_eq!(ProviderType::Anthropic.to_string(), "anthropic");
        assert_eq!(ProviderType::Gemini.to_string(), "gemini");
    }

    #[test]
    fn test_provider_config_builder() {
        let config = ProviderConfig::new(ProviderType::Anthropic)
            .with_api_key("test-key")
            .with_base_url("https://custom.api.com");

        assert_eq!(config.provider_type, ProviderType::Anthropic);
        assert_eq!(config.api_key, Some("test-key".to_string()));
        assert_eq!(config.base_url, Some("https://custom.api.com".to_string()));
    }

    #[test]
    fn test_driver_registry_requires_api_key() {
        // Register a mock factory
        let mut registry = DriverRegistry::new();
        registry.register(ProviderType::OpenAI, |_config| {
            // Return a mock driver - just need something that compiles
            struct MockDriver;
            #[async_trait]
            impl ChatDriver for MockDriver {
                async fn chat_completion_stream(
                    &self,
                    _messages: Vec<LlmMessage>,
                    _config: &LlmCallConfig,
                ) -> Result<LlmResponseStream> {
                    unimplemented!()
                }
            }
            Box::new(MockDriver)
        });

        // Driver without API key should fail
        let config = ProviderConfig::new(ProviderType::OpenAI);
        let result = registry.create_chat_driver(&config);
        assert!(result.is_err());

        // Driver with API key should succeed
        let config_with_key = ProviderConfig::new(ProviderType::OpenAI).with_api_key("test-key");
        let result = registry.create_chat_driver(&config_with_key);
        assert!(result.is_ok());
    }

    #[test]
    fn test_driver_registry_returns_error_for_unregistered_provider() {
        let registry = DriverRegistry::new();
        let config = ProviderConfig::new(ProviderType::Anthropic).with_api_key("test-key");

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

        assert!(!registry.has_driver(&ProviderType::OpenAI));
        assert!(!registry.has_driver(&ProviderType::Anthropic));

        registry.register(ProviderType::OpenAI, |_config| {
            struct MockDriver;
            #[async_trait]
            impl ChatDriver for MockDriver {
                async fn chat_completion_stream(
                    &self,
                    _messages: Vec<LlmMessage>,
                    _config: &LlmCallConfig,
                ) -> Result<LlmResponseStream> {
                    unimplemented!()
                }
            }
            Box::new(MockDriver)
        });

        assert!(registry.has_driver(&ProviderType::OpenAI));
        assert!(!registry.has_driver(&ProviderType::Anthropic));
    }

    #[test]
    fn test_register_external_and_create_driver_without_api_key() {
        struct MockDriver;
        #[async_trait]
        impl ChatDriver for MockDriver {
            async fn chat_completion_stream(
                &self,
                _messages: Vec<LlmMessage>,
                _config: &LlmCallConfig,
            ) -> Result<LlmResponseStream> {
                unimplemented!()
            }
        }

        let mut registry = DriverRegistry::new();
        registry.register_external("openai-codex", |config| {
            // External providers may authenticate via metadata, not an api_key.
            assert_eq!(config.provider_type, ProviderType::external("openai-codex"));
            Box::new(MockDriver)
        });

        assert!(registry.has_driver(&ProviderType::external("openai-codex")));

        // No api_key required for external providers.
        let config = ProviderConfig::new(ProviderType::external("openai-codex")).with_metadata(
            ProviderMetadata {
                refresh_token: Some("rt".into()),
                ..Default::default()
            },
        );
        assert!(registry.create_chat_driver(&config).is_ok());
    }

    #[test]
    #[should_panic(expected = "already registered")]
    fn test_register_duplicate_panics() {
        struct MockDriver;
        #[async_trait]
        impl ChatDriver for MockDriver {
            async fn chat_completion_stream(
                &self,
                _messages: Vec<LlmMessage>,
                _config: &LlmCallConfig,
            ) -> Result<LlmResponseStream> {
                unimplemented!()
            }
        }

        let mut registry = DriverRegistry::new();
        registry.register(ProviderType::OpenAI, |_config| Box::new(MockDriver));
        // Second registration for the same provider must panic.
        registry.register(ProviderType::OpenAI, |_config| Box::new(MockDriver));
    }

    #[test]
    fn test_register_or_replace_overwrites() {
        struct MockDriver;
        #[async_trait]
        impl ChatDriver for MockDriver {
            async fn chat_completion_stream(
                &self,
                _messages: Vec<LlmMessage>,
                _config: &LlmCallConfig,
            ) -> Result<LlmResponseStream> {
                unimplemented!()
            }
        }

        let mut registry = DriverRegistry::new();
        registry.register(ProviderType::LlmSim, |_config| Box::new(MockDriver));
        // Replacing intentionally must not panic.
        registry.register_or_replace(ProviderType::LlmSim, |_config| Box::new(MockDriver));
        assert!(registry.has_driver(&ProviderType::LlmSim));
    }

    // ========================================================================
    // Image resolution tests
    // ========================================================================

    use crate::{ContentPart, ImageFileContentPart, Message, MessageRole, TextContentPart};

    #[test]
    fn test_message_has_image_files_with_image_file() {
        let message = Message {
            id: uuid::Uuid::new_v4().into(),
            role: MessageRole::User,
            content: vec![
                ContentPart::Text(TextContentPart {
                    text: "Look at this image".to_string(),
                }),
                ContentPart::ImageFile(ImageFileContentPart {
                    image_id: uuid::Uuid::new_v4().into(),
                    filename: Some("test.png".to_string()),
                }),
            ],
            phase: None,
            thinking: None,
            thinking_signature: None,
            controls: None,
            metadata: None,
            external_actor: None,
            created_at: chrono::Utc::now(),
        };

        assert!(LlmMessage::message_has_image_files(&message));
    }

    #[test]
    fn test_message_has_image_files_without_image_file() {
        let message = Message {
            id: uuid::Uuid::new_v4().into(),
            role: MessageRole::User,
            content: vec![ContentPart::Text(TextContentPart {
                text: "Just text".to_string(),
            })],
            phase: None,
            thinking: None,
            thinking_signature: None,
            controls: None,
            metadata: None,
            external_actor: None,
            created_at: chrono::Utc::now(),
        };

        assert!(!LlmMessage::message_has_image_files(&message));
    }

    #[test]
    fn test_extract_image_file_ids() {
        let id1 = uuid::Uuid::new_v4();
        let id2 = uuid::Uuid::new_v4();

        let message = Message {
            id: uuid::Uuid::new_v4().into(),
            role: MessageRole::User,
            content: vec![
                ContentPart::Text(TextContentPart {
                    text: "Look at these images".to_string(),
                }),
                ContentPart::ImageFile(ImageFileContentPart {
                    image_id: id1.into(),
                    filename: Some("test1.png".to_string()),
                }),
                ContentPart::ImageFile(ImageFileContentPart {
                    image_id: id2.into(),
                    filename: Some("test2.png".to_string()),
                }),
            ],
            phase: None,
            thinking: None,
            thinking_signature: None,
            controls: None,
            metadata: None,
            external_actor: None,
            created_at: chrono::Utc::now(),
        };

        let ids = LlmMessage::extract_image_file_ids(&message);
        assert_eq!(ids.len(), 2);
        assert!(ids.contains(&id1));
        assert!(ids.contains(&id2));
    }

    #[test]
    fn test_from_message_with_images_text_only() {
        let message = Message {
            id: uuid::Uuid::new_v4().into(),
            role: MessageRole::User,
            content: vec![ContentPart::Text(TextContentPart {
                text: "Hello".to_string(),
            })],
            phase: None,
            thinking: None,
            thinking_signature: None,
            controls: None,
            metadata: None,
            external_actor: None,
            created_at: chrono::Utc::now(),
        };

        let resolved = std::collections::HashMap::new();
        let llm_message = LlmMessage::from_message_with_images(&message, &resolved);

        assert_eq!(llm_message.role, LlmMessageRole::User);
        match llm_message.content {
            LlmMessageContent::Text(text) => assert_eq!(text, "Hello"),
            _ => panic!("Expected text content"),
        }
    }

    #[test]
    fn test_from_message_with_images_resolved_image() {
        let image_id = uuid::Uuid::new_v4();
        let message = Message {
            id: uuid::Uuid::new_v4().into(),
            role: MessageRole::User,
            content: vec![
                ContentPart::Text(TextContentPart {
                    text: "Look at this".to_string(),
                }),
                ContentPart::ImageFile(ImageFileContentPart {
                    image_id: image_id.into(),
                    filename: Some("test.png".to_string()),
                }),
            ],
            phase: None,
            thinking: None,
            thinking_signature: None,
            controls: None,
            metadata: None,
            external_actor: None,
            created_at: chrono::Utc::now(),
        };

        let mut resolved = std::collections::HashMap::new();
        resolved.insert(
            image_id,
            crate::ResolvedImage::new("base64data", "image/png"),
        );

        let llm_message = LlmMessage::from_message_with_images(&message, &resolved);

        match &llm_message.content {
            LlmMessageContent::Parts(parts) => {
                assert_eq!(parts.len(), 2);
                // First part should be text
                assert!(matches!(&parts[0], LlmContentPart::Text { .. }));
                // Second part should be resolved image
                if let LlmContentPart::Image { url } = &parts[1] {
                    assert!(url.starts_with("data:image/png;base64,"));
                } else {
                    panic!("Expected image content part");
                }
            }
            _ => panic!("Expected parts content"),
        }
    }

    #[test]
    fn test_from_message_with_images_unresolved_image() {
        let image_id = uuid::Uuid::new_v4();
        let message = Message {
            id: uuid::Uuid::new_v4().into(),
            role: MessageRole::User,
            content: vec![ContentPart::ImageFile(ImageFileContentPart {
                image_id: image_id.into(),
                filename: Some("missing.png".to_string()),
            })],
            phase: None,
            thinking: None,
            thinking_signature: None,
            controls: None,
            metadata: None,
            external_actor: None,
            created_at: chrono::Utc::now(),
        };

        // Empty resolved map - image not found
        let resolved = std::collections::HashMap::new();
        let llm_message = LlmMessage::from_message_with_images(&message, &resolved);

        // Should have placeholder text for missing image
        // When there's only one part, it may return Text directly instead of Parts
        match &llm_message.content {
            LlmMessageContent::Text(text) => {
                assert!(text.contains("Image not found"));
            }
            LlmMessageContent::Parts(parts) => {
                assert_eq!(parts.len(), 1);
                if let LlmContentPart::Text { text } = &parts[0] {
                    assert!(text.contains("Image not found"));
                } else {
                    panic!("Expected text placeholder for missing image");
                }
            }
        }
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
}
