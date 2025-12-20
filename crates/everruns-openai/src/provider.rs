// OpenAI Provider Implementation
//
// This module provides OpenAiProvider as a wrapper around the core
// OpenAIProtocolLlmProvider implementation.

use anyhow::Result;

/// OpenAI LLM provider
///
/// This is a thin wrapper around `everruns_core::openai::OpenAIProtocolLlmProvider`
/// that provides backward compatibility with the existing API.
pub struct OpenAiProvider {
    inner: everruns_core::openai::OpenAIProtocolLlmProvider,
}

impl OpenAiProvider {
    /// Create a new OpenAI provider
    /// Requires OPENAI_API_KEY environment variable
    pub fn new() -> Result<Self> {
        let inner = everruns_core::openai::OpenAIProtocolLlmProvider::from_env()
            .map_err(|e| anyhow::anyhow!("{}", e))?;
        Ok(Self { inner })
    }

    /// Create a new OpenAI provider with a custom API key
    pub fn with_api_key(api_key: String) -> Self {
        Self {
            inner: everruns_core::openai::OpenAIProtocolLlmProvider::new(api_key),
        }
    }

    /// Create a new OpenAI provider with a custom API key and base URL
    pub fn with_base_url(api_key: String, base_url: String) -> Self {
        Self {
            inner: everruns_core::openai::OpenAIProtocolLlmProvider::with_base_url(
                api_key, base_url,
            ),
        }
    }

    /// Get a reference to the inner provider
    pub fn inner(&self) -> &everruns_core::openai::OpenAIProtocolLlmProvider {
        &self.inner
    }
}

impl Default for OpenAiProvider {
    fn default() -> Self {
        Self::new().expect("Failed to create OpenAI provider")
    }
}

// Delegate LlmProvider implementation to inner
use async_trait::async_trait;
use everruns_core::llm::{LlmCallConfig, LlmMessage, LlmProvider, LlmResponseStream};

#[async_trait]
impl LlmProvider for OpenAiProvider {
    async fn chat_completion_stream(
        &self,
        messages: Vec<LlmMessage>,
        config: &LlmCallConfig,
    ) -> everruns_core::error::Result<LlmResponseStream> {
        self.inner.chat_completion_stream(messages, config).await
    }
}

impl std::fmt::Debug for OpenAiProvider {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("OpenAiProvider")
            .field("inner", &self.inner)
            .finish()
    }
}
