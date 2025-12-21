// Database-backed adapters for core traits
//
// These implementations are now in everruns-storage.
// This file re-exports them for backward compatibility.

pub use everruns_storage::{create_db_message_store, DbMessageStore};

// Provider factory helper for creating LLM providers
use everruns_core::{
    provider_factory::{create_provider, BoxedLlmProvider, ProviderConfig, ProviderType},
    AgentLoopError, Result,
};

/// Create an LLM provider based on configuration
///
/// This factory supports all provider types: OpenAI, Anthropic, Azure, Ollama, and Custom.
pub fn create_llm_provider(
    provider_type: &str,
    api_key: Option<&str>,
    base_url: Option<&str>,
) -> Result<BoxedLlmProvider> {
    let ptype: ProviderType = provider_type
        .parse()
        .map_err(|e: String| AgentLoopError::llm(e))?;

    let mut config = ProviderConfig::new(ptype);
    if let Some(key) = api_key {
        config = config.with_api_key(key);
    }
    if let Some(url) = base_url {
        config = config.with_base_url(url);
    }

    create_provider(&config)
}
