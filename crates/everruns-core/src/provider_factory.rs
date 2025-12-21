// LLM Provider Factory
//
// Factory for creating LlmProvider instances based on provider type and configuration.
// This enables dynamic provider selection at runtime based on model/provider configuration.

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
    Ollama,
    Custom,
}

impl std::str::FromStr for ProviderType {
    type Err = String;

    fn from_str(s: &str) -> std::result::Result<Self, Self::Err> {
        match s.to_lowercase().as_str() {
            "openai" => Ok(ProviderType::OpenAI),
            "anthropic" => Ok(ProviderType::Anthropic),
            "azure_openai" => Ok(ProviderType::AzureOpenAI),
            "ollama" => Ok(ProviderType::Ollama),
            "custom" => Ok(ProviderType::Custom),
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
            ProviderType::Ollama => write!(f, "ollama"),
            ProviderType::Custom => write!(f, "custom"),
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
/// If no API key is provided, falls back to environment variables.
pub fn create_provider(config: &ProviderConfig) -> Result<BoxedLlmProvider> {
    match config.provider_type {
        ProviderType::OpenAI | ProviderType::AzureOpenAI => {
            let provider = match (&config.api_key, &config.base_url) {
                (Some(key), Some(url)) => OpenAIProtocolLlmProvider::with_base_url(key, url),
                (Some(key), None) => OpenAIProtocolLlmProvider::new(key),
                (None, Some(url)) => {
                    // Get key from env since none was provided
                    let key = std::env::var("OPENAI_API_KEY").map_err(|_| {
                        AgentLoopError::llm("OPENAI_API_KEY environment variable not set")
                    })?;
                    OpenAIProtocolLlmProvider::with_base_url(key, url)
                }
                (None, None) => OpenAIProtocolLlmProvider::from_env()?,
            };
            Ok(Box::new(provider))
        }
        ProviderType::Anthropic => {
            let provider = match (&config.api_key, &config.base_url) {
                (Some(key), Some(url)) => AnthropicLlmProvider::with_base_url(key, url),
                (Some(key), None) => AnthropicLlmProvider::new(key),
                (None, Some(url)) => {
                    let key = std::env::var("ANTHROPIC_API_KEY").map_err(|_| {
                        AgentLoopError::llm("ANTHROPIC_API_KEY environment variable not set")
                    })?;
                    AnthropicLlmProvider::with_base_url(key, url)
                }
                (None, None) => AnthropicLlmProvider::from_env()?,
            };
            Ok(Box::new(provider))
        }
        ProviderType::Ollama => {
            // Ollama uses OpenAI-compatible API
            let api_key = config
                .api_key
                .clone()
                .unwrap_or_else(|| "ollama".to_string());
            let base_url = config
                .base_url
                .clone()
                .unwrap_or_else(|| "http://localhost:11434/v1/chat/completions".to_string());
            let provider = OpenAIProtocolLlmProvider::with_base_url(api_key, base_url);
            Ok(Box::new(provider))
        }
        ProviderType::Custom => {
            // Custom providers use OpenAI-compatible API with required base_url
            let base_url = config
                .base_url
                .as_ref()
                .ok_or_else(|| AgentLoopError::llm("Custom provider requires base_url"))?;
            let api_key = config
                .api_key
                .clone()
                .or_else(|| std::env::var("CUSTOM_LLM_API_KEY").ok())
                .unwrap_or_else(|| "custom".to_string());
            let provider = OpenAIProtocolLlmProvider::with_base_url(api_key, base_url);
            Ok(Box::new(provider))
        }
    }
}

/// Create a provider from environment variables based on provider type
pub fn create_provider_from_env(provider_type: &str) -> Result<BoxedLlmProvider> {
    let ptype: ProviderType = provider_type
        .parse()
        .map_err(|e: String| AgentLoopError::llm(e))?;

    create_provider(&ProviderConfig::new(ptype))
}

/// Create the default provider (OpenAI from environment)
pub fn create_default_provider() -> Result<BoxedLlmProvider> {
    create_provider(&ProviderConfig::new(ProviderType::OpenAI))
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
        assert_eq!(
            "ollama".parse::<ProviderType>().unwrap(),
            ProviderType::Ollama
        );
        assert_eq!(
            "custom".parse::<ProviderType>().unwrap(),
            ProviderType::Custom
        );
    }

    #[test]
    fn test_provider_type_display() {
        assert_eq!(ProviderType::OpenAI.to_string(), "openai");
        assert_eq!(ProviderType::Anthropic.to_string(), "anthropic");
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
}
