// OpenAI LLM Driver
//
// Production implementation for OpenAI's API.
// Supports both Chat Completions API and Open Responses API.
//
// Open Responses (https://www.openresponses.org/) is a vendor-neutral
// API specification for multi-provider LLM interfaces, recommended for
// new projects.

use async_trait::async_trait;
use chrono::TimeZone;

use everruns_core::OpenAIProtocolLlmDriver;
use everruns_core::OpenResponsesProtocolLlmDriver;
use everruns_core::error::{AgentLoopError, Result};
use everruns_core::llm_driver_registry::{
    BoxedLlmDriver, DiscoveredModel, DriverRegistry, LlmCallConfig, LlmDriver, LlmMessage,
    LlmResponseStream, ProviderType,
};

use crate::types::{OpenAIApiMode, OpenAiModelsResponse};

const OPENAI_MODELS_URL: &str = "https://api.openai.com/v1/models";

/// OpenAI LLM Driver
///
/// Production driver for OpenAI's API. Supports both:
/// - **Chat Completions API** (default): Traditional API for backward compatibility
/// - **Open Responses API**: Vendor-neutral spec (https://www.openresponses.org/),
///   recommended for new projects with better performance and multi-provider support
///
/// # Example
///
/// ```ignore
/// use everruns_openai::{OpenAILlmDriver, OpenAIApiMode};
///
/// // Default: uses Open Responses API (recommended)
/// let driver = OpenAILlmDriver::new("your-api-key");
///
/// // Explicit Responses mode
/// let driver = OpenAILlmDriver::with_mode("your-api-key", OpenAIApiMode::Responses);
///
/// // Use Completions API (for backward compatibility)
/// let driver = OpenAILlmDriver::with_mode("your-api-key", OpenAIApiMode::Completions);
///
/// // With custom endpoint
/// let driver = OpenAILlmDriver::with_base_url_and_mode(
///     "your-api-key",
///     "https://api.example.com/v1/responses",
///     OpenAIApiMode::Responses,
/// );
/// ```
#[derive(Clone)]
pub struct OpenAILlmDriver {
    inner: InnerDriver,
    mode: OpenAIApiMode,
    /// Whether using a custom base URL (not OpenAI's API)
    uses_custom_url: bool,
}

/// Inner driver enum for supporting multiple API protocols
#[derive(Clone)]
enum InnerDriver {
    Completions(OpenAIProtocolLlmDriver),
    Responses(OpenResponsesProtocolLlmDriver),
}

impl OpenAILlmDriver {
    /// Create a new driver with the given API key (uses Open Responses API by default)
    ///
    /// Open Responses is the recommended API for new projects, offering:
    /// - Better performance with reasoning models
    /// - Provider-agnostic streaming events
    /// - Native agentic loop support
    ///
    /// Use `with_mode(api_key, OpenAIApiMode::Completions)` for backward compatibility.
    pub fn new(api_key: impl Into<String>) -> Self {
        Self::with_mode(api_key, OpenAIApiMode::Responses)
    }

    /// Create a new driver with specified API mode
    pub fn with_mode(api_key: impl Into<String>, mode: OpenAIApiMode) -> Self {
        let api_key = api_key.into();
        let inner = match mode {
            OpenAIApiMode::Completions => {
                InnerDriver::Completions(OpenAIProtocolLlmDriver::new(api_key))
            }
            OpenAIApiMode::Responses => {
                InnerDriver::Responses(OpenResponsesProtocolLlmDriver::new(api_key))
            }
        };
        Self {
            inner,
            mode,
            uses_custom_url: false,
        }
    }

    /// Create a new driver from the OPENAI_API_KEY environment variable
    /// (uses Open Responses API by default)
    pub fn from_env() -> Result<Self> {
        Ok(Self {
            inner: InnerDriver::Responses(OpenResponsesProtocolLlmDriver::from_env()?),
            mode: OpenAIApiMode::Responses,
            uses_custom_url: false,
        })
    }

    /// Create a new driver from environment with specified API mode
    pub fn from_env_with_mode(mode: OpenAIApiMode) -> Result<Self> {
        let api_key = std::env::var("OPENAI_API_KEY").map_err(|_| {
            AgentLoopError::llm("OPENAI_API_KEY environment variable not set")
        })?;
        Ok(Self::with_mode(api_key, mode))
    }

    /// Create a new driver with a custom API URL (uses Completions API by default)
    pub fn with_base_url(api_key: impl Into<String>, api_url: impl Into<String>) -> Self {
        Self {
            inner: InnerDriver::Completions(OpenAIProtocolLlmDriver::with_base_url(
                api_key, api_url,
            )),
            mode: OpenAIApiMode::Completions,
            uses_custom_url: true,
        }
    }

    /// Create a new driver with a custom API URL and specified mode
    pub fn with_base_url_and_mode(
        api_key: impl Into<String>,
        api_url: impl Into<String>,
        mode: OpenAIApiMode,
    ) -> Self {
        let api_key = api_key.into();
        let api_url = api_url.into();
        let inner = match mode {
            OpenAIApiMode::Completions => {
                InnerDriver::Completions(OpenAIProtocolLlmDriver::with_base_url(api_key, api_url))
            }
            OpenAIApiMode::Responses => InnerDriver::Responses(
                OpenResponsesProtocolLlmDriver::with_base_url(api_key, api_url),
            ),
        };
        Self {
            inner,
            mode,
            uses_custom_url: true,
        }
    }

    /// Get the API URL
    pub fn api_url(&self) -> &str {
        match &self.inner {
            InnerDriver::Completions(d) => d.api_url(),
            InnerDriver::Responses(d) => d.api_url(),
        }
    }

    /// Get the current API mode
    pub fn api_mode(&self) -> OpenAIApiMode {
        self.mode
    }

    /// Check if using a custom base URL
    pub fn uses_custom_url(&self) -> bool {
        self.uses_custom_url
    }

    /// Get the API key (for internal use)
    fn api_key(&self) -> &str {
        match &self.inner {
            InnerDriver::Completions(d) => d.api_key(),
            InnerDriver::Responses(d) => d.api_key(),
        }
    }

    /// Get the HTTP client (for internal use)
    fn client(&self) -> &reqwest::Client {
        match &self.inner {
            InnerDriver::Completions(d) => d.client(),
            InnerDriver::Responses(d) => d.client(),
        }
    }
}

#[async_trait]
impl LlmDriver for OpenAILlmDriver {
    async fn chat_completion_stream(
        &self,
        messages: Vec<LlmMessage>,
        config: &LlmCallConfig,
    ) -> Result<LlmResponseStream> {
        match &self.inner {
            InnerDriver::Completions(driver) => {
                driver.chat_completion_stream(messages, config).await
            }
            InnerDriver::Responses(driver) => driver.chat_completion_stream(messages, config).await,
        }
    }

    async fn list_models(&self) -> Result<Option<Vec<DiscoveredModel>>> {
        // Skip discovery for custom URLs (proxies, self-hosted)
        if self.uses_custom_url {
            return Ok(None);
        }

        let response = self
            .client()
            .get(OPENAI_MODELS_URL)
            .bearer_auth(self.api_key())
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
            .field("api_mode", &self.mode)
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
/// By default, uses the Open Responses API (https://www.openresponses.org/)
/// which is recommended for new projects. To use the Chat Completions API
/// for backward compatibility, use `register_driver_with_mode`.
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
    register_driver_with_mode(registry, OpenAIApiMode::Responses)
}

/// Register the OpenAI driver with specified API mode
///
/// # Example
///
/// ```ignore
/// use everruns_core::DriverRegistry;
/// use everruns_openai::{register_driver_with_mode, OpenAIApiMode};
///
/// let mut registry = DriverRegistry::new();
///
/// // Use Responses API for better performance with reasoning models
/// register_driver_with_mode(&mut registry, OpenAIApiMode::Responses);
/// ```
pub fn register_driver_with_mode(registry: &mut DriverRegistry, mode: OpenAIApiMode) {
    // Register for OpenAI
    registry.register(ProviderType::OpenAI, move |api_key, base_url| {
        let driver = match base_url {
            Some(url) => OpenAILlmDriver::with_base_url_and_mode(api_key, url, mode),
            None => OpenAILlmDriver::with_mode(api_key, mode),
        };
        Box::new(driver) as BoxedLlmDriver
    });

    // Register for Azure OpenAI (uses same driver implementation)
    registry.register(ProviderType::AzureOpenAI, move |api_key, base_url| {
        let driver = match base_url {
            Some(url) => OpenAILlmDriver::with_base_url_and_mode(api_key, url, mode),
            None => OpenAILlmDriver::with_mode(api_key, mode),
        };
        Box::new(driver) as BoxedLlmDriver
    });
}
