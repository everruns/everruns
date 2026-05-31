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
use everruns_core::{Caller, LlmProviderStatus, LlmProviderType, Permission, Policy, Rule};
use reqwest::Url;
use std::sync::Arc;
use tracing::error;
use uuid::Uuid;

use crate::api::llm_providers::{CreateLlmProviderRequest, UpdateLlmProviderRequest};

pub const LLM_PROVIDER_VIEW: Policy = Policy {
    id: "llm_provider.view",
    rules: &[Rule::UserHasPermission(Permission::OrgLlmProvidersView)],
};
pub const LLM_PROVIDER_MANAGE: Policy = Policy {
    id: "llm_provider.manage",
    rules: &[Rule::UserHasPermission(Permission::OrgLlmProvidersManage)],
};

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

    pub async fn create(
        &self,
        caller: &Caller,
        req: CreateLlmProviderRequest,
    ) -> Result<LlmProvider> {
        validate_provider_base_url(req.provider_type.clone(), req.base_url.as_deref())?;

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

        let row = self.db.create_llm_provider(caller.org_id, input).await?;
        self.invalidate_resolver_cache(caller.org_id).await;
        Ok(Self::row_to_provider(&row))
    }

    pub async fn get(&self, caller: &Caller, id: Uuid) -> Result<Option<LlmProvider>> {
        // EVE-417: log the underlying DB error with org/provider context so
        // operators can diagnose the org-scoped read failures that surface
        // through MCP as a structured `internal: <message>` (and previously as
        // an opaque `callback failed`). The error still propagates unchanged.
        let row = self
            .db
            .get_llm_provider(caller.org_id, id)
            .await
            .inspect_err(|err| {
                error!(
                    org_id = caller.org_id,
                    provider_id = %id,
                    error = %err,
                    "Failed to read llm provider"
                );
            })?;
        Ok(row.as_ref().map(Self::row_to_provider))
    }

    pub async fn list(&self, caller: &Caller) -> Result<Vec<LlmProvider>> {
        // EVE-417: same diagnostic logging as `get`. List failures here are
        // the most common path for org-scoped MCP read regressions.
        let rows = self
            .db
            .list_llm_providers(caller.org_id)
            .await
            .inspect_err(|err| {
                error!(
                    org_id = caller.org_id,
                    error = %err,
                    "Failed to list llm providers"
                );
            })?;
        Ok(rows.iter().map(Self::row_to_provider).collect())
    }

    pub async fn update(
        &self,
        caller: &Caller,
        id: Uuid,
        req: UpdateLlmProviderRequest,
    ) -> Result<Option<LlmProvider>> {
        let existing = match self.db.get_llm_provider(caller.org_id, id).await? {
            Some(row) => row,
            None => return Ok(None),
        };

        let provider_type = req.provider_type.clone().unwrap_or_else(|| {
            existing
                .provider_type
                .parse()
                .unwrap_or(LlmProviderType::Openai)
        });
        let base_url = req.base_url.as_deref().or(existing.base_url.as_deref());
        validate_provider_base_url(provider_type, base_url)?;

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

        let row = self
            .db
            .update_llm_provider(caller.org_id, id, input)
            .await?;
        if row.is_some() {
            self.invalidate_resolver_cache(caller.org_id).await;
        }
        Ok(row.as_ref().map(Self::row_to_provider))
    }

    pub async fn delete(&self, caller: &Caller, id: Uuid) -> Result<bool> {
        let deleted = self.db.delete_llm_provider(caller.org_id, id).await?;
        if deleted {
            self.invalidate_resolver_cache(caller.org_id).await;
        }
        Ok(deleted)
    }

    fn row_to_provider(row: &LlmProviderRow) -> LlmProvider {
        let provider_type = row.provider_type.parse().unwrap_or(LlmProviderType::Openai);
        let api_key_set = row.api_key_set;

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

fn validate_provider_base_url(
    provider_type: LlmProviderType,
    base_url: Option<&str>,
) -> Result<()> {
    let parsed = match base_url {
        Some(url) => Some(validate_safe_url(url).map_err(|e| anyhow!("Invalid base URL: {e}"))?),
        None => None,
    };

    if provider_type == LlmProviderType::AzureOpenai {
        let parsed = parsed.ok_or_else(|| {
            anyhow!(
                "Invalid base URL: Azure OpenAI providers require a base URL ending in /openai/v1 on an Azure host"
            )
        })?;
        validate_azure_openai_base_url(&parsed).map_err(|e| anyhow!("Invalid base URL: {e}"))?;
    }

    Ok(())
}

fn validate_azure_openai_base_url(url: &Url) -> Result<()> {
    let host = url
        .host_str()
        .ok_or_else(|| anyhow!("Azure OpenAI base URL must include a host"))?
        .to_ascii_lowercase();

    let valid_host =
        host.ends_with(".openai.azure.com") || host.ends_with(".services.ai.azure.com");
    if !valid_host {
        return Err(anyhow!(
            "Azure OpenAI base URL must use *.openai.azure.com or *.services.ai.azure.com"
        ));
    }

    let path = url.path().trim_end_matches('/');
    let valid_path = matches!(path, "/openai/v1" | "/openai/v1/responses");
    if !valid_path {
        return Err(anyhow!(
            "Azure OpenAI base URL must point to /openai/v1 or /openai/v1/responses"
        ));
    }

    Ok(())
}

#[cfg(test)]
mod tests {
    use super::validate_provider_base_url;
    use everruns_core::LlmProviderType;
    use everruns_core::url_validation::validate_safe_url;

    // ---- SSRF prevention tests (EVE-69) ----

    #[test]
    fn create_rejects_localhost_base_url() {
        let url = "https://127.0.0.1/v1";
        let err = validate_safe_url(url).unwrap_err();
        let msg = format!("{err}");
        assert!(
            msg.contains("Blocked host"),
            "expected blocked host error, got: {msg}"
        );
    }

    #[test]
    fn create_rejects_private_ip_base_url() {
        let url = "https://10.0.0.1/v1";
        let err = validate_safe_url(url).unwrap_err();
        let msg = format!("{err}");
        assert!(
            msg.contains("Blocked host"),
            "expected blocked host error, got: {msg}"
        );
    }

    #[test]
    fn create_rejects_metadata_endpoint_base_url() {
        let url = "https://169.254.169.254/latest/meta-data/";
        let err = validate_safe_url(url).unwrap_err();
        let msg = format!("{err}");
        assert!(
            msg.contains("Blocked host"),
            "expected blocked host error, got: {msg}"
        );
    }

    #[test]
    fn create_accepts_valid_public_base_url() {
        assert!(validate_safe_url("https://api.openai.com/v1").is_ok());
        assert!(validate_safe_url("https://api.anthropic.com/v1").is_ok());
        assert!(validate_safe_url("https://resource.openai.azure.com/openai/v1").is_ok());
    }

    #[test]
    fn azure_openai_requires_base_url() {
        let err = validate_provider_base_url(LlmProviderType::AzureOpenai, None).unwrap_err();
        assert!(
            err.to_string()
                .contains("Invalid base URL: Azure OpenAI providers require a base URL")
        );
    }

    #[test]
    fn azure_openai_rejects_non_azure_hosts() {
        let err = validate_provider_base_url(
            LlmProviderType::AzureOpenai,
            Some("https://api.openai.com/v1"),
        )
        .unwrap_err();
        assert!(err.to_string().contains("*.openai.azure.com"));
    }

    #[test]
    fn azure_openai_accepts_supported_domains() {
        assert!(
            validate_provider_base_url(
                LlmProviderType::AzureOpenai,
                Some("https://resource.openai.azure.com/openai/v1"),
            )
            .is_ok()
        );
        assert!(
            validate_provider_base_url(
                LlmProviderType::AzureOpenai,
                Some("https://resource.services.ai.azure.com/openai/v1/responses"),
            )
            .is_ok()
        );
    }

    #[test]
    fn azure_openai_rejects_chat_completions_base_url() {
        let err = validate_provider_base_url(
            LlmProviderType::AzureOpenai,
            Some("https://resource.openai.azure.com/openai/v1/chat/completions"),
        )
        .unwrap_err();
        assert!(
            err.to_string()
                .contains("/openai/v1 or /openai/v1/responses")
        );
    }
}
