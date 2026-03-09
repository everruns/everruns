// LLM Provider service for business logic
//
// On create/update/delete, the LLM resolver cache is invalidated so that
// subsequent model resolutions pick up the new provider config.

use crate::services::LlmResolverService;
use crate::storage::{
    EncryptionService, StorageBackend,
    models::{CreateLlmProviderRow, LlmProviderRow, UpdateLlmProvider},
};
use anyhow::{Result, anyhow};
use everruns_core::llm_models::LlmProvider;
use everruns_core::url_validation::validate_safe_url;
use everruns_core::{LlmProviderStatus, LlmProviderType};
use std::sync::Arc;
use uuid::Uuid;

use crate::api::llm_providers::{CreateLlmProviderRequest, UpdateLlmProviderRequest};

pub struct LlmProviderService {
    db: Arc<StorageBackend>,
    encryption: Option<Arc<EncryptionService>>,
    llm_resolver: Option<Arc<LlmResolverService>>,
}

impl LlmProviderService {
    pub fn new(db: Arc<StorageBackend>, encryption: Option<Arc<EncryptionService>>) -> Self {
        Self {
            db,
            encryption,
            llm_resolver: None,
        }
    }

    pub fn with_resolver(
        db: Arc<StorageBackend>,
        encryption: Option<Arc<EncryptionService>>,
        resolver: Arc<LlmResolverService>,
    ) -> Self {
        Self {
            db,
            encryption,
            llm_resolver: Some(resolver),
        }
    }

    /// Invalidate resolver cache after provider mutation.
    async fn invalidate_resolver_cache(&self, org_id: i64) {
        if let Some(ref resolver) = self.llm_resolver {
            resolver.invalidate_cache(org_id).await;
        }
    }

    pub async fn create(&self, org_id: i64, req: CreateLlmProviderRequest) -> Result<LlmProvider> {
        // SSRF prevention: validate base_url before persisting (EVE-69)
        if let Some(ref url) = req.base_url {
            validate_safe_url(url).map_err(|e| anyhow!("Invalid base URL: {e}"))?;
        }

        // Encrypt API key if provided
        let api_key_encrypted = if let Some(api_key) = &req.api_key {
            let encryption = self
                .encryption
                .as_ref()
                .ok_or_else(|| anyhow!("Encryption not configured. Cannot store API key."))?;
            Some(encryption.encrypt_string(api_key)?)
        } else {
            None
        };

        let input = CreateLlmProviderRow {
            name: req.name,
            provider_type: req.provider_type.to_string(),
            base_url: req.base_url,
            api_key_encrypted,
            settings: None,
        };

        let row = self.db.create_llm_provider(org_id, input).await?;
        self.invalidate_resolver_cache(org_id).await;
        Ok(Self::row_to_provider(&row))
    }

    pub async fn get(&self, org_id: i64, id: Uuid) -> Result<Option<LlmProvider>> {
        let row = self.db.get_llm_provider(org_id, id).await?;
        Ok(row.as_ref().map(Self::row_to_provider))
    }

    pub async fn list(&self, org_id: i64) -> Result<Vec<LlmProvider>> {
        let rows = self.db.list_llm_providers(org_id).await?;
        Ok(rows.iter().map(Self::row_to_provider).collect())
    }

    pub async fn update(
        &self,
        org_id: i64,
        id: Uuid,
        req: UpdateLlmProviderRequest,
    ) -> Result<Option<LlmProvider>> {
        // SSRF prevention: validate base_url before persisting (EVE-69)
        if let Some(ref url) = req.base_url {
            validate_safe_url(url).map_err(|e| anyhow!("Invalid base URL: {e}"))?;
        }

        // Encrypt API key if provided
        let api_key_encrypted = if let Some(api_key) = &req.api_key {
            let encryption = self
                .encryption
                .as_ref()
                .ok_or_else(|| anyhow!("Encryption not configured. Cannot store API key."))?;
            Some(encryption.encrypt_string(api_key)?)
        } else {
            None
        };

        let input = UpdateLlmProvider {
            name: req.name,
            provider_type: req.provider_type.map(|t| t.to_string()),
            base_url: req.base_url,
            api_key_encrypted,
            status: req.status.map(|s| match s {
                LlmProviderStatus::Active => "active".to_string(),
                LlmProviderStatus::Disabled => "disabled".to_string(),
            }),
            settings: None,
        };

        let row = self.db.update_llm_provider(org_id, id, input).await?;
        if row.is_some() {
            self.invalidate_resolver_cache(org_id).await;
        }
        Ok(row.as_ref().map(Self::row_to_provider))
    }

    pub async fn delete(&self, org_id: i64, id: Uuid) -> Result<bool> {
        let deleted = self.db.delete_llm_provider(org_id, id).await?;
        if deleted {
            self.invalidate_resolver_cache(org_id).await;
        }
        Ok(deleted)
    }

    fn row_to_provider(row: &LlmProviderRow) -> LlmProvider {
        let provider_type = row.provider_type.parse().unwrap_or(LlmProviderType::Openai);
        // api_key_set is true if either:
        // 1. The API key is set in the database, OR
        // 2. A DEFAULT_ environment variable is available for this provider type
        let api_key_set = row.api_key_set || has_default_api_key_from_env(&row.provider_type);

        LlmProvider {
            id: row.id,
            name: row.name.clone(),
            provider_type,
            base_url: row.base_url.clone(),
            api_key_set,
            status: match row.status.as_str() {
                "active" => LlmProviderStatus::Active,
                _ => LlmProviderStatus::Disabled,
            },
            last_synced_at: row.last_synced_at,
            created_at: row.created_at,
            updated_at: row.updated_at,
        }
    }
}

/// Check if a default API key is available from environment variable.
///
/// Environment variables (for development convenience):
/// - DEFAULT_OPENAI_API_KEY: Fallback API key for OpenAI providers
/// - DEFAULT_ANTHROPIC_API_KEY: Fallback API key for Anthropic providers
fn has_default_api_key_from_env(provider_type: &str) -> bool {
    let env_var = match provider_type.to_lowercase().as_str() {
        "openai" => "DEFAULT_OPENAI_API_KEY",
        "anthropic" => "DEFAULT_ANTHROPIC_API_KEY",
        "gemini" => "DEFAULT_GEMINI_API_KEY",
        _ => return false,
    };

    std::env::var(env_var)
        .ok()
        .filter(|s| !s.is_empty())
        .is_some()
}

#[cfg(test)]
mod tests {
    use everruns_core::url_validation::validate_safe_url;

    // ---- SSRF prevention tests (EVE-69) ----

    #[test]
    fn create_rejects_localhost_base_url() {
        let url = "https://127.0.0.1/v1";
        let err = validate_safe_url(url).unwrap_err();
        let msg = format!("Invalid base URL: {err}");
        assert!(msg.contains("Invalid base URL"));
        assert!(msg.contains("blocked address"));
    }

    #[test]
    fn create_rejects_private_ip_base_url() {
        let url = "https://10.0.0.1/v1";
        let err = validate_safe_url(url).unwrap_err();
        let msg = format!("Invalid base URL: {err}");
        assert!(msg.contains("private network"));
    }

    #[test]
    fn create_rejects_metadata_endpoint_base_url() {
        let url = "https://169.254.169.254/latest/meta-data/";
        let err = validate_safe_url(url).unwrap_err();
        let msg = format!("Invalid base URL: {err}");
        assert!(msg.contains("blocked address"));
    }

    #[test]
    fn create_accepts_valid_public_base_url() {
        assert!(validate_safe_url("https://api.openai.com/v1").is_ok());
        assert!(validate_safe_url("https://api.anthropic.com/v1").is_ok());
    }

    #[test]
    fn create_rejects_http_base_url() {
        let err = validate_safe_url("http://api.example.com/v1").unwrap_err();
        let msg = format!("Invalid base URL: {err}");
        assert!(msg.contains("https"));
    }

    /// Testable version with injectable env lookup (test-only).
    fn has_default_api_key_with_lookup<F>(provider_type: &str, env_lookup: F) -> bool
    where
        F: Fn(&str) -> Option<String>,
    {
        let env_var = match provider_type.to_lowercase().as_str() {
            "openai" => "DEFAULT_OPENAI_API_KEY",
            "anthropic" => "DEFAULT_ANTHROPIC_API_KEY",
            "gemini" => "DEFAULT_GEMINI_API_KEY",
            _ => return false,
        };

        env_lookup(env_var).filter(|s| !s.is_empty()).is_some()
    }

    fn mock_env<'a>(vars: &'a [(&'a str, &'a str)]) -> impl Fn(&str) -> Option<String> + 'a {
        move |name| {
            vars.iter()
                .find(|(k, _)| *k == name)
                .map(|(_, v)| v.to_string())
        }
    }

    #[test]
    fn test_has_default_api_key_openai() {
        // Not set
        assert!(!has_default_api_key_with_lookup("openai", mock_env(&[])));
        assert!(!has_default_api_key_with_lookup("OpenAI", mock_env(&[])));

        // Set
        let env = mock_env(&[("DEFAULT_OPENAI_API_KEY", "sk-test-key")]);
        assert!(has_default_api_key_with_lookup("openai", &env));
        assert!(has_default_api_key_with_lookup("OpenAI", &env));
    }

    #[test]
    fn test_has_default_api_key_anthropic() {
        // Not set
        assert!(!has_default_api_key_with_lookup("anthropic", mock_env(&[])));

        // Set
        let env = mock_env(&[("DEFAULT_ANTHROPIC_API_KEY", "sk-ant-test-key")]);
        assert!(has_default_api_key_with_lookup("anthropic", &env));
        assert!(has_default_api_key_with_lookup("Anthropic", &env));
    }

    #[test]
    fn test_has_default_api_key_unknown_provider() {
        let env = mock_env(&[
            ("DEFAULT_OPENAI_API_KEY", "sk-test"),
            ("DEFAULT_ANTHROPIC_API_KEY", "sk-ant-test"),
        ]);
        // Unknown providers and providers without defaults return false
        assert!(!has_default_api_key_with_lookup("unknown", &env));
        assert!(!has_default_api_key_with_lookup("openai_completions", &env));
    }

    #[test]
    fn test_has_default_api_key_empty_value() {
        let env = mock_env(&[("DEFAULT_OPENAI_API_KEY", "")]);
        assert!(!has_default_api_key_with_lookup("openai", &env));
    }
}
