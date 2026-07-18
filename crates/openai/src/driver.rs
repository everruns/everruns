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

use everruns_provider::OpenAIProtocolChatDriver;
use everruns_provider::OpenResponsesProtocolChatDriver;
use everruns_provider::credential_schema::CredentialFormSchema;
use everruns_provider::driver_registry::{
    BoxedChatDriver, BoxedEmbeddingsDriver, ChatDriver, DiscoveredModel, DriverDescriptor,
    DriverId, DriverRegistry, EmbeddingsDriverFactory, LlmCallConfig, LlmMessage,
    LlmResponseStream, ServiceKind,
};
use everruns_provider::error::{AgentLoopError, Result};
use everruns_provider::openai_protocol::{
    apply_models_api_auth, is_azure_openai_api_url, is_openai_api_url, models_api_status_error,
    models_url_for_api_url, normalize_api_url,
};
use everruns_provider::{CompactRequest, CompactResponse};

use crate::types::OpenAiModelsResponse;

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
        list_openai_models(self.inner.client(), self.inner.api_key(), &models_url).await
    }

    fn supports_compact(&self) -> bool {
        self.inner.supports_compact()
    }

    fn supports_parallel_tool_calls(&self, model: &str) -> bool {
        self.inner.supports_parallel_tool_calls(model)
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
        list_openai_models(self.inner.client(), self.inner.api_key(), &models_url).await
    }

    fn supports_parallel_tool_calls(&self, model: &str) -> bool {
        self.inner.supports_parallel_tool_calls(model)
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

/// Fetch and filter OpenAI models (shared between both OpenAI drivers)
async fn list_openai_models(
    client: &reqwest::Client,
    api_key: &str,
    models_url: &str,
) -> Result<Option<Vec<DiscoveredModel>>> {
    let response = apply_models_api_auth(client.get(models_url), models_url, api_key)
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

/// Whether model discovery should run against `api_url`. Only OpenAI's hosted
/// API and Azure OpenAI expose a `/models` endpoint we can rely on; custom
/// proxy URLs are skipped to avoid requests against unknown infrastructure.
fn supports_model_listing(api_url: &str) -> bool {
    is_openai_api_url(api_url) || is_azure_openai_api_url(api_url)
}

// ============================================================================
// Driver Registration
// ============================================================================

/// Register all OpenAI drivers with the driver registry
///
/// This registers:
/// - `DriverId::OpenAI` - Open Responses API (recommended)
/// - `DriverId::AzureOpenAI` - Azure OpenAI Responses API
/// - `DriverId::OpenAICompletions` - Chat Completions API (backward compatibility)
///
/// OpenRouter is registered separately by the `everruns-openrouter` crate.
///
/// # Example
///
/// ```ignore
/// use everruns_provider::DriverRegistry;
/// use everruns_openai::register_driver;
///
/// let mut registry = DriverRegistry::new();
/// register_driver(&mut registry);
/// ```
pub fn register_driver(registry: &mut DriverRegistry) {
    // Register OpenAI with Open Responses API (recommended). OpenAI providers
    // also power realtime voice sessions (specs/voice.md) and text embeddings
    // (specs/providers.md phase 6), so the descriptor declares those services
    // alongside Chat.
    let openai_embeddings_factory: EmbeddingsDriverFactory = std::sync::Arc::new(|config| {
        let api_key = config.api_key.as_deref().unwrap_or("");
        let driver = match config.base_url.as_deref() {
            Some(url) => crate::embeddings::OpenAIEmbeddingsDriver::with_base_url(api_key, url),
            None => crate::embeddings::OpenAIEmbeddingsDriver::new(api_key),
        };
        Box::new(driver) as BoxedEmbeddingsDriver
    });
    registry.register_descriptor(DriverDescriptor {
        services: vec![ServiceKind::Chat, ServiceKind::Realtime, ServiceKind::Embeddings],
        credential_schema: CredentialFormSchema::api_key(
            "Create an API key at [platform.openai.com/api-keys](https://platform.openai.com/api-keys).",
        ),
        embeddings: Some(openai_embeddings_factory),
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
    use super::{is_openai_api_url, supports_model_listing};

    #[test]
    fn supports_model_listing_for_openai_host_with_port() {
        assert!(supports_model_listing(
            "https://api.openai.com:443/v1/responses"
        ));
    }

    #[test]
    fn rejects_non_openai_hosts_for_model_listing() {
        assert!(!is_openai_api_url("https://example.com/v1/responses"));
        assert!(!supports_model_listing(
            "https://openrouter.ai/api/v1/responses"
        ));
    }
}
