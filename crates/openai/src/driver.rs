// OpenAI Chat Drivers
//
// This module provides two separate drivers for OpenAI:
//
// 1. OpenAIChatDriver - Uses Open Responses API (https://www.openresponses.org/)
//    Recommended for new projects. Better performance with reasoning models.
//
// 2. OpenAICompletionsChatDriver - Uses Chat Completions API
//    For backward compatibility with /v1/chat/completions endpoint.

use async_trait::async_trait;
use chrono::TimeZone;
use reqwest::RequestBuilder;
use reqwest::Url;

use everruns_core::OpenAIProtocolChatDriver;
use everruns_core::OpenResponsesProtocolChatDriver;
use everruns_core::credential_schema::CredentialFormSchema;
use everruns_core::error::{AgentLoopError, Result};
use everruns_core::llm_driver_registry::{
    BoxedChatDriver, ChatDriver, DiscoveredModel, DriverDescriptor, DriverRegistry, LlmCallConfig,
    LlmMessage, LlmResponseStream, DriverId, ServiceKind,
};
use everruns_core::openai_protocol::is_azure_openai_api_url;
use everruns_core::{CompactRequest, CompactResponse};

use crate::types::{OpenAiModelsResponse, OpenRouterModelsResponse};

const OPENAI_MODELS_URL: &str = "https://api.openai.com/v1/models";
const OPENROUTER_RESPONSES_URL: &str = "https://openrouter.ai/api/v1/responses";

// ============================================================================
// OpenAI Chat Driver (Open Responses API)
// ============================================================================

/// OpenAI Chat Driver using Open Responses API
///
/// Production driver for OpenAI using the Open Responses specification
/// (<https://www.openresponses.org/>). This is the recommended driver for
/// new projects, offering:
/// - Better performance with reasoning models (o1, o3, GPT-5)
/// - Provider-agnostic streaming events
/// - Native agentic loop support
/// - 40-80% better cache utilization
///
/// For backward compatibility with the Chat Completions API, use
/// `OpenAICompletionsChatDriver` instead.
///
/// # Example
///
/// ```ignore
/// use everruns_openai::OpenAIChatDriver;
///
/// let driver = OpenAIChatDriver::new("your-api-key");
///
/// // With custom endpoint
/// let driver = OpenAIChatDriver::with_base_url(
///     "your-api-key",
///     "https://api.example.com/v1/responses",
/// );
/// ```
#[derive(Clone)]
pub struct OpenAIChatDriver {
    inner: OpenResponsesProtocolChatDriver,
    /// Whether using a custom base URL (not OpenAI's API)
    uses_custom_url: bool,
}

impl OpenAIChatDriver {
    /// Create a new driver with the given API key
    pub fn new(api_key: impl Into<String>) -> Self {
        Self {
            inner: OpenResponsesProtocolChatDriver::new(api_key),
            uses_custom_url: false,
        }
    }

    /// Create a new driver from the OPENAI_API_KEY environment variable
    pub fn from_env() -> Result<Self> {
        Ok(Self {
            inner: OpenResponsesProtocolChatDriver::from_env()?,
            uses_custom_url: false,
        })
    }

    /// Create a new driver with a custom API URL
    pub fn with_base_url(api_key: impl Into<String>, api_url: impl Into<String>) -> Self {
        let api_url = normalize_api_url(&api_url.into(), "/responses");
        Self {
            inner: OpenResponsesProtocolChatDriver::with_base_url(api_key, api_url),
            uses_custom_url: true,
        }
    }

    /// Get the API URL
    pub fn api_url(&self) -> &str {
        self.inner.api_url()
    }

    /// Check if using a custom base URL
    pub fn uses_custom_url(&self) -> bool {
        self.uses_custom_url
    }

    /// Compact a conversation to reduce context size
    ///
    /// This method calls the /v1/responses/compact endpoint to compress the conversation
    /// history. User messages are kept verbatim, while assistant messages, tool calls,
    /// and tool results are replaced by an encrypted compaction item.
    ///
    /// # Arguments
    ///
    /// * `request` - The compact request containing the model and input items
    ///
    /// # Returns
    ///
    /// Returns a `CompactResponse` containing the compacted output items.
    /// The output can be used directly as input for the next /v1/responses call.
    pub async fn compact_conversation(&self, request: CompactRequest) -> Result<CompactResponse> {
        self.inner.compact(request).await
    }

    /// Check if this driver supports the compact endpoint
    ///
    /// Returns true for OpenAI's Responses API. Custom endpoints may or may not
    /// support compaction.
    pub fn can_compact(&self) -> bool {
        self.inner.supports_compact()
    }
}

#[async_trait]
impl ChatDriver for OpenAIChatDriver {
    async fn chat_completion_stream(
        &self,
        messages: Vec<LlmMessage>,
        config: &LlmCallConfig,
    ) -> Result<LlmResponseStream> {
        self.inner.chat_completion_stream(messages, config).await
    }

    async fn list_models(&self) -> Result<Option<Vec<DiscoveredModel>>> {
        // Skip discovery for non-standard custom URLs (proxies, self-hosted)
        if self.uses_custom_url && !supports_model_listing(self.api_url()) {
            return Ok(None);
        }

        let models_url = models_url_for_api_url(self.api_url());
        list_models_for_url(self.inner.client(), self.inner.api_key(), &models_url).await
    }

    fn supports_compact(&self) -> bool {
        self.inner.supports_compact()
    }

    async fn compact(&self, request: CompactRequest) -> Result<Option<CompactResponse>> {
        Ok(Some(self.inner.compact(request).await?))
    }
}

impl std::fmt::Debug for OpenAIChatDriver {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("OpenAIChatDriver")
            .field("api_url", &self.api_url())
            .field("api", &"Open Responses")
            .field("api_key", &"[REDACTED]")
            .finish()
    }
}

// ============================================================================
// OpenRouter Chat Driver (OpenAI-compatible Responses API)
// ============================================================================

/// OpenRouter driver using its OpenAI-compatible Responses API.
#[derive(Clone)]
pub struct OpenRouterChatDriver {
    inner: OpenResponsesProtocolChatDriver,
    /// Whether constructed with an explicit base URL override via [`with_base_url`].
    uses_custom_url: bool,
}

impl OpenRouterChatDriver {
    /// Create a new OpenRouter driver with the default Responses API endpoint.
    pub fn new(api_key: impl Into<String>) -> Self {
        Self {
            inner: OpenResponsesProtocolChatDriver::with_base_url(
                api_key,
                OPENROUTER_RESPONSES_URL,
            )
            .with_provider_type(DriverId::OpenRouter),
            uses_custom_url: false,
        }
    }

    /// Create a new OpenRouter driver with an explicit API URL override.
    pub fn with_base_url(api_key: impl Into<String>, api_url: impl Into<String>) -> Self {
        let api_url = normalize_api_url(&api_url.into(), "/responses");
        Self {
            inner: OpenResponsesProtocolChatDriver::with_base_url(api_key, api_url)
                .with_provider_type(DriverId::OpenRouter),
            uses_custom_url: true,
        }
    }

    /// Get the API URL.
    pub fn api_url(&self) -> &str {
        self.inner.api_url()
    }

    /// Get the provider type used for model profile lookup.
    pub fn provider_type(&self) -> &DriverId {
        self.inner.provider_type()
    }

    /// Check if using a custom base URL.
    pub fn uses_custom_url(&self) -> bool {
        self.uses_custom_url
    }
}

#[async_trait]
impl ChatDriver for OpenRouterChatDriver {
    async fn chat_completion_stream(
        &self,
        messages: Vec<LlmMessage>,
        config: &LlmCallConfig,
    ) -> Result<LlmResponseStream> {
        self.inner.chat_completion_stream(messages, config).await
    }

    async fn list_models(&self) -> Result<Option<Vec<DiscoveredModel>>> {
        // OpenRouter discovery is only safe against OpenRouter's own host.
        // Custom proxy URLs may resolve to private infrastructure at request time.
        if self.uses_custom_url && !is_openrouter_api_url(self.api_url()) {
            return Ok(None);
        }

        let models_url = models_url_for_api_url(self.api_url());
        list_models_for_url(self.inner.client(), self.inner.api_key(), &models_url).await
    }
}

impl std::fmt::Debug for OpenRouterChatDriver {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("OpenRouterChatDriver")
            .field("api_url", &self.api_url())
            .field("api", &"OpenRouter Responses")
            .field("api_key", &"[REDACTED]")
            .finish()
    }
}

// ============================================================================
// OpenAI Completions Chat Driver (Chat Completions API)
// ============================================================================

/// OpenAI Chat Driver using Chat Completions API
///
/// Driver for OpenAI using the traditional Chat Completions API
/// (/v1/chat/completions). Use this for backward compatibility with
/// existing integrations or when Open Responses API is not suitable.
///
/// For new projects, prefer `OpenAIChatDriver` which uses the Open Responses
/// specification (<https://www.openresponses.org/>).
///
/// # Example
///
/// ```ignore
/// use everruns_openai::OpenAICompletionsChatDriver;
///
/// let driver = OpenAICompletionsChatDriver::new("your-api-key");
///
/// // With custom endpoint
/// let driver = OpenAICompletionsChatDriver::with_base_url(
///     "your-api-key",
///     "https://api.example.com/v1/chat/completions",
/// );
/// ```
#[derive(Clone)]
pub struct OpenAICompletionsChatDriver {
    inner: OpenAIProtocolChatDriver,
    /// Whether using a custom base URL (not OpenAI's API)
    uses_custom_url: bool,
}

impl OpenAICompletionsChatDriver {
    /// Create a new driver with the given API key
    pub fn new(api_key: impl Into<String>) -> Self {
        Self {
            inner: OpenAIProtocolChatDriver::new(api_key),
            uses_custom_url: false,
        }
    }

    /// Create a new driver from the OPENAI_API_KEY environment variable
    pub fn from_env() -> Result<Self> {
        Ok(Self {
            inner: OpenAIProtocolChatDriver::from_env()?,
            uses_custom_url: false,
        })
    }

    /// Create a new driver with a custom API URL
    pub fn with_base_url(api_key: impl Into<String>, api_url: impl Into<String>) -> Self {
        let api_url = normalize_api_url(&api_url.into(), "/chat/completions");
        Self {
            inner: OpenAIProtocolChatDriver::with_base_url(api_key, api_url),
            uses_custom_url: true,
        }
    }

    /// Get the API URL
    pub fn api_url(&self) -> &str {
        self.inner.api_url()
    }

    /// Check if using a custom base URL
    pub fn uses_custom_url(&self) -> bool {
        self.uses_custom_url
    }
}

#[async_trait]
impl ChatDriver for OpenAICompletionsChatDriver {
    async fn chat_completion_stream(
        &self,
        messages: Vec<LlmMessage>,
        config: &LlmCallConfig,
    ) -> Result<LlmResponseStream> {
        self.inner.chat_completion_stream(messages, config).await
    }

    async fn list_models(&self) -> Result<Option<Vec<DiscoveredModel>>> {
        // Skip discovery for non-standard custom URLs (proxies, self-hosted)
        if self.uses_custom_url && !supports_model_listing(self.api_url()) {
            return Ok(None);
        }

        let models_url = models_url_for_api_url(self.api_url());
        list_models_for_url(self.inner.client(), self.inner.api_key(), &models_url).await
    }
}

impl std::fmt::Debug for OpenAICompletionsChatDriver {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("OpenAICompletionsChatDriver")
            .field("api_url", &self.api_url())
            .field("api", &"Chat Completions")
            .field("api_key", &"[REDACTED]")
            .finish()
    }
}

// ============================================================================
// Shared Utilities
// ============================================================================

/// Fetch and filter models for a `/models` URL, dispatching on the provider host.
///
/// OpenRouter returns much richer metadata than OpenAI (a `supported_parameters`
/// array), so we parse it separately to derive capability profiles — notably
/// `reasoning` support, which gates the UI's effort selector.
async fn list_models_for_url(
    client: &reqwest::Client,
    api_key: &str,
    models_url: &str,
) -> Result<Option<Vec<DiscoveredModel>>> {
    if is_openrouter_api_url(models_url) {
        list_openrouter_models(client, api_key, models_url).await
    } else {
        list_openai_models(client, api_key, models_url).await
    }
}

/// Fetch and filter OpenRouter models, building capability profiles from the
/// `supported_parameters` metadata OpenRouter advertises.
async fn list_openrouter_models(
    client: &reqwest::Client,
    api_key: &str,
    models_url: &str,
) -> Result<Option<Vec<DiscoveredModel>>> {
    let response = apply_models_auth(client.get(models_url), models_url, api_key)
        .send()
        .await
        .map_err(|e| AgentLoopError::llm(format!("Failed to fetch models: {}", e)))?;

    if !response.status().is_success() {
        let status = response.status();
        let _ = response.bytes().await; // drain body to allow connection reuse
        return Err(models_api_status_error(status));
    }

    let models_response: OpenRouterModelsResponse = response
        .json()
        .await
        .map_err(|e| AgentLoopError::llm(format!("Failed to parse models response: {}", e)))?;

    let discovered: Vec<DiscoveredModel> = models_response
        .data
        .into_iter()
        .filter(|m| m.is_chat_model())
        .map(|m| {
            let profile = m.to_discovered_profile();
            DiscoveredModel {
                created_at: m
                    .created
                    .and_then(|ts| chrono::Utc.timestamp_opt(ts, 0).single()),
                display_name: m.name.clone(),
                owned_by: m.id.split('/').next().map(str::to_owned),
                model_id: m.id,
                discovered_profile: Some(profile),
            }
        })
        .collect();

    Ok(Some(discovered))
}

/// Fetch and filter OpenAI models (shared between both drivers)
async fn list_openai_models(
    client: &reqwest::Client,
    api_key: &str,
    models_url: &str,
) -> Result<Option<Vec<DiscoveredModel>>> {
    let response = apply_models_auth(client.get(models_url), models_url, api_key)
        .send()
        .await
        .map_err(|e| AgentLoopError::llm(format!("Failed to fetch models: {}", e)))?;

    if !response.status().is_success() {
        let status = response.status();
        let _ = response.bytes().await; // drain body to allow connection reuse
        return Err(models_api_status_error(status));
    }

    let models_response: OpenAiModelsResponse = response
        .json()
        .await
        .map_err(|e| AgentLoopError::llm(format!("Failed to parse models response: {}", e)))?;

    // Filter to chat models only and convert to DiscoveredModel
    let discovered: Vec<DiscoveredModel> = models_response
        .data
        .into_iter()
        .filter(|m| m.is_chat_model())
        .map(|m| DiscoveredModel {
            model_id: m.id,
            display_name: None, // OpenAI doesn't provide display names
            created_at: chrono::Utc.timestamp_opt(m.created, 0).single(),
            owned_by: Some(m.owned_by),
            discovered_profile: None,
        })
        .collect();

    Ok(Some(discovered))
}

fn models_api_status_error(status: reqwest::StatusCode) -> AgentLoopError {
    AgentLoopError::llm(format!("Models API returned status {status}"))
}

fn normalize_api_url(base_url: &str, endpoint_suffix: &str) -> String {
    let trimmed = base_url.trim_end_matches('/');
    if trimmed.ends_with(endpoint_suffix) {
        trimmed.to_string()
    } else {
        format!("{trimmed}{endpoint_suffix}")
    }
}

fn models_url_for_api_url(api_url: &str) -> String {
    let trimmed = api_url.trim_end_matches('/');

    if let Some(prefix) = trimmed.strip_suffix("/responses") {
        return format!("{prefix}/models");
    }
    if let Some(prefix) = trimmed.strip_suffix("/chat/completions") {
        return format!("{prefix}/models");
    }
    if trimmed.ends_with("/models") {
        return trimmed.to_string();
    }
    if trimmed.ends_with("/v1") || trimmed.ends_with("/openai/v1") {
        return format!("{trimmed}/models");
    }

    OPENAI_MODELS_URL.to_string()
}

fn supports_model_listing(api_url: &str) -> bool {
    is_openai_api_url(api_url) || is_azure_openai_api_url(api_url) || is_openrouter_api_url(api_url)
}

fn is_openai_api_url(api_url: &str) -> bool {
    url_host_eq(api_url, "api.openai.com")
}

/// OpenRouter exposes an OpenAI-compatible `/models` endpoint with richer
/// metadata; recognize its host so discovery (and capability profiling) runs.
fn is_openrouter_api_url(api_url: &str) -> bool {
    url_host_eq(api_url, "openrouter.ai")
}

fn url_host_eq(api_url: &str, host: &str) -> bool {
    Url::parse(api_url)
        .ok()
        .and_then(|url| url.host_str().map(str::to_owned))
        .is_some_and(|h| h.eq_ignore_ascii_case(host))
}

fn apply_models_auth(request: RequestBuilder, api_url: &str, api_key: &str) -> RequestBuilder {
    if is_azure_openai_api_url(api_url) {
        request.header("api-key", api_key)
    } else {
        request.bearer_auth(api_key)
    }
}

// ============================================================================
// Driver Registration
// ============================================================================

/// Register all OpenAI drivers with the driver registry
///
/// This registers:
/// - `DriverId::OpenAI` - Open Responses API (recommended)
/// - `DriverId::OpenRouter` - OpenRouter Responses API
/// - `DriverId::AzureOpenAI` - Azure OpenAI Responses API
/// - `DriverId::OpenAICompletions` - Chat Completions API (backward compatibility)
///
/// # Example
///
/// ```ignore
/// use everruns_core::DriverRegistry;
/// use everruns_openai::register_driver;
///
/// let mut registry = DriverRegistry::new();
/// register_driver(&mut registry);
/// ```
pub fn register_driver(registry: &mut DriverRegistry) {
    // Register OpenAI with Open Responses API (recommended). OpenAI providers
    // also power realtime voice sessions (specs/voice.md), so the descriptor
    // declares the Realtime service alongside Chat.
    registry.register_descriptor(DriverDescriptor {
        services: vec![ServiceKind::Chat, ServiceKind::Realtime],
        credential_schema: CredentialFormSchema::api_key(
            "Create an API key at [platform.openai.com/api-keys](https://platform.openai.com/api-keys).",
        ),
        ..DriverDescriptor::chat_only(DriverId::OpenAI, |config| {
            let api_key = config.api_key.as_deref().unwrap_or("");
            let driver = match config.base_url.as_deref() {
                Some(url) => OpenAIChatDriver::with_base_url(api_key, url),
                None => OpenAIChatDriver::new(api_key),
            };
            Box::new(driver) as BoxedChatDriver
        })
    });

    registry.register_descriptor(DriverDescriptor {
        credential_schema: CredentialFormSchema::api_key(
            "Create an API key at [openrouter.ai/settings/keys](https://openrouter.ai/settings/keys).",
        ),
        ..DriverDescriptor::chat_only(DriverId::OpenRouter, |config| {
            let api_key = config.api_key.as_deref().unwrap_or("");
            let driver = match config.base_url.as_deref() {
                Some(url) => OpenRouterChatDriver::with_base_url(api_key, url),
                None => OpenRouterChatDriver::new(api_key),
            };
            Box::new(driver) as BoxedChatDriver
        })
    });

    registry.register_descriptor(DriverDescriptor {
        credential_schema: CredentialFormSchema::api_key(
            "Use an API key for your Azure OpenAI resource and set the resource endpoint as the base URL.",
        ),
        ..DriverDescriptor::chat_only(DriverId::AzureOpenAI, |config| {
            let api_key = config.api_key.as_deref().unwrap_or("");
            let driver = match config.base_url.as_deref() {
                Some(url) => OpenAIChatDriver::with_base_url(api_key, url),
                None => OpenAIChatDriver::new(api_key),
            };
            Box::new(driver) as BoxedChatDriver
        })
    });

    // Register OpenAI Completions with Chat Completions API
    registry.register_descriptor(DriverDescriptor {
        credential_schema: CredentialFormSchema::api_key(
            "Create an API key at [platform.openai.com/api-keys](https://platform.openai.com/api-keys).",
        ),
        ..DriverDescriptor::chat_only(DriverId::OpenAICompletions, |config| {
            let api_key = config.api_key.as_deref().unwrap_or("");
            let driver = match config.base_url.as_deref() {
                Some(url) => OpenAICompletionsChatDriver::with_base_url(api_key, url),
                None => OpenAICompletionsChatDriver::new(api_key),
            };
            Box::new(driver) as BoxedChatDriver
        })
    });
}

#[cfg(test)]
mod tests {
    use super::{
        is_openai_api_url, is_openrouter_api_url, models_url_for_api_url, supports_model_listing,
    };

    #[test]
    fn supports_model_listing_for_openai_host_with_port() {
        assert!(supports_model_listing(
            "https://api.openai.com:443/v1/responses"
        ));
    }

    #[test]
    fn rejects_non_openai_hosts_for_model_listing() {
        assert!(!is_openai_api_url("https://example.com/v1/responses"));
    }

    #[test]
    fn supports_model_listing_for_openrouter() {
        // OpenRouter is reached via the Open Responses driver with a custom base
        // URL; discovery must run so capability profiles (reasoning) are derived.
        assert!(is_openrouter_api_url(
            "https://openrouter.ai/api/v1/responses"
        ));
        assert!(supports_model_listing(
            "https://openrouter.ai/api/v1/responses"
        ));
        assert!(!is_openrouter_api_url("https://example.com/v1/responses"));
    }

    #[test]
    fn openrouter_models_url_is_derived_from_responses_url() {
        assert_eq!(
            models_url_for_api_url("https://openrouter.ai/api/v1/responses"),
            "https://openrouter.ai/api/v1/models"
        );
    }
}
