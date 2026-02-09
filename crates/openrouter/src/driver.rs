// OpenRouter LLM Driver
//
// Wraps the OpenAI Chat Completions protocol driver with OpenRouter-specific
// configuration and model discovery. OpenRouter provides a unified API for
// accessing models from multiple providers (OpenAI, Anthropic, Google, Meta, etc.).
//
// Base URL: https://openrouter.ai/api/v1/chat/completions
// Models:   https://openrouter.ai/api/v1/models

use async_trait::async_trait;

use everruns_core::OpenAIProtocolLlmDriver;
use everruns_core::error::{AgentLoopError, Result};
use everruns_core::llm_driver_registry::{
    BoxedLlmDriver, DiscoveredModel, DriverRegistry, LlmCallConfig, LlmDriver, LlmMessage,
    LlmResponseStream, ProviderType,
};

const OPENROUTER_API_URL: &str = "https://openrouter.ai/api/v1/chat/completions";
const OPENROUTER_MODELS_URL: &str = "https://openrouter.ai/api/v1/models";

/// OpenRouter LLM Driver
///
/// Uses the OpenAI Chat Completions protocol via OpenRouter's API.
/// OpenRouter routes requests to the appropriate upstream provider based
/// on the model ID (e.g., "openai/gpt-4o", "anthropic/claude-3-opus").
///
/// Model discovery fetches all available models from OpenRouter's catalog.
#[derive(Clone)]
pub struct OpenRouterLlmDriver {
    inner: OpenAIProtocolLlmDriver,
    /// Whether using a custom base URL (not OpenRouter's API)
    uses_custom_url: bool,
}

impl OpenRouterLlmDriver {
    /// Create a new driver with the given API key
    pub fn new(api_key: impl Into<String>) -> Self {
        Self {
            inner: OpenAIProtocolLlmDriver::with_base_url(api_key, OPENROUTER_API_URL),
            uses_custom_url: false,
        }
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
}

#[async_trait]
impl LlmDriver for OpenRouterLlmDriver {
    async fn chat_completion_stream(
        &self,
        messages: Vec<LlmMessage>,
        config: &LlmCallConfig,
    ) -> Result<LlmResponseStream> {
        self.inner.chat_completion_stream(messages, config).await
    }

    async fn list_models(&self) -> Result<Option<Vec<DiscoveredModel>>> {
        // Skip discovery for custom URLs
        if self.uses_custom_url {
            return Ok(None);
        }

        list_openrouter_models(self.inner.client(), self.inner.api_key()).await
    }
}

impl std::fmt::Debug for OpenRouterLlmDriver {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("OpenRouterLlmDriver")
            .field("api_url", &self.api_url())
            .field("api_key", &"[REDACTED]")
            .finish()
    }
}

// ============================================================================
// Model Discovery
// ============================================================================

/// OpenRouter models API response
#[derive(serde::Deserialize)]
struct OpenRouterModelsResponse {
    data: Vec<OpenRouterModel>,
}

/// A single model from the OpenRouter models API
#[derive(serde::Deserialize)]
struct OpenRouterModel {
    /// Model ID (e.g., "openai/gpt-4o", "anthropic/claude-3-opus")
    id: String,
    /// Display name
    #[serde(default)]
    name: Option<String>,
    /// Created timestamp (unix seconds)
    #[serde(default)]
    created: Option<i64>,
}

/// Fetch available models from OpenRouter's catalog
async fn list_openrouter_models(
    client: &reqwest::Client,
    api_key: &str,
) -> Result<Option<Vec<DiscoveredModel>>> {
    let response = client
        .get(OPENROUTER_MODELS_URL)
        .bearer_auth(api_key)
        .send()
        .await
        .map_err(|e| AgentLoopError::llm(format!("Failed to fetch OpenRouter models: {}", e)))?;

    if !response.status().is_success() {
        let status = response.status();
        let body = response.text().await.unwrap_or_default();
        return Err(AgentLoopError::llm(format!(
            "OpenRouter Models API returned {}: {}",
            status, body
        )));
    }

    let models_response: OpenRouterModelsResponse = response
        .json()
        .await
        .map_err(|e| AgentLoopError::llm(format!("Failed to parse OpenRouter models: {}", e)))?;

    let discovered: Vec<DiscoveredModel> = models_response
        .data
        .into_iter()
        .map(|m| DiscoveredModel {
            model_id: m.id,
            display_name: m.name,
            created_at: m
                .created
                .and_then(|ts| chrono::DateTime::from_timestamp(ts, 0)),
            owned_by: None,
        })
        .collect();

    Ok(Some(discovered))
}

// ============================================================================
// Driver Registration
// ============================================================================

/// Register the OpenRouter driver with the driver registry
pub fn register_driver(registry: &mut DriverRegistry) {
    registry.register(ProviderType::OpenRouter, |api_key, base_url| {
        let driver = match base_url {
            Some(url) => OpenRouterLlmDriver::with_base_url(api_key, url),
            None => OpenRouterLlmDriver::new(api_key),
        };
        Box::new(driver) as BoxedLlmDriver
    });
}

// ============================================================================
// Tests
// ============================================================================

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_driver_default_url() {
        let driver = OpenRouterLlmDriver::new("test-key");
        assert_eq!(driver.api_url(), OPENROUTER_API_URL);
    }

    #[test]
    fn test_driver_custom_url() {
        let driver =
            OpenRouterLlmDriver::with_base_url("test-key", "https://custom.openrouter.ai/v1");
        assert_eq!(driver.api_url(), "https://custom.openrouter.ai/v1");
    }

    #[test]
    fn test_driver_debug_redacts_api_key() {
        let driver = OpenRouterLlmDriver::new("sk-or-v1-secret");
        let debug_str = format!("{:?}", driver);
        assert!(debug_str.contains("[REDACTED]"));
        assert!(!debug_str.contains("sk-or-v1-secret"));
    }

    #[test]
    fn test_register_driver() {
        let mut registry = DriverRegistry::new();
        register_driver(&mut registry);
        assert!(registry.has_driver(&ProviderType::OpenRouter));
    }
}
