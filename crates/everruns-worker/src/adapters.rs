// Database-backed adapters for core traits
//
// These implementations are now in everruns-storage.
// This file re-exports them for backward compatibility.

pub use everruns_storage::{
    create_db_agent_store, create_db_llm_provider_store, create_db_message_store,
    create_db_session_file_store, create_db_session_store, DbAgentStore, DbLlmProviderStore,
    DbMessageStore, DbSessionFileStore, DbSessionStore,
};

// Driver factory helper for creating LLM drivers
use everruns_core::{
    create_driver, AgentLoopError, BoxedLlmDriver, ProviderConfig, ProviderType, Result,
};

/// Create an LLM driver based on configuration
///
/// This factory supports all provider types: OpenAI, Anthropic, Azure.
pub fn create_llm_driver(
    provider_type: &str,
    api_key: Option<&str>,
    base_url: Option<&str>,
) -> Result<BoxedLlmDriver> {
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

    create_driver(&config)
}
