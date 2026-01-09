// LLM Providers configuration loading
//
// Loads providers from TOML config file or uses built-in defaults.
// Config providers are read-only and use deterministic UUIDs (UUID v5).

use anyhow::{Context, Result};
use chrono::Utc;
use everruns_core::llm_models::LlmProvider;
use everruns_core::{
    get_model_profile, LlmModel, LlmModelStatus, LlmModelWithProvider, LlmProviderStatus,
    LlmProviderType,
};
use serde::Deserialize;
use std::path::Path;
use uuid::Uuid;

/// Built-in default providers configuration (embedded in binary)
const DEFAULT_PROVIDERS_CONFIG: &str = include_str!("../../../../config/providers.toml");

/// Namespace UUID for generating deterministic provider/model IDs
/// This is a random UUID used as the namespace for UUID v5 generation
const PROVIDER_NAMESPACE: Uuid = Uuid::from_bytes([
    0x6b, 0xa7, 0xb8, 0x10, 0x9d, 0xad, 0x11, 0xd1, 0x80, 0xb4, 0x00, 0xc0, 0x4f, 0xd4, 0x30, 0xc8,
]);

/// Model config from TOML file
#[derive(Debug, Deserialize)]
pub struct ModelConfig {
    pub model_id: String,
    pub display_name: String,
    #[serde(default)]
    pub is_default: bool,
    #[serde(default)]
    pub capabilities: Vec<String>,
}

/// Provider config from TOML file
#[derive(Debug, Deserialize)]
pub struct ProviderConfig {
    pub name: String,
    pub provider_type: String,
    #[serde(default)]
    pub is_default: bool,
    pub base_url: Option<String>,
    #[serde(default)]
    pub models: Vec<ModelConfig>,
}

/// Root config structure
#[derive(Debug, Deserialize)]
pub struct ProvidersConfigFile {
    #[serde(default)]
    pub providers: Vec<ProviderConfig>,
}

/// Parsed providers configuration with domain types
#[derive(Debug, Clone)]
pub struct ProvidersConfig {
    pub providers: Vec<LlmProvider>,
    pub models: Vec<LlmModelWithProvider>,
}

impl ProvidersConfig {
    /// Get all config providers
    pub fn providers(&self) -> &[LlmProvider] {
        &self.providers
    }

    /// Get all config models
    pub fn models(&self) -> &[LlmModelWithProvider] {
        &self.models
    }

    /// Get provider by ID
    pub fn get_provider(&self, id: Uuid) -> Option<&LlmProvider> {
        self.providers.iter().find(|p| p.id == id)
    }

    /// Get provider by name
    pub fn get_provider_by_name(&self, name: &str) -> Option<&LlmProvider> {
        self.providers.iter().find(|p| p.name == name)
    }

    /// Get model by ID
    pub fn get_model(&self, id: Uuid) -> Option<&LlmModelWithProvider> {
        self.models.iter().find(|m| m.id == id)
    }

    /// Get models for a provider
    pub fn get_models_for_provider(&self, provider_id: Uuid) -> Vec<&LlmModelWithProvider> {
        self.models
            .iter()
            .filter(|m| m.provider_id == provider_id)
            .collect()
    }

    /// Get the default model from config (if any)
    pub fn get_default_model(&self) -> Option<&LlmModelWithProvider> {
        self.models.iter().find(|m| m.is_default)
    }
}

/// Generate a deterministic UUID v5 for a provider based on its name
fn generate_provider_id(name: &str) -> Uuid {
    Uuid::new_v5(&PROVIDER_NAMESPACE, format!("provider:{}", name).as_bytes())
}

/// Generate a deterministic UUID v5 for a model based on provider name and model_id
fn generate_model_id(provider_name: &str, model_id: &str) -> Uuid {
    Uuid::new_v5(
        &PROVIDER_NAMESPACE,
        format!("model:{}:{}", provider_name, model_id).as_bytes(),
    )
}

/// Load providers configuration from file or use built-in defaults
///
/// If `config_path` is provided and exists, loads from that file.
/// Otherwise, uses the built-in default configuration.
pub fn load_providers_config(config_path: Option<&Path>) -> Result<ProvidersConfig> {
    let config_str = if let Some(path) = config_path {
        if path.exists() {
            tracing::info!("Loading providers config from: {}", path.display());
            std::fs::read_to_string(path)
                .with_context(|| format!("Failed to read config file: {}", path.display()))?
        } else {
            tracing::info!("Config file not found, using built-in defaults");
            DEFAULT_PROVIDERS_CONFIG.to_string()
        }
    } else {
        tracing::info!("No config path specified, using built-in defaults");
        DEFAULT_PROVIDERS_CONFIG.to_string()
    };

    parse_providers_config(&config_str)
}

/// Parse providers configuration from TOML string
fn parse_providers_config(config_str: &str) -> Result<ProvidersConfig> {
    let config_file: ProvidersConfigFile =
        toml::from_str(config_str).context("Failed to parse providers config TOML")?;

    let now = Utc::now();
    let mut providers = Vec::new();
    let mut models = Vec::new();

    for provider_config in config_file.providers {
        let provider_type: LlmProviderType = provider_config
            .provider_type
            .parse()
            .map_err(|e: String| anyhow::anyhow!(e))?;

        let provider_id = generate_provider_id(&provider_config.name);

        let provider = LlmProvider {
            id: provider_id,
            name: provider_config.name.clone(),
            provider_type: provider_type.clone(),
            base_url: provider_config.base_url,
            api_key_set: false, // Config providers don't have API keys
            status: LlmProviderStatus::Active,
            created_at: now,
            updated_at: now,
            readonly: true,
        };

        providers.push(provider);

        for model_config in provider_config.models {
            let model_id = generate_model_id(&provider_config.name, &model_config.model_id);

            // Look up profile based on provider_type and model_id
            let profile = get_model_profile(&provider_type, &model_config.model_id);

            let model = LlmModelWithProvider {
                id: model_id,
                provider_id,
                model_id: model_config.model_id,
                display_name: model_config.display_name,
                capabilities: model_config.capabilities,
                is_default: model_config.is_default,
                status: LlmModelStatus::Active,
                created_at: now,
                updated_at: now,
                provider_name: provider_config.name.clone(),
                provider_type: provider_type.clone(),
                profile,
                readonly: true,
            };

            models.push(model);
        }
    }

    tracing::info!(
        "Loaded {} providers and {} models from config",
        providers.len(),
        models.len()
    );

    Ok(ProvidersConfig { providers, models })
}

/// Convert LlmModelWithProvider to LlmModel (for provider-specific model lists)
pub fn model_with_provider_to_model(m: &LlmModelWithProvider) -> LlmModel {
    LlmModel {
        id: m.id,
        provider_id: m.provider_id,
        model_id: m.model_id.clone(),
        display_name: m.display_name.clone(),
        capabilities: m.capabilities.clone(),
        is_default: m.is_default,
        status: m.status.clone(),
        created_at: m.created_at,
        updated_at: m.updated_at,
        readonly: m.readonly,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_parse_default_config() {
        let config = parse_providers_config(DEFAULT_PROVIDERS_CONFIG)
            .expect("Failed to parse default config");

        // Should have OpenAI and Anthropic providers
        assert_eq!(config.providers.len(), 2);

        let openai = config
            .providers
            .iter()
            .find(|p| p.name == "OpenAI")
            .unwrap();
        assert!(matches!(openai.provider_type, LlmProviderType::Openai));
        assert!(openai.readonly);

        let anthropic = config
            .providers
            .iter()
            .find(|p| p.name == "Anthropic")
            .unwrap();
        assert!(matches!(
            anthropic.provider_type,
            LlmProviderType::Anthropic
        ));
        assert!(anthropic.readonly);
    }

    #[test]
    fn test_deterministic_ids() {
        let config1 = parse_providers_config(DEFAULT_PROVIDERS_CONFIG).unwrap();
        let config2 = parse_providers_config(DEFAULT_PROVIDERS_CONFIG).unwrap();

        // IDs should be deterministic
        assert_eq!(config1.providers[0].id, config2.providers[0].id);
        assert_eq!(config1.models[0].id, config2.models[0].id);
    }

    #[test]
    fn test_generate_provider_id() {
        let id1 = generate_provider_id("OpenAI");
        let id2 = generate_provider_id("OpenAI");
        let id3 = generate_provider_id("Anthropic");

        // Same name = same ID
        assert_eq!(id1, id2);
        // Different name = different ID
        assert_ne!(id1, id3);
    }

    #[test]
    fn test_generate_model_id() {
        let id1 = generate_model_id("OpenAI", "gpt-4o");
        let id2 = generate_model_id("OpenAI", "gpt-4o");
        let id3 = generate_model_id("OpenAI", "gpt-4o-mini");
        let id4 = generate_model_id("Anthropic", "gpt-4o"); // Same model_id, different provider

        // Same provider+model = same ID
        assert_eq!(id1, id2);
        // Different model = different ID
        assert_ne!(id1, id3);
        // Different provider = different ID (even with same model_id)
        assert_ne!(id1, id4);
    }

    #[test]
    fn test_config_providers_are_readonly() {
        let config = parse_providers_config(DEFAULT_PROVIDERS_CONFIG).unwrap();

        for provider in &config.providers {
            assert!(
                provider.readonly,
                "Provider {} should be readonly",
                provider.name
            );
        }

        for model in &config.models {
            assert!(
                model.readonly,
                "Model {} should be readonly",
                model.model_id
            );
        }
    }

    #[test]
    fn test_config_providers_have_no_api_key() {
        let config = parse_providers_config(DEFAULT_PROVIDERS_CONFIG).unwrap();

        for provider in &config.providers {
            assert!(
                !provider.api_key_set,
                "Provider {} should not have api_key_set",
                provider.name
            );
        }
    }

    #[test]
    fn test_default_model_in_config() {
        let config = parse_providers_config(DEFAULT_PROVIDERS_CONFIG).unwrap();

        let default_model = config.get_default_model();
        assert!(default_model.is_some(), "Should have a default model");

        let model = default_model.unwrap();
        assert_eq!(model.model_id, "gpt-5.2");
        assert!(model.is_default);
    }
}
