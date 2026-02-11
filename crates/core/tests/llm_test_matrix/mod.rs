// Shared provider/model configuration for parametrized LLM integration tests.
//
// Defines ProviderModelConfig structs and a unified DriverRegistry so test
// files can iterate over providers × models without duplicating helpers.
//
// Add new providers/models here — all test files pick them up automatically.

#![allow(dead_code)] // Not all test binaries use every constant.

use everruns_core::llm_driver_registry::DriverRegistry;
use everruns_core::llm_models::LlmProviderType;
use everruns_core::traits::ModelWithProvider;

// ============================================================================
// Provider + Model configuration
// ============================================================================

/// One cell in the test matrix: a (provider, model, env-var) tuple.
#[derive(Clone, Debug)]
pub struct ProviderModelConfig {
    pub provider_type: LlmProviderType,
    pub model_name: &'static str,
    pub env_var: &'static str,
}

impl ProviderModelConfig {
    pub const fn new(
        provider_type: LlmProviderType,
        model_name: &'static str,
        env_var: &'static str,
    ) -> Self {
        Self {
            provider_type,
            model_name,
            env_var,
        }
    }

    /// Build a `ModelWithProvider` from env, returning `None` if the key is
    /// missing or empty, or if the provider appears in `SKIP_LLM_PROVIDERS`
    /// (comma-separated list, e.g. `SKIP_LLM_PROVIDERS=gemini,openai`).
    pub fn model(&self) -> Option<ModelWithProvider> {
        if let Ok(skip) = std::env::var("SKIP_LLM_PROVIDERS") {
            let provider = self.provider_type.to_string().to_lowercase();
            if skip.split(',').any(|s| s.trim().to_lowercase() == provider) {
                return None;
            }
        }
        let api_key = std::env::var(self.env_var).ok().filter(|k| !k.is_empty())?;
        Some(ModelWithProvider {
            model: self.model_name.to_string(),
            provider_type: self.provider_type.clone(),
            api_key: Some(api_key),
            base_url: None,
        })
    }

    /// Human-readable label for skip messages.
    pub fn label(&self) -> String {
        format!("{}:{}", self.env_var, self.model_name)
    }
}

impl std::fmt::Display for ProviderModelConfig {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}({})", self.provider_type, self.model_name)
    }
}

// ============================================================================
// Provider catalogue — add new providers/models here
// ============================================================================

pub const ANTHROPIC_HAIKU: ProviderModelConfig = ProviderModelConfig::new(
    LlmProviderType::Anthropic,
    "claude-haiku-4-5-20251001",
    "ANTHROPIC_API_KEY",
);

pub const ANTHROPIC_SONNET: ProviderModelConfig = ProviderModelConfig::new(
    LlmProviderType::Anthropic,
    "claude-sonnet-4-20250514",
    "ANTHROPIC_API_KEY",
);

pub const OPENAI_GPT4O_MINI: ProviderModelConfig =
    ProviderModelConfig::new(LlmProviderType::Openai, "gpt-4o-mini", "OPENAI_API_KEY");

pub const GEMINI_FLASH: ProviderModelConfig = ProviderModelConfig::new(
    LlmProviderType::Gemini,
    "gemini-2.0-flash",
    "GEMINI_API_KEY",
);

// ============================================================================
// Unified driver registry
// ============================================================================

/// Registry with all real providers registered.
pub fn all_providers_registry() -> DriverRegistry {
    let mut registry = DriverRegistry::new();
    everruns_anthropic::register_driver(&mut registry);
    everruns_openai::register_driver(&mut registry);
    everruns_gemini::register_driver(&mut registry);
    registry
}
