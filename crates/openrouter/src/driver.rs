// OpenRouter LLM Driver
//
// Wraps the OpenAI Chat Completions protocol driver with OpenRouter-specific
// configuration and model discovery. OpenRouter provides a unified API for
// accessing models from multiple providers (OpenAI, Anthropic, Google, Meta, etc.).
//
// App attribution headers (https://openrouter.ai/docs/app-attribution):
// - HTTP-Referer: identifies app URL, used for rankings. Default: "https://everruns.com"
// - X-Title: display name for rankings. Default: agent name from LlmCallConfig metadata.
// Both can be overridden via provider settings in the database.
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
const DEFAULT_HTTP_REFERER: &str = "https://everruns.com";

/// OpenRouter LLM Driver
///
/// Uses the OpenAI Chat Completions protocol via OpenRouter's API.
/// OpenRouter routes requests to the appropriate upstream provider based
/// on the model ID (e.g., "openai/gpt-4o", "anthropic/claude-3-opus").
///
/// Sends app attribution headers with every request:
/// - `HTTP-Referer`: defaults to "https://everruns.com", overridable via provider settings
/// - `X-Title`: defaults to agent name (from metadata), overridable via provider settings
///
/// Model discovery fetches all available models from OpenRouter's catalog.
#[derive(Clone)]
pub struct OpenRouterLlmDriver {
    inner: OpenAIProtocolLlmDriver,
    /// Whether using a custom base URL (not OpenRouter's API)
    uses_custom_url: bool,
    /// HTTP-Referer header override (from provider settings)
    http_referer: String,
    /// X-Title header override (from provider settings). None = use agent name from metadata.
    x_title: Option<String>,
}

impl OpenRouterLlmDriver {
    /// Create a new driver with the given API key
    pub fn new(api_key: impl Into<String>) -> Self {
        Self {
            inner: OpenAIProtocolLlmDriver::with_base_url(api_key, OPENROUTER_API_URL),
            uses_custom_url: false,
            http_referer: DEFAULT_HTTP_REFERER.to_string(),
            x_title: None,
        }
    }

    /// Create a new driver with a custom API URL
    pub fn with_base_url(api_key: impl Into<String>, api_url: impl Into<String>) -> Self {
        Self {
            inner: OpenAIProtocolLlmDriver::with_base_url(api_key, api_url),
            uses_custom_url: true,
            http_referer: DEFAULT_HTTP_REFERER.to_string(),
            x_title: None,
        }
    }

    /// Create a driver from provider settings JSON.
    ///
    /// Settings JSON schema:
    /// ```json
    /// {
    ///   "http_referer": "https://myapp.com",  // optional, default: "https://everruns.com"
    ///   "x_title": "My App Name"              // optional, default: agent name
    /// }
    /// ```
    pub fn from_settings(
        api_key: impl Into<String>,
        base_url: Option<&str>,
        settings: Option<&serde_json::Value>,
    ) -> Self {
        let mut driver = match base_url {
            Some(url) => Self::with_base_url(api_key, url),
            None => Self::new(api_key),
        };

        if let Some(settings) = settings {
            if let Some(referer) = settings.get("http_referer").and_then(|v| v.as_str())
                && !referer.is_empty()
            {
                driver.http_referer = referer.to_string();
            }
            if let Some(title) = settings.get("x_title").and_then(|v| v.as_str())
                && !title.is_empty()
            {
                driver.x_title = Some(title.to_string());
            }
        }

        driver
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
        // Inject OpenRouter attribution headers into the config
        let mut config = config.clone();
        config
            .extra_headers
            .insert("HTTP-Referer".to_string(), self.http_referer.clone());

        // X-Title: use provider override, or fall back to agent name from metadata
        let x_title = self
            .x_title
            .clone()
            .or_else(|| config.metadata.get("agent_name").cloned());
        if let Some(title) = x_title {
            config.extra_headers.insert("X-Title".to_string(), title);
        }

        self.inner.chat_completion_stream(messages, &config).await
    }

    async fn list_models(&self) -> Result<Option<Vec<DiscoveredModel>>> {
        // Skip discovery for custom URLs
        if self.uses_custom_url {
            return Ok(None);
        }

        list_openrouter_models(
            self.inner.client(),
            self.inner.api_key(),
            &self.http_referer,
            self.x_title.as_deref(),
        )
        .await
    }
}

impl std::fmt::Debug for OpenRouterLlmDriver {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("OpenRouterLlmDriver")
            .field("api_url", &self.api_url())
            .field("api_key", &"[REDACTED]")
            .field("http_referer", &self.http_referer)
            .field("x_title", &self.x_title)
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
    http_referer: &str,
    x_title: Option<&str>,
) -> Result<Option<Vec<DiscoveredModel>>> {
    let mut req = client
        .get(OPENROUTER_MODELS_URL)
        .bearer_auth(api_key)
        .header("HTTP-Referer", http_referer);

    if let Some(title) = x_title {
        req = req.header("X-Title", title);
    }

    let response = req
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
    registry.register(ProviderType::OpenRouter, |api_key, base_url, settings| {
        let driver = OpenRouterLlmDriver::from_settings(api_key, base_url, settings);
        Box::new(driver) as BoxedLlmDriver
    });
}

// ============================================================================
// Tests
// ============================================================================

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;
    use std::collections::HashMap;

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

    #[test]
    fn test_default_http_referer() {
        let driver = OpenRouterLlmDriver::new("test-key");
        assert_eq!(driver.http_referer, DEFAULT_HTTP_REFERER);
        assert!(driver.x_title.is_none());
    }

    #[test]
    fn test_from_settings_with_overrides() {
        let settings = json!({
            "http_referer": "https://myapp.com",
            "x_title": "My App"
        });
        let driver = OpenRouterLlmDriver::from_settings("test-key", None, Some(&settings));
        assert_eq!(driver.http_referer, "https://myapp.com");
        assert_eq!(driver.x_title, Some("My App".to_string()));
    }

    #[test]
    fn test_from_settings_empty_values_use_defaults() {
        let settings = json!({
            "http_referer": "",
            "x_title": ""
        });
        let driver = OpenRouterLlmDriver::from_settings("test-key", None, Some(&settings));
        assert_eq!(driver.http_referer, DEFAULT_HTTP_REFERER);
        assert!(driver.x_title.is_none());
    }

    #[test]
    fn test_from_settings_no_settings() {
        let driver = OpenRouterLlmDriver::from_settings("test-key", None, None);
        assert_eq!(driver.http_referer, DEFAULT_HTTP_REFERER);
        assert!(driver.x_title.is_none());
    }

    #[test]
    fn test_from_settings_partial_override() {
        let settings = json!({ "x_title": "Custom Title" });
        let driver = OpenRouterLlmDriver::from_settings("test-key", None, Some(&settings));
        assert_eq!(driver.http_referer, DEFAULT_HTTP_REFERER);
        assert_eq!(driver.x_title, Some("Custom Title".to_string()));
    }

    #[test]
    fn test_extra_headers_injected() {
        let driver = OpenRouterLlmDriver::from_settings(
            "test-key",
            None,
            Some(&json!({ "x_title": "Override Title" })),
        );

        // Simulate what chat_completion_stream does
        let mut config = LlmCallConfig {
            model: "openai/gpt-4o".to_string(),
            temperature: None,
            max_tokens: None,
            tools: vec![],
            reasoning_effort: None,
            metadata: HashMap::new(),
            extra_headers: HashMap::new(),
        };
        config
            .metadata
            .insert("agent_name".to_string(), "Test Agent".to_string());

        // Apply header injection logic (same as in chat_completion_stream)
        config
            .extra_headers
            .insert("HTTP-Referer".to_string(), driver.http_referer.clone());
        let x_title = driver
            .x_title
            .clone()
            .or_else(|| config.metadata.get("agent_name").cloned());
        if let Some(title) = x_title {
            config.extra_headers.insert("X-Title".to_string(), title);
        }

        assert_eq!(
            config.extra_headers.get("HTTP-Referer"),
            Some(&DEFAULT_HTTP_REFERER.to_string())
        );
        // Override title takes precedence over agent_name
        assert_eq!(
            config.extra_headers.get("X-Title"),
            Some(&"Override Title".to_string())
        );
    }

    #[test]
    fn test_agent_name_used_as_x_title_fallback() {
        let driver = OpenRouterLlmDriver::new("test-key");

        let mut config = LlmCallConfig {
            model: "openai/gpt-4o".to_string(),
            temperature: None,
            max_tokens: None,
            tools: vec![],
            reasoning_effort: None,
            metadata: HashMap::new(),
            extra_headers: HashMap::new(),
        };
        config
            .metadata
            .insert("agent_name".to_string(), "My Agent".to_string());

        // Apply header injection logic
        config
            .extra_headers
            .insert("HTTP-Referer".to_string(), driver.http_referer.clone());
        let x_title = driver
            .x_title
            .clone()
            .or_else(|| config.metadata.get("agent_name").cloned());
        if let Some(title) = x_title {
            config.extra_headers.insert("X-Title".to_string(), title);
        }

        assert_eq!(
            config.extra_headers.get("X-Title"),
            Some(&"My Agent".to_string())
        );
    }
}
