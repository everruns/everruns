// LLM Provider Factory
//
// Factory for creating LlmProvider instances based on provider type and configuration.
// This enables dynamic provider selection at runtime based on model/provider configuration.
//
// IMPORTANT: API keys must be provided from the database. This factory does NOT read
// from environment variables. Keys should be decrypted and passed via ProviderConfig.

use crate::anthropic::AnthropicLlmProvider;
use crate::error::{AgentLoopError, Result};
use crate::llm::LlmProvider;
use crate::openai::OpenAIProtocolLlmProvider;

/// Provider type enumeration matching the database/contracts
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ProviderType {
    OpenAI,
    Anthropic,
    AzureOpenAI,
}

impl std::str::FromStr for ProviderType {
    type Err = String;

    fn from_str(s: &str) -> std::result::Result<Self, Self::Err> {
        match s.to_lowercase().as_str() {
            "openai" => Ok(ProviderType::OpenAI),
            "anthropic" => Ok(ProviderType::Anthropic),
            "azure_openai" => Ok(ProviderType::AzureOpenAI),
            _ => Err(format!("Unknown provider type: {}", s)),
        }
    }
}

impl std::fmt::Display for ProviderType {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            ProviderType::OpenAI => write!(f, "openai"),
            ProviderType::Anthropic => write!(f, "anthropic"),
            ProviderType::AzureOpenAI => write!(f, "azure_openai"),
        }
    }
}

/// Configuration for creating an LLM provider
#[derive(Debug, Clone)]
pub struct ProviderConfig {
    /// Type of provider
    pub provider_type: ProviderType,
    /// API key for authentication
    pub api_key: Option<String>,
    /// Base URL override (optional)
    pub base_url: Option<String>,
}

impl ProviderConfig {
    /// Create a new provider config
    pub fn new(provider_type: ProviderType) -> Self {
        Self {
            provider_type,
            api_key: None,
            base_url: None,
        }
    }

    /// Set the API key
    pub fn with_api_key(mut self, api_key: impl Into<String>) -> Self {
        self.api_key = Some(api_key.into());
        self
    }

    /// Set the base URL
    pub fn with_base_url(mut self, base_url: impl Into<String>) -> Self {
        self.base_url = Some(base_url.into());
        self
    }
}

/// Boxed LLM provider for dynamic dispatch
pub type BoxedLlmProvider = Box<dyn LlmProvider>;

/// Create an LLM provider based on configuration
///
/// API keys must be provided in the config. This function does NOT fall back to
/// environment variables. Keys should be decrypted from the database and passed here.
pub fn create_provider(config: &ProviderConfig) -> Result<BoxedLlmProvider> {
    // API key is required - it should be decrypted from the database
    let api_key = config.api_key.as_ref().ok_or_else(|| {
        AgentLoopError::llm("API key is required. Configure the API key in provider settings.")
    })?;

    match config.provider_type {
        ProviderType::OpenAI | ProviderType::AzureOpenAI => {
            let provider = match &config.base_url {
                Some(url) => OpenAIProtocolLlmProvider::with_base_url(api_key, url),
                None => OpenAIProtocolLlmProvider::new(api_key),
            };
            Ok(Box::new(provider))
        }
        ProviderType::Anthropic => {
            let provider = match &config.base_url {
                Some(url) => AnthropicLlmProvider::with_base_url(api_key, url),
                None => AnthropicLlmProvider::new(api_key),
            };
            Ok(Box::new(provider))
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_provider_type_parsing() {
        assert_eq!(
            "openai".parse::<ProviderType>().unwrap(),
            ProviderType::OpenAI
        );
        assert_eq!(
            "anthropic".parse::<ProviderType>().unwrap(),
            ProviderType::Anthropic
        );
        assert_eq!(
            "azure_openai".parse::<ProviderType>().unwrap(),
            ProviderType::AzureOpenAI
        );
        // Ollama and Custom are no longer supported
        assert!("ollama".parse::<ProviderType>().is_err());
        assert!("custom".parse::<ProviderType>().is_err());
    }

    #[test]
    fn test_provider_type_display() {
        assert_eq!(ProviderType::OpenAI.to_string(), "openai");
        assert_eq!(ProviderType::Anthropic.to_string(), "anthropic");
        assert_eq!(ProviderType::AzureOpenAI.to_string(), "azure_openai");
    }

    #[test]
    fn test_provider_config_builder() {
        let config = ProviderConfig::new(ProviderType::Anthropic)
            .with_api_key("test-key")
            .with_base_url("https://custom.api.com");

        assert_eq!(config.provider_type, ProviderType::Anthropic);
        assert_eq!(config.api_key, Some("test-key".to_string()));
        assert_eq!(config.base_url, Some("https://custom.api.com".to_string()));
    }

    #[test]
    fn test_create_provider_requires_api_key() {
        // Provider without API key should fail
        let config = ProviderConfig::new(ProviderType::OpenAI);
        let result = create_provider(&config);
        assert!(result.is_err());

        // Provider with API key should succeed
        let config_with_key = ProviderConfig::new(ProviderType::OpenAI).with_api_key("test-key");
        let result = create_provider(&config_with_key);
        assert!(result.is_ok());
    }
}
