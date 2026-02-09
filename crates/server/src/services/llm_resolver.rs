// LLM Resolver service for resolving models with decrypted provider credentials
//
// This service handles the resolution of LLM models with their provider credentials,
// including API key decryption. Used by gRPC service for worker communication.
//
// API key resolution order (handled at service layer):
// 1. Decrypted key from database (if set)
// 2. Environment variable fallback (for development convenience):
//    - openai: DEFAULT_OPENAI_API_KEY
//    - anthropic: DEFAULT_ANTHROPIC_API_KEY

use crate::storage::{EncryptionService, StorageBackend, models::LlmProviderRow};
use anyhow::Result;
use everruns_core::DEFAULT_ORG_ID;
use std::sync::Arc;
use uuid::Uuid;

/// Get default API key from environment variable based on provider type.
///
/// Environment variables (for development convenience):
/// - DEFAULT_OPENAI_API_KEY: Fallback API key for OpenAI providers
/// - DEFAULT_ANTHROPIC_API_KEY: Fallback API key for Anthropic providers
/// - DEFAULT_OPENROUTER_API_KEY: Fallback API key for OpenRouter providers
///
/// These are only used when the provider doesn't have an API key set in the database.
pub fn get_default_api_key_from_env(provider_type: &str) -> Option<String> {
    get_default_api_key_with_lookup(provider_type, |name| std::env::var(name).ok())
}

/// Testable version with injectable env lookup.
fn get_default_api_key_with_lookup<F>(provider_type: &str, env_lookup: F) -> Option<String>
where
    F: Fn(&str) -> Option<String>,
{
    let env_var = match provider_type.to_lowercase().as_str() {
        "openai" => "DEFAULT_OPENAI_API_KEY",
        "anthropic" => "DEFAULT_ANTHROPIC_API_KEY",
        "openrouter" => "DEFAULT_OPENROUTER_API_KEY",
        _ => return None,
    };

    env_lookup(env_var).filter(|s| !s.is_empty())
}

/// Resolved model with provider credentials (decrypted API key)
///
/// This is the service-layer representation of a model with its provider details.
/// Used for internal communication (gRPC) where decrypted credentials are needed.
#[derive(Debug, Clone)]
pub struct ResolvedModel {
    /// The model identifier (e.g., "gpt-4", "claude-3-opus")
    pub model_id: String,
    /// Provider type (e.g., "openai", "anthropic")
    pub provider_type: String,
    /// Decrypted API key (if available)
    pub api_key: Option<String>,
    /// Provider base URL override (if set)
    pub base_url: Option<String>,
}

pub struct LlmResolverService {
    db: Arc<StorageBackend>,
    encryption: Option<Arc<EncryptionService>>,
}

impl LlmResolverService {
    pub fn new(db: Arc<StorageBackend>, encryption: Option<Arc<EncryptionService>>) -> Self {
        Self { db, encryption }
    }

    /// Resolve a model by ID with decrypted provider credentials
    pub async fn resolve_model(&self, model_id: Uuid) -> Result<Option<ResolvedModel>> {
        // Look up the model
        let model_row = self.db.get_llm_model(model_id).await?;

        let model_row = match model_row {
            Some(row) => row,
            None => return Ok(None),
        };

        // Look up the provider
        let provider_row = self
            .db
            .get_llm_provider(model_row.provider_id.uuid())
            .await?;

        let provider_row = match provider_row {
            Some(row) => row,
            None => return Ok(None),
        };

        // Try to decrypt API key if encryption is available and provider has encrypted key
        let api_key = self.resolve_api_key(&provider_row)?;

        Ok(Some(ResolvedModel {
            model_id: model_row.model_id,
            provider_type: provider_row.provider_type.clone(),
            api_key,
            base_url: provider_row.base_url.clone(),
        }))
    }

    /// Resolve the default model with decrypted provider credentials
    pub async fn resolve_default_model(&self) -> Result<Option<ResolvedModel>> {
        // Look up the default model
        // TODO: Get org_id from context after Phase 3
        let model_row = self.db.get_default_llm_model(DEFAULT_ORG_ID).await?;

        let model_row = match model_row {
            Some(row) => row,
            None => return Ok(None),
        };

        // Look up the provider
        let provider_row = self
            .db
            .get_llm_provider(model_row.provider_id.uuid())
            .await?;

        let provider_row = match provider_row {
            Some(row) => row,
            None => return Ok(None),
        };

        // Try to decrypt API key if encryption is available and provider has encrypted key
        let api_key = self.resolve_api_key(&provider_row)?;

        Ok(Some(ResolvedModel {
            model_id: model_row.model_id,
            provider_type: provider_row.provider_type.clone(),
            api_key,
            base_url: provider_row.base_url.clone(),
        }))
    }

    /// Resolve API key for a provider
    ///
    /// Resolution order:
    /// 1. Decrypt from database if encryption is available and key is set
    /// 2. Fall back to environment variable (DEFAULT_OPENAI_API_KEY or DEFAULT_ANTHROPIC_API_KEY)
    ///
    /// If provider has an encrypted key but encryption service is not available,
    /// logs a warning and falls back to environment variable.
    fn resolve_api_key(&self, provider: &LlmProviderRow) -> Result<Option<String>> {
        // If provider has an encrypted API key, try to decrypt it
        if provider.api_key_encrypted.is_some() {
            if let Some(ref encryption) = self.encryption {
                let provider_with_key = self.db.get_provider_with_api_key(provider, encryption)?;
                if provider_with_key.api_key.is_some() {
                    return Ok(provider_with_key.api_key);
                }
            } else {
                // Provider has encrypted key but no encryption service
                tracing::warn!(
                    provider_id = %provider.id,
                    provider_type = %provider.provider_type,
                    "Provider has encrypted API key but encryption service is not configured. \
                     Falling back to environment variable."
                );
            }
        }

        // Fall back to environment variable
        Ok(get_default_api_key_from_env(&provider.provider_type))
    }

    /// Check if encryption service is available
    pub fn has_encryption(&self) -> bool {
        self.encryption.is_some()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn mock_env<'a>(vars: &'a [(&'a str, &'a str)]) -> impl Fn(&str) -> Option<String> + 'a {
        move |name| {
            vars.iter()
                .find(|(k, _)| *k == name)
                .map(|(_, v)| v.to_string())
        }
    }

    #[test]
    fn test_get_default_api_key_openai() {
        // Not set
        assert_eq!(
            get_default_api_key_with_lookup("openai", mock_env(&[])),
            None
        );
        assert_eq!(
            get_default_api_key_with_lookup("OpenAI", mock_env(&[])),
            None
        );

        // Set
        let env = mock_env(&[("DEFAULT_OPENAI_API_KEY", "sk-test-key")]);
        assert_eq!(
            get_default_api_key_with_lookup("openai", &env),
            Some("sk-test-key".to_string())
        );
        assert_eq!(
            get_default_api_key_with_lookup("OpenAI", &env),
            Some("sk-test-key".to_string())
        );
    }

    #[test]
    fn test_get_default_api_key_anthropic() {
        // Not set
        assert_eq!(
            get_default_api_key_with_lookup("anthropic", mock_env(&[])),
            None
        );

        // Set
        let env = mock_env(&[("DEFAULT_ANTHROPIC_API_KEY", "sk-ant-test-key")]);
        assert_eq!(
            get_default_api_key_with_lookup("anthropic", &env),
            Some("sk-ant-test-key".to_string())
        );
        assert_eq!(
            get_default_api_key_with_lookup("Anthropic", &env),
            Some("sk-ant-test-key".to_string())
        );
    }

    #[test]
    fn test_get_default_api_key_unknown_provider() {
        let env = mock_env(&[
            ("DEFAULT_OPENAI_API_KEY", "sk-test"),
            ("DEFAULT_ANTHROPIC_API_KEY", "sk-ant-test"),
        ]);
        // Unknown providers and providers without defaults return None
        assert_eq!(get_default_api_key_with_lookup("unknown", &env), None);
        assert_eq!(
            get_default_api_key_with_lookup("openai_completions", &env),
            None
        );
    }

    #[test]
    fn test_get_default_api_key_empty_value() {
        let env = mock_env(&[("DEFAULT_OPENAI_API_KEY", "")]);
        assert_eq!(get_default_api_key_with_lookup("openai", &env), None);
    }
}
