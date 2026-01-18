// OpenAI LLM Driver
//
// Production implementation for OpenAI's API.
// Wraps OpenAIProtocolLlmDriver from core and can add OpenAI-specific features in the future.

use async_trait::async_trait;
use chrono::TimeZone;

use everruns_core::OpenAIProtocolLlmDriver;
use everruns_core::error::{AgentLoopError, Result};
use everruns_core::llm_driver_registry::{
    BoxedLlmDriver, DiscoveredModel, DriverRegistry, LlmCallConfig, LlmDriver, LlmMessage,
    LlmResponseStream, ProviderType,
};

use crate::types::OpenAiModelsResponse;

const OPENAI_MODELS_URL: &str = "https://api.openai.com/v1/models";

/// OpenAI LLM Driver
///
/// Production driver for OpenAI's API. Wraps `OpenAIProtocolLlmDriver` and can add
/// OpenAI-specific features in the future (e.g., structured outputs, function calling v2, etc.)
///
/// # Example
///
/// ```ignore
/// use everruns_openai::OpenAILlmDriver;
///
/// let driver = OpenAILlmDriver::from_env()?;
/// // or
/// let driver = OpenAILlmDriver::new("your-api-key");
/// // or with custom endpoint
/// let driver = OpenAILlmDriver::with_base_url("your-api-key", "https://api.example.com/v1/chat/completions");
/// ```
#[derive(Clone)]
pub struct OpenAILlmDriver {
    inner: OpenAIProtocolLlmDriver,
    /// Whether using a custom base URL (not OpenAI's API)
    uses_custom_url: bool,
}

impl OpenAILlmDriver {
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
impl LlmDriver for OpenAILlmDriver {
    async fn chat_completion_stream(
        &self,
        messages: Vec<LlmMessage>,
        config: &LlmCallConfig,
    ) -> Result<LlmResponseStream> {
        // Delegate to the base protocol implementation
        // Future: Add OpenAI-specific preprocessing here
        self.inner.chat_completion_stream(messages, config).await
    }

    async fn list_models(&self) -> Result<Option<Vec<DiscoveredModel>>> {
        // Skip discovery for custom URLs (proxies, self-hosted)
        if self.uses_custom_url {
            return Ok(None);
        }

        let response = self
            .inner
            .client()
            .get(OPENAI_MODELS_URL)
            .bearer_auth(self.inner.api_key())
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
            })
            .collect();

        Ok(Some(discovered))
    }
}

impl std::fmt::Debug for OpenAILlmDriver {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("OpenAILlmDriver")
            .field("api_url", &self.api_url())
            .field("api_key", &"[REDACTED]")
            .finish()
    }
}

// ============================================================================
// Driver Registration
// ============================================================================

/// Register the OpenAI driver with the driver registry
///
/// This registers drivers for both OpenAI and Azure OpenAI provider types.
/// Should be called at application startup to enable OpenAI model support.
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
    // Register for OpenAI
    registry.register(ProviderType::OpenAI, |api_key, base_url| {
        let driver = match base_url {
            Some(url) => OpenAILlmDriver::with_base_url(api_key, url),
            None => OpenAILlmDriver::new(api_key),
        };
        Box::new(driver) as BoxedLlmDriver
    });

    // Register for Azure OpenAI (uses same driver implementation)
    registry.register(ProviderType::AzureOpenAI, |api_key, base_url| {
        let driver = match base_url {
            Some(url) => OpenAILlmDriver::with_base_url(api_key, url),
            None => OpenAILlmDriver::new(api_key),
        };
        Box::new(driver) as BoxedLlmDriver
    });
}
