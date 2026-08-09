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
    ChatDriver, DiscoveredModel, DriverDescriptor, DriverId, DriverRegistry,
    EmbeddingsDriverFactory, LlmCallConfig, LlmMessage, LlmResponseStream, ServiceKind,
};
use everruns_provider::error::{AgentLoopError, Result};
use everruns_provider::openai_protocol::{
    is_azure_openai_api_url, is_openai_api_url, models_api_status_error, models_url_for_api_url,
};
use everruns_provider::{
    BearerAuth, CompactRequest, CompactResponse, Provider, ProviderEndpoint, StaticHeaderAuth,
};

use crate::types::OpenAiModelsResponse;

/// Ready-to-use OpenAI Responses provider assembly.
pub fn provider(
    id: impl Into<everruns_provider::ProviderKey>,
    api_key: impl Into<String>,
) -> Provider {
    Provider::new(id, OpenAIChatDriver::new())
        .base_url("https://api.openai.com/v1")
        .auth(BearerAuth::new(api_key))
}

/// Ready-to-use Azure OpenAI Responses provider assembly.
pub fn azure_provider(
    id: impl Into<everruns_provider::ProviderKey>,
    base_url: impl Into<String>,
    api_key: impl Into<String>,
) -> Provider {
    Provider::new(id, OpenAIChatDriver::new())
        .base_url(base_url)
        .auth(StaticHeaderAuth::new("api-key", api_key))
}

/// Ready-to-use OpenAI Chat Completions provider assembly.
pub fn completions_provider(
    id: impl Into<everruns_provider::ProviderKey>,
    api_key: impl Into<String>,
) -> Provider {
    Provider::new(id, OpenAICompletionsChatDriver::new())
        .base_url("https://api.openai.com/v1")
        .auth(BearerAuth::new(api_key))
}

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
/// let driver = OpenAIChatDriver::new();
/// // Bind it to a runtime Provider for endpoint and authentication.
/// ```
#[derive(Clone)]
pub struct OpenAIChatDriver {
    inner: OpenResponsesProtocolChatDriver,
}

impl OpenAIChatDriver {
    /// Create an Open Responses wire-protocol driver.
    pub fn new() -> Self {
        Self {
            inner: OpenResponsesProtocolChatDriver::new()
                .with_stateful_responses(true)
                .with_native_features(true, true),
        }
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
    pub async fn compact_conversation(
        &self,
        endpoint: &ProviderEndpoint,
        request: CompactRequest,
    ) -> Result<CompactResponse> {
        self.inner.compact(endpoint, request).await
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
        endpoint: &ProviderEndpoint,
        messages: Vec<LlmMessage>,
        config: &LlmCallConfig,
    ) -> Result<LlmResponseStream> {
        self.inner
            .chat_completion_stream(endpoint, messages, config)
            .await
    }

    async fn list_models(
        &self,
        endpoint: &ProviderEndpoint,
    ) -> Result<Option<Vec<DiscoveredModel>>> {
        let Some(api_url) = endpoint.url("responses") else {
            return Ok(None);
        };
        // Skip discovery for non-standard custom URLs (proxies, self-hosted)
        if !supports_model_listing(&api_url) {
            return Ok(None);
        }

        let models_url = models_url_for_api_url(&api_url);
        list_openai_models(self.inner.client(), endpoint, &models_url).await
    }

    fn supports_compact(&self) -> bool {
        self.inner.supports_compact()
    }

    fn supports_parallel_tool_calls(&self, model: &str) -> bool {
        self.inner.supports_parallel_tool_calls(model)
    }

    async fn compact(
        &self,
        endpoint: &ProviderEndpoint,
        request: CompactRequest,
    ) -> Result<Option<CompactResponse>> {
        Ok(Some(self.inner.compact(endpoint, request).await?))
    }
}

impl std::fmt::Debug for OpenAIChatDriver {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("OpenAIChatDriver")
            .field("api", &"Open Responses")
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
/// let driver = OpenAICompletionsChatDriver::new();
/// // Bind it to a runtime Provider for endpoint and authentication.
/// ```
#[derive(Clone)]
pub struct OpenAICompletionsChatDriver {
    inner: OpenAIProtocolChatDriver,
}

impl OpenAICompletionsChatDriver {
    /// Create a Chat Completions wire-protocol driver.
    pub fn new() -> Self {
        Self {
            inner: OpenAIProtocolChatDriver::new(),
        }
    }
}

#[async_trait]
impl ChatDriver for OpenAICompletionsChatDriver {
    async fn chat_completion_stream(
        &self,
        endpoint: &ProviderEndpoint,
        messages: Vec<LlmMessage>,
        config: &LlmCallConfig,
    ) -> Result<LlmResponseStream> {
        self.inner
            .chat_completion_stream(endpoint, messages, config)
            .await
    }

    async fn list_models(
        &self,
        endpoint: &ProviderEndpoint,
    ) -> Result<Option<Vec<DiscoveredModel>>> {
        let Some(api_url) = endpoint.url("chat/completions") else {
            return Ok(None);
        };
        // Skip discovery for non-standard custom URLs (proxies, self-hosted)
        if !supports_model_listing(&api_url) {
            return Ok(None);
        }

        let models_url = models_url_for_api_url(&api_url);
        list_openai_models(self.inner.client(), endpoint, &models_url).await
    }

    fn supports_parallel_tool_calls(&self, model: &str) -> bool {
        self.inner.supports_parallel_tool_calls(model)
    }
}

impl std::fmt::Debug for OpenAICompletionsChatDriver {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("OpenAICompletionsChatDriver")
            .field("api", &"Chat Completions")
            .finish()
    }
}

// ============================================================================
// Shared Utilities
// ============================================================================

/// Fetch and filter OpenAI models (shared between both OpenAI drivers)
async fn list_openai_models(
    client: &reqwest::Client,
    endpoint: &ProviderEndpoint,
    models_url: &str,
) -> Result<Option<Vec<DiscoveredModel>>> {
    let resolved = endpoint.resolve("GET", models_url, &[]).await?;
    let mut request = client.get(&resolved.url);
    for (name, value) in resolved.headers {
        request = request.header(name, value);
    }
    let response = request
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

    // Keep models supported by one of this driver's concrete services. The
    // provider supports both chat and embeddings, so filtering to chat here
    // made legitimate embedding models impossible to discover.
    let discovered: Vec<DiscoveredModel> = models_response
        .data
        .into_iter()
        .filter(|m| m.is_chat_model() || m.is_embedding_model())
        .map(|m| DiscoveredModel {
            capabilities: if m.is_embedding_model() {
                vec!["embeddings".to_string()]
            } else {
                vec!["chat".to_string()]
            },
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
    // also power realtime voice sessions (knowledge/operations/voice.md) and text embeddings
    // (knowledge/foundations/providers.md phase 6), so the descriptor declares those services
    // alongside Chat.
    let openai_embeddings_factory: EmbeddingsDriverFactory = std::sync::Arc::new(|config| {
        Provider::new(config.provider.clone(), OpenAIChatDriver::new())
            .base_url(
                config
                    .base_url
                    .as_deref()
                    .unwrap_or("https://api.openai.com/v1"),
            )
            .auth(BearerAuth::new(config.api_key.clone().unwrap_or_default()))
            .bind_embeddings(Box::new(crate::embeddings::OpenAIEmbeddingsDriver::new()))
    });
    registry.register_descriptor(DriverDescriptor {
        display_name: "OpenAI".into(),
        services: vec![ServiceKind::Chat, ServiceKind::Realtime, ServiceKind::Embeddings],
        credential_schema: CredentialFormSchema::api_key(
            "Create an API key at [platform.openai.com/api-keys](https://platform.openai.com/api-keys).",
        ),
        embeddings: Some(openai_embeddings_factory),
        ..DriverDescriptor::chat_only(DriverId::OpenAI, |config| {
            let api_key = config.api_key.as_deref().unwrap_or("");
            Provider::new(config.provider.clone(), OpenAIChatDriver::new())
                .base_url(config.base_url.as_deref().unwrap_or("https://api.openai.com/v1"))
                .auth(BearerAuth::new(api_key))
                .into_boxed_driver()
        })
    });

    registry.register_descriptor(DriverDescriptor {
        display_name: "Azure OpenAI".into(),
        credential_schema: CredentialFormSchema::api_key(
            "Use an API key for your Azure OpenAI resource and set the resource endpoint as the base URL.",
        ),
        ..DriverDescriptor::chat_only(DriverId::AzureOpenAI, |config| {
            let api_key = config.api_key.as_deref().unwrap_or("");
            Provider::new(config.provider.clone(), OpenAIChatDriver::new())
                .base_url(config.base_url.as_deref().unwrap_or("https://api.openai.com/v1"))
                .auth(StaticHeaderAuth::new("api-key", api_key))
                .into_boxed_driver()
        })
    });

    // Register OpenAI Completions with Chat Completions API
    registry.register_descriptor(DriverDescriptor {
        display_name: "OpenAI (Chat Completions)".into(),
        credential_schema: CredentialFormSchema::api_key(
            "Create an API key at [platform.openai.com/api-keys](https://platform.openai.com/api-keys).",
        ),
        ..DriverDescriptor::chat_only(DriverId::OpenAICompletions, |config| {
            let api_key = config.api_key.as_deref().unwrap_or("");
            Provider::new(
                config.provider.clone(),
                OpenAICompletionsChatDriver::new(),
            )
            .base_url(config.base_url.as_deref().unwrap_or("https://api.openai.com/v1"))
            .auth(BearerAuth::new(api_key))
            .into_boxed_driver()
        })
    });
}

impl Default for OpenAIChatDriver {
    fn default() -> Self {
        Self::new()
    }
}

impl Default for OpenAICompletionsChatDriver {
    fn default() -> Self {
        Self::new()
    }
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
