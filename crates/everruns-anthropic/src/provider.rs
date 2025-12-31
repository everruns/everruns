// Anthropic Driver Implementation
//
// This module provides AnthropicDriver as a wrapper around the core
// AnthropicLlmProvider implementation.

use anyhow::Result;

/// Anthropic Claude LLM driver
///
/// This is a thin wrapper around `everruns_core::anthropic::AnthropicLlmProvider`
/// that provides a clean API for the worker and other components.
pub struct AnthropicDriver {
    inner: everruns_core::anthropic::AnthropicLlmProvider,
}

impl AnthropicDriver {
    /// Create a new Anthropic driver
    /// Requires ANTHROPIC_API_KEY environment variable
    pub fn new() -> Result<Self> {
        let inner = everruns_core::anthropic::AnthropicLlmProvider::from_env()
            .map_err(|e| anyhow::anyhow!("{}", e))?;
        Ok(Self { inner })
    }

    /// Create a new Anthropic driver with a custom API key
    pub fn with_api_key(api_key: String) -> Self {
        Self {
            inner: everruns_core::anthropic::AnthropicLlmProvider::new(api_key),
        }
    }

    /// Create a new Anthropic driver with a custom API key and base URL
    pub fn with_base_url(api_key: String, base_url: String) -> Self {
        Self {
            inner: everruns_core::anthropic::AnthropicLlmProvider::with_base_url(api_key, base_url),
        }
    }

    /// Get a reference to the inner driver
    pub fn inner(&self) -> &everruns_core::anthropic::AnthropicLlmProvider {
        &self.inner
    }
}

impl Default for AnthropicDriver {
    fn default() -> Self {
        Self::new().expect("Failed to create Anthropic driver")
    }
}

// Delegate LlmDriver implementation to inner
use async_trait::async_trait;
use everruns_core::llm::{LlmCallConfig, LlmDriver, LlmMessage, LlmResponseStream};

#[async_trait]
impl LlmDriver for AnthropicDriver {
    async fn chat_completion_stream(
        &self,
        messages: Vec<LlmMessage>,
        config: &LlmCallConfig,
    ) -> everruns_core::error::Result<LlmResponseStream> {
        self.inner.chat_completion_stream(messages, config).await
    }
}

impl std::fmt::Debug for AnthropicDriver {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("AnthropicDriver")
            .field("inner", &self.inner)
            .finish()
    }
}
