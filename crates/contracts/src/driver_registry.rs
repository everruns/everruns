// Chat Driver Abstractions
//
// This module encapsulates all abstractions needed to interact with LLM Providers:
// - ChatDriver trait and types for provider-agnostic LLM interactions
// - DriverRegistry for dynamic driver registration at startup
// - Message types for LLM calls
//
// Supports both simple text content and multipart content (text, images, audio).
//
// Credentials are supplied by the host through ProviderConfig; the registry
// does not read environment variables or own credential persistence.
// Vendor drivers depend on these contracts and register at the host boundary.

use crate::compact::{CompactRequest, CompactResponse};
use crate::credential_schema::CredentialFormSchema;
use crate::error::{AgentLoopError, LlmErrorKind, Result};
use crate::tool_types::{ToolCall, ToolDefinition};
use async_trait::async_trait;
use futures::Stream;
use std::collections::HashMap;
use std::pin::Pin;
use std::sync::Arc;

// ============================================================================
// ChatDriver Trait
// ============================================================================

/// Type alias for the LLM response stream
pub type LlmResponseStream = Pin<Box<dyn Stream<Item = Result<LlmStreamEvent>> + Send>>;

pub use crate::provider_managed::{ProviderCheckpointCandidate, ProviderOpaqueContext};

pub use crate::stream_error::LlmStreamError;

pub use crate::stream_event::{
    HostedToolCall, HostedToolCallStatus, LlmCompletionMetadata, LlmStreamEvent,
    disjoint_prompt_tokens,
};

// `DiscoveredModel` is the discovery module's own type; it lives there and is
// re-exported here so every existing path keeps working.
pub use crate::model_discovery::DiscoveredModel;

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
    /// Opt in on a supported provider/model, retaining the synchronous fallback.
    fn native_async_driver(
        &self,
        _model: &str,
        _tools: std::collections::BTreeMap<String, Option<serde_json::Value>>,
        _continuation: Option<crate::native_async::Delivery>,
    ) -> Option<Arc<dyn ChatDriver>> {
        None
    }
    /// Call the LLM with streaming response
    async fn chat_completion_stream(
        &self,
        endpoint: &crate::runtime_provider::ProviderEndpoint,
        messages: Vec<Message>,
        config: &LlmCallConfig,
    ) -> Result<LlmResponseStream>;

    /// Call the LLM without streaming (convenience method)
    async fn chat_completion(
        &self,
        endpoint: &crate::runtime_provider::ProviderEndpoint,
        messages: Vec<Message>,
        config: &LlmCallConfig,
    ) -> Result<LlmResponse> {
        // One folding loop for the whole runtime: the same rules (and the
        // same `config.limits`) apply whether a caller collects the stream
        // itself or takes the non-streaming path.
        //
        // The connect phase is inside the budget too. It is a provider round
        // trip that can hang before any event exists to collect, so bounding
        // only the fold would leave the whole point of the limit unbounded.
        let limits = config.limits;
        let (stream, spent) = crate::turn_collector::connect_within(
            &limits,
            self.chat_completion_stream(endpoint, messages, config),
        )
        .await?;
        Ok(
            crate::turn_collector::collect_turn(stream, &limits.after(spent), |_| {})
                .await?
                .into_response(),
        )
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
        messages: Vec<Message>,
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

    /// Whether this driver enforces [`LlmCallConfig::response_format`] natively
    /// for `model`. `false` (the default) makes the provider reject the call
    /// rather than silently drop the constraint.
    fn supports_response_format(&self, _model: &str) -> bool {
        false
    }

    /// Resolve a provider-managed history reduction request for this endpoint
    /// and model. A bound provider substitutes its captured real endpoint.
    fn provider_managed_reduction_option(
        &self,
        _endpoint: &crate::runtime_provider::ProviderEndpoint,
        _model: &str,
        _budget_tokens: usize,
    ) -> Option<(String, serde_json::Value)> {
        None
    }

    /// Return a stable fallback reason when the completed request configuration
    /// cannot safely use the provider-managed reduction option selected during
    /// turn assembly.
    fn provider_managed_reduction_fallback_reason(
        &self,
        _endpoint: &crate::runtime_provider::ProviderEndpoint,
        _config: &LlmCallConfig,
    ) -> Option<&'static str> {
        None
    }

    /// Validate provider-owned checkpoint context before the runtime replaces
    /// full raw history with a suffix-only load.
    fn validate_provider_opaque_context(&self, _context: &ProviderOpaqueContext) -> bool {
        true
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
    fn native_async_driver(
        &self,
        model: &str,
        tools: std::collections::BTreeMap<String, Option<serde_json::Value>>,
        continuation: Option<crate::native_async::Delivery>,
    ) -> Option<Arc<dyn ChatDriver>> {
        (**self).native_async_driver(model, tools, continuation)
    }
    async fn chat_completion_stream(
        &self,
        endpoint: &crate::runtime_provider::ProviderEndpoint,
        messages: Vec<Message>,
        config: &LlmCallConfig,
    ) -> Result<LlmResponseStream> {
        (**self)
            .chat_completion_stream(endpoint, messages, config)
            .await
    }

    async fn chat_completion(
        &self,
        endpoint: &crate::runtime_provider::ProviderEndpoint,
        messages: Vec<Message>,
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
        messages: Vec<Message>,
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

    fn supports_response_format(&self, model: &str) -> bool {
        (**self).supports_response_format(model)
    }

    fn provider_managed_reduction_option(
        &self,
        endpoint: &crate::runtime_provider::ProviderEndpoint,
        model: &str,
        budget_tokens: usize,
    ) -> Option<(String, serde_json::Value)> {
        (**self).provider_managed_reduction_option(endpoint, model, budget_tokens)
    }

    fn provider_managed_reduction_fallback_reason(
        &self,
        endpoint: &crate::runtime_provider::ProviderEndpoint,
        config: &LlmCallConfig,
    ) -> Option<&'static str> {
        (**self).provider_managed_reduction_fallback_reason(endpoint, config)
    }

    fn validate_provider_opaque_context(&self, context: &ProviderOpaqueContext) -> bool {
        (**self).validate_provider_opaque_context(context)
    }

    async fn compact(
        &self,
        endpoint: &crate::runtime_provider::ProviderEndpoint,
        request: CompactRequest,
    ) -> Result<Option<CompactResponse>> {
        (**self).compact(endpoint, request).await
    }
}

// The message types moved to `message` when this file outgrew what anyone can
// hold in their head; re-exported here so every existing path keeps working.
pub use crate::message::{
    LlmContentPart, Message, MessageContent, MessageRole, fold_system_messages,
};

// ============================================================================
// Configuration and Response Types
// ============================================================================

mod configuration;
pub use configuration::*;

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

// `LlmCallConfigBuilder` lives in `llm_call_config_builder.rs`.
pub use crate::llm_call_config_builder::LlmCallConfigBuilder;

// The Message->Message adapters (plain, with-images, and image-file
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
///
/// `#[non_exhaustive]`: construct with [`ProviderConfig::new`] or
/// [`ProviderConfig::for_provider`] and the `with_*` setters, so that adding a
/// connection-level field stays a non-breaking change.
#[derive(Clone)]
#[non_exhaustive]
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
        _messages: Vec<Message>,
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

    fn supports_response_format(&self, model: &str) -> bool {
        self.inner.supports_response_format(model)
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
    inner: Arc<dyn ChatDriver>,
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
            inner: Arc::from(driver),
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
    fn native_async_driver(
        &self,
        model: &str,
        tools: std::collections::BTreeMap<String, Option<serde_json::Value>>,
        continuation: Option<crate::native_async::Delivery>,
    ) -> Option<Arc<dyn ChatDriver>> {
        Some(Arc::new(Self {
            inner: self.inner.native_async_driver(model, tools, continuation)?,
            options: self.options.clone(),
        }))
    }
    async fn chat_completion_stream(
        &self,
        endpoint: &crate::runtime_provider::ProviderEndpoint,
        messages: Vec<Message>,
        config: &LlmCallConfig,
    ) -> Result<LlmResponseStream> {
        self.inner
            .chat_completion_stream(endpoint, messages, &self.apply(config))
            .await
    }

    async fn chat_completion(
        &self,
        endpoint: &crate::runtime_provider::ProviderEndpoint,
        messages: Vec<Message>,
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
        messages: Vec<Message>,
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

    fn supports_response_format(&self, model: &str) -> bool {
        self.inner.supports_response_format(model)
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
pub use crate::model_profile_data::ServiceKind;

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
    /// Environment variable that overrides this driver's default endpoint in
    /// standalone/dev use (e.g. `OPENAI_BASE_URL`), following the vendor's own
    /// SDK convention.
    ///
    /// Declared here rather than on the credential schema because the base URL
    /// is not a credential field: it lives on `ProviderConfig`, never in the
    /// stored credential document, and connectors share the schema type without
    /// having endpoints at all. `None` for drivers whose vendor defines no
    /// endpoint variable, or whose endpoint is part of the credential itself
    /// (Bedrock's region).
    pub base_url_env: Option<String>,
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
            base_url_env: None,
            services: vec![ServiceKind::Chat],
            oauth: None,
            chat: Some(Arc::new(factory)),
            embeddings: None,
            id,
        }
    }

    /// Declare the environment variable that overrides the default endpoint.
    pub fn with_base_url_env(mut self, base_url_env: impl Into<String>) -> Self {
        self.base_url_env = Some(base_url_env.into());
        self
    }

    /// Whether the driver declares the given service.
    pub fn supports(&self, service: ServiceKind) -> bool {
        self.services.contains(&service)
    }

    /// Every environment variable this driver declares, credential fields first
    /// in schema order and the endpoint override last.
    ///
    /// Drives actionable "set X or Y" messages and the published credential
    /// documentation, both of which would otherwise have to restate names the
    /// driver already owns.
    pub fn declared_env_vars(&self) -> Vec<String> {
        self.credential_schema
            .fields
            .iter()
            .flat_map(|field| field.env.iter().cloned())
            .chain(self.base_url_env.clone())
            .collect()
    }

    /// The declared endpoint override, when this driver declares one and it is
    /// set to a non-empty value in the given environment.
    pub fn base_url_from_env<F>(&self, lookup: F) -> Option<String>
    where
        F: Fn(&str) -> Option<String>,
    {
        self.base_url_env
            .as_deref()
            .and_then(lookup)
            .filter(|value| !value.trim().is_empty())
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
        // No environment variable: the registry cannot know a driver's vendor
        // convention, and guessing one is what the per-driver declaration
        // exists to prevent. A driver that wants env resolution overrides this
        // schema and names its own variables.
        CredentialFormSchema {
            fields: vec![
                crate::credential_schema::FormField::password("api_key", "API Key").required(),
            ],
            instructions_markdown: String::new(),
        }
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
/// use everruns_contracts::{DriverRegistry, DriverId};
/// use everruns_drivers::anthropic::register_driver;
/// use everruns_drivers::openai::register_driver as register_openai;
///
/// let mut registry = DriverRegistry::new();
/// everruns_drivers::anthropic::register_driver(&mut registry);
/// everruns_drivers::openai::register_driver(&mut registry);
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
#[path = "driver_registry_tests.rs"]
mod tests;
