// LLM driver factory helpers
//
// Decision: Workers use gRPC adapters for database operations, not direct DB access.
// This module only contains LLM driver factory helpers.

use everruns_core::{
    AgentLoopError, BoxedLlmDriver, DriverRegistry, ProviderConfig, ProviderType, Result,
};
use everruns_openai::OpenAIApiMode;

/// Create and configure the driver registry with all supported LLM providers
///
/// This registers drivers for:
/// - OpenAI (and Azure OpenAI) - uses OPENAI_API_MODE env var to select API
/// - Anthropic Claude
/// - LlmSim (for testing)
///
/// # Environment Variables
///
/// - `OPENAI_API_MODE`: Controls which OpenAI API to use
///   - `responses` (default): Open Responses API (https://www.openresponses.org/)
///   - `completions`: Chat Completions API (for backward compatibility)
pub fn create_driver_registry() -> DriverRegistry {
    let mut registry = DriverRegistry::new();

    // Check OPENAI_API_MODE env var for API mode selection
    let openai_mode = match std::env::var("OPENAI_API_MODE")
        .unwrap_or_default()
        .to_lowercase()
        .as_str()
    {
        "completions" => OpenAIApiMode::Completions,
        _ => OpenAIApiMode::Responses, // Default to Responses (Open Responses spec)
    };

    everruns_openai::register_driver_with_mode(&mut registry, openai_mode);
    everruns_anthropic::register_driver(&mut registry);
    everruns_core::llmsim_driver::register_driver(&mut registry);
    registry
}

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

    let registry = create_driver_registry();
    registry.create_driver(&config)
}
