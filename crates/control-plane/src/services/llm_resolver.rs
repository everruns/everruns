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

use crate::storage::{EncryptionService, StorageBackend};
use anyhow::{Result, anyhow};
use std::sync::Arc;
use uuid::Uuid;

/// Get default API key from environment variable based on provider type.
///
/// Environment variables (for development convenience):
/// - DEFAULT_OPENAI_API_KEY: Fallback API key for OpenAI providers
/// - DEFAULT_ANTHROPIC_API_KEY: Fallback API key for Anthropic providers
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
        let encryption = match &self.encryption {
            Some(enc) => enc.as_ref().clone(),
            None => return Err(anyhow!("Encryption service not configured")),
        };

        // Look up the model
        let model_row = self.db.get_llm_model(model_id).await?;

        let model_row = match model_row {
            Some(row) => row,
            None => return Ok(None),
        };

        // Look up the provider
        let provider_row = self.db.get_llm_provider(model_row.provider_id).await?;

        let provider_row = match provider_row {
            Some(row) => row,
            None => return Ok(None),
        };

        // Decrypt the API key from database
        let provider_with_key = self
            .db
            .get_provider_with_api_key(&provider_row, &encryption)?;

        // Apply env fallback if no API key in database
        let api_key = provider_with_key
            .api_key
            .or_else(|| get_default_api_key_from_env(&provider_with_key.provider_type));

        Ok(Some(ResolvedModel {
            model_id: model_row.model_id,
            provider_type: provider_with_key.provider_type,
            api_key,
            base_url: provider_with_key.base_url,
        }))
    }

    /// Resolve the default model with decrypted provider credentials
    pub async fn resolve_default_model(&self) -> Result<Option<ResolvedModel>> {
        let encryption = match &self.encryption {
            Some(enc) => enc.as_ref().clone(),
            None => return Err(anyhow!("Encryption service not configured")),
        };

        // Look up the default model
        let model_row = self.db.get_default_llm_model().await?;

        let model_row = match model_row {
            Some(row) => row,
            None => return Ok(None),
        };

        // Look up the provider
        let provider_row = self.db.get_llm_provider(model_row.provider_id).await?;

        let provider_row = match provider_row {
            Some(row) => row,
            None => return Ok(None),
        };

        // Decrypt the API key from database
        let provider_with_key = self
            .db
            .get_provider_with_api_key(&provider_row, &encryption)?;

        // Apply env fallback if no API key in database
        let api_key = provider_with_key
            .api_key
            .or_else(|| get_default_api_key_from_env(&provider_with_key.provider_type));

        Ok(Some(ResolvedModel {
            model_id: model_row.model_id,
            provider_type: provider_with_key.provider_type,
            api_key,
            base_url: provider_with_key.base_url,
        }))
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
        assert_eq!(get_default_api_key_with_lookup("azure_openai", &env), None);
        assert_eq!(get_default_api_key_with_lookup("unknown", &env), None);
    }

    #[test]
    fn test_get_default_api_key_empty_value() {
        let env = mock_env(&[("DEFAULT_OPENAI_API_KEY", "")]);
        assert_eq!(get_default_api_key_with_lookup("openai", &env), None);
    }
}
