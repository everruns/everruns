// OpenAI LLM Drivers
//
// This module provides two separate drivers for OpenAI:
//
// 1. OpenAILlmDriver - Uses Open Responses API (https://www.openresponses.org/)
//    Recommended for new projects. Better performance with reasoning models.
//
// 2. OpenAICompletionsLlmDriver - Uses Chat Completions API
//    For backward compatibility with /v1/chat/completions endpoint.

use async_trait::async_trait;
use chrono::TimeZone;

use everruns_core::OpenAIProtocolLlmDriver;
use everruns_core::OpenResponsesProtocolLlmDriver;
use everruns_core::error::{AgentLoopError, Result};
use everruns_core::llm_driver_registry::{
    BoxedLlmDriver, DiscoveredModel, DriverRegistry, LlmCallConfig, LlmDriver, LlmMessage,
    LlmResponseStream, ProviderType,
};
use everruns_core::{CompactRequest, CompactResponse};

use crate::types::OpenAiModelsResponse;

const OPENAI_MODELS_URL: &str = "https://api.openai.com/v1/models";

// ============================================================================
// OpenAI LLM Driver (Open Responses API)
// ============================================================================

/// OpenAI LLM Driver using Open Responses API
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
/// `OpenAICompletionsLlmDriver` instead.
///
/// # Example
///
/// ```ignore
/// use everruns_openai::OpenAILlmDriver;
///
/// let driver = OpenAILlmDriver::new("your-api-key");
///
/// // With custom endpoint
/// let driver = OpenAILlmDriver::with_base_url(
///     "your-api-key",
///     "https://api.example.com/v1/responses",
/// );
/// ```
#[derive(Clone)]
pub struct OpenAILlmDriver {
    inner: OpenResponsesProtocolLlmDriver,
    /// Whether using a custom base URL (not OpenAI's API)
    uses_custom_url: bool,
}

impl OpenAILlmDriver {
    /// Create a new driver with the given API key
    pub fn new(api_key: impl Into<String>) -> Self {
        Self {
            inner: OpenResponsesProtocolLlmDriver::new(api_key),
            uses_custom_url: false,
        }
    }

    /// Create a new driver from the OPENAI_API_KEY environment variable
    pub fn from_env() -> Result<Self> {
        Ok(Self {
            inner: OpenResponsesProtocolLlmDriver::from_env()?,
            uses_custom_url: false,
        })
    }

    /// Create a new driver with a custom API URL
    pub fn with_base_url(api_key: impl Into<String>, api_url: impl Into<String>) -> Self {
        Self {
            inner: OpenResponsesProtocolLlmDriver::with_base_url(api_key, api_url),
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
impl LlmDriver for OpenAILlmDriver {
    async fn chat_completion_stream(
        &self,
        messages: Vec<LlmMessage>,
        config: &LlmCallConfig,
    ) -> Result<LlmResponseStream> {
        self.inner.chat_completion_stream(messages, config).await
    }

    async fn list_models(&self) -> Result<Option<Vec<DiscoveredModel>>> {
        // Skip discovery for custom URLs (proxies, self-hosted)
        if self.uses_custom_url {
            return Ok(None);
        }

        list_openai_models(self.inner.client(), self.inner.api_key()).await
    }

    fn supports_compact(&self) -> bool {
        self.inner.supports_compact()
    }

    async fn compact(&self, request: CompactRequest) -> Result<Option<CompactResponse>> {
        Ok(Some(self.inner.compact(request).await?))
    }
}

impl std::fmt::Debug for OpenAILlmDriver {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("OpenAILlmDriver")
            .field("api_url", &self.api_url())
            .field("api", &"Open Responses")
            .field("api_key", &"[REDACTED]")
            .finish()
    }
}

// ============================================================================
// OpenAI Completions LLM Driver (Chat Completions API)
// ============================================================================

/// OpenAI LLM Driver using Chat Completions API
///
/// Driver for OpenAI using the traditional Chat Completions API
/// (/v1/chat/completions). Use this for backward compatibility with
/// existing integrations or when Open Responses API is not suitable.
///
/// For new projects, prefer `OpenAILlmDriver` which uses the Open Responses
/// specification (<https://www.openresponses.org/>).
///
/// # Example
///
/// ```ignore
/// use everruns_openai::OpenAICompletionsLlmDriver;
///
/// let driver = OpenAICompletionsLlmDriver::new("your-api-key");
///
/// // With custom endpoint
/// let driver = OpenAICompletionsLlmDriver::with_base_url(
///     "your-api-key",
///     "https://api.example.com/v1/chat/completions",
/// );
/// ```
#[derive(Clone)]
pub struct OpenAICompletionsLlmDriver {
    inner: OpenAIProtocolLlmDriver,
    /// Whether using a custom base URL (not OpenAI's API)
    uses_custom_url: bool,
}

impl OpenAICompletionsLlmDriver {
    /// Create a new driver with the given API key
    pub fn new(api_key: impl Into<String>) -> Self {
        Self {
            inner: OpenAIProtocolLlmDriver::new(api_key),
            uses_custom_url: false,
        }
    }

    /// Create a new driver from the OPENAI_API_KEY environment variable
    pub fn from_env() -> Result<Self> {
        Ok(Self {
            inner: OpenAIProtocolLlmDriver::from_env()?,
            uses_custom_url: false,
        })
    }

    /// Create a new driver with a custom API URL
    pub fn with_base_url(api_key: impl Into<String>, api_url: impl Into<String>) -> Self {
        Self {
            inner: OpenAIProtocolLlmDriver::with_base_url(api_key, api_url),
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
impl LlmDriver for OpenAICompletionsLlmDriver {
    async fn chat_completion_stream(
        &self,
        messages: Vec<LlmMessage>,
        config: &LlmCallConfig,
    ) -> Result<LlmResponseStream> {
        self.inner.chat_completion_stream(messages, config).await
    }

    async fn list_models(&self) -> Result<Option<Vec<DiscoveredModel>>> {
        // Skip discovery for custom URLs (proxies, self-hosted)
        if self.uses_custom_url {
            return Ok(None);
        }

        list_openai_models(self.inner.client(), self.inner.api_key()).await
    }
}

impl std::fmt::Debug for OpenAICompletionsLlmDriver {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("OpenAICompletionsLlmDriver")
            .field("api_url", &self.api_url())
            .field("api", &"Chat Completions")
            .field("api_key", &"[REDACTED]")
            .finish()
    }
}

// ============================================================================
// Shared Utilities
// ============================================================================

/// Fetch and filter OpenAI models (shared between both drivers)
async fn list_openai_models(
    client: &reqwest::Client,
    api_key: &str,
) -> Result<Option<Vec<DiscoveredModel>>> {
    let response = client
        .get(OPENAI_MODELS_URL)
        .bearer_auth(api_key)
        .send()
        .await
        .map_err(|e| AgentLoopError::llm(format!("Failed to fetch models: {}", e)))?;

    if !response.status().is_success() {
        let status = response.status();
        let body = response.text().await.unwrap_or_default();
        return Err(AgentLoopError::llm(format!(
            "Models API returned {}: {}",
            status, body
        )));
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

// ============================================================================
// Driver Registration
// ============================================================================

/// Register all OpenAI drivers with the driver registry
///
/// This registers:
/// - `ProviderType::OpenAI` - Open Responses API (recommended)
/// - `ProviderType::OpenAICompletions` - Chat Completions API (backward compatibility)
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
    // Register OpenAI with Open Responses API (recommended)
    registry.register(ProviderType::OpenAI, |api_key, base_url| {
        let driver = match base_url {
            Some(url) => OpenAILlmDriver::with_base_url(api_key, url),
            None => OpenAILlmDriver::new(api_key),
        };
        Box::new(driver) as BoxedLlmDriver
    });

    // Register OpenAI Completions with Chat Completions API
    registry.register(ProviderType::OpenAICompletions, |api_key, base_url| {
        let driver = match base_url {
            Some(url) => OpenAICompletionsLlmDriver::with_base_url(api_key, url),
            None => OpenAICompletionsLlmDriver::new(api_key),
        };
        Box::new(driver) as BoxedLlmDriver
    });
}
