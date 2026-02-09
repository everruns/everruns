// Model Sync Service
//
// Handles synchronization of models from provider APIs into the database.
// Supports both manual sync (via API endpoint) and background sync (periodic).

use crate::storage::{
    StorageBackend,
    models::{CreateLlmModelRow, LlmProviderRow, UpdateLlmModel},
};
use anyhow::{Context, Result};
use chrono::Utc;
use everruns_core::{
    DiscoveredModel, DriverRegistry, LlmProviderType, ProviderConfig, ProviderId, ProviderType,
    get_model_profile,
};
use serde::{Deserialize, Serialize};
use std::sync::Arc;
use uuid::Uuid;

/// Result of a model sync operation
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "status", rename_all = "snake_case")]
pub enum SyncResult {
    /// Sync succeeded
    Success {
        /// Number of new models discovered
        created: usize,
        /// Number of existing models updated
        updated: usize,
        /// Number of models marked as stale (no longer in API)
        stale: usize,
    },
    /// Provider doesn't support model listing (e.g., custom URL)
    NotSupported,
    /// Sync failed with error
    Failed { error: String },
}

pub struct ModelSyncService {
    db: Arc<StorageBackend>,
    driver_registry: Arc<DriverRegistry>,
}

impl ModelSyncService {
    pub fn new(db: Arc<StorageBackend>, driver_registry: Arc<DriverRegistry>) -> Self {
        Self {
            db,
            driver_registry,
        }
    }

    /// Sync models for a single provider
    pub async fn sync_provider(&self, provider_id: Uuid) -> Result<SyncResult> {
        // Get the provider
        let provider_row = self
            .db
            .get_llm_provider(provider_id)
            .await?
            .context("Provider not found")?;

        // Skip sync for providers with custom base URLs
        if provider_row.base_url.is_some() {
            tracing::debug!(
                provider_id = %provider_id,
                "Skipping sync for provider with custom base URL"
            );
            return Ok(SyncResult::NotSupported);
        }

        // Parse provider type
        let provider_type: LlmProviderType = provider_row
            .provider_type
            .parse()
            .map_err(|e| anyhow::anyhow!("Invalid provider type: {}", e))?;

        // Get API key (try database first, then env fallback)
        let api_key = self.resolve_api_key(&provider_row).await?;
        let Some(api_key) = api_key else {
            return Ok(SyncResult::Failed {
                error: "No API key configured for provider".to_string(),
            });
        };

        // Create driver for the provider
        let driver_type = match provider_type {
            LlmProviderType::Openai => ProviderType::OpenAI,
            LlmProviderType::OpenaiCompletions => ProviderType::OpenAICompletions,
            LlmProviderType::Anthropic => ProviderType::Anthropic,
            LlmProviderType::Gemini => ProviderType::Gemini,
            LlmProviderType::OpenRouter => ProviderType::OpenRouter,
            LlmProviderType::LlmSim => {
                // LlmSim doesn't support model discovery
                return Ok(SyncResult::NotSupported);
            }
        };

        let config = ProviderConfig {
            provider_type: driver_type,
            api_key: Some(api_key),
            base_url: None, // We already checked for custom URLs above
            settings: None,
        };

        let driver = self
            .driver_registry
            .create_driver(&config)
            .map_err(|e| anyhow::anyhow!("Failed to create driver: {}", e))?;

        // Call list_models on the driver
        let discovered = match driver.list_models().await {
            Ok(Some(models)) => models,
            Ok(None) => {
                tracing::debug!(
                    provider_id = %provider_id,
                    "Provider driver doesn't support model listing"
                );
                return Ok(SyncResult::NotSupported);
            }
            Err(e) => {
                tracing::error!(
                    provider_id = %provider_id,
                    error = %e,
                    "Failed to list models from provider"
                );
                return Ok(SyncResult::Failed {
                    error: format!("Failed to list models: {}", e),
                });
            }
        };

        tracing::info!(
            provider_id = %provider_id,
            count = discovered.len(),
            "Discovered models from provider API"
        );

        // Sync discovered models to database (use provider's org_id)
        let sync_result = self.sync_models_to_db(&provider_row, &discovered).await?;

        // Update provider's last_synced_at
        self.db
            .update_provider_last_synced(provider_id, Utc::now())
            .await?;

        Ok(sync_result)
    }

    /// Sync all providers (called by background job)
    pub async fn sync_all(&self) -> Result<Vec<(ProviderId, SyncResult)>> {
        let providers = self.db.list_llm_providers().await?;
        let mut results = Vec::with_capacity(providers.len());

        for provider in providers {
            let result = self
                .sync_provider(provider.id.uuid())
                .await
                .unwrap_or_else(|e| SyncResult::Failed {
                    error: e.to_string(),
                });
            results.push((provider.id, result));
        }

        Ok(results)
    }

    /// Sync discovered models to database
    async fn sync_models_to_db(
        &self,
        provider: &LlmProviderRow,
        discovered: &[DiscoveredModel],
    ) -> Result<SyncResult> {
        let now = Utc::now();
        let mut created = 0;
        let mut updated = 0;

        // Parse provider type for profile lookup
        let provider_type: LlmProviderType = provider
            .provider_type
            .parse()
            .unwrap_or(LlmProviderType::Openai);

        // Get existing models for this provider
        let existing = self
            .db
            .list_llm_models_for_provider(provider.id.uuid())
            .await?;
        let existing_ids: std::collections::HashSet<_> =
            existing.iter().map(|m| m.model_id.as_str()).collect();

        // Track which model_ids we've seen in this sync
        let mut seen_model_ids: std::collections::HashSet<String> =
            std::collections::HashSet::new();

        for model in discovered {
            seen_model_ids.insert(model.model_id.clone());

            // Build provider metadata
            let metadata = serde_json::json!({
                "display_name": model.display_name,
                "created_at": model.created_at,
                "owned_by": model.owned_by,
            });

            if existing_ids.contains(model.model_id.as_str()) {
                // Update existing model's last_seen_at and metadata
                if let Some(existing_model) = existing.iter().find(|m| m.model_id == model.model_id)
                {
                    let update = UpdateLlmModel {
                        last_seen_at: Some(now),
                        provider_metadata: Some(metadata),
                        ..Default::default()
                    };
                    self.db
                        .update_llm_model(existing_model.id.uuid(), update)
                        .await?;
                    updated += 1;
                }
            } else {
                // Create new model in the same org as the provider
                // Try to get display name from model profile first, then API response, then model_id
                let display_name = get_model_profile(&provider_type, &model.model_id)
                    .map(|p| p.name)
                    .or_else(|| model.display_name.clone())
                    .unwrap_or_else(|| model.model_id.clone());

                let input = CreateLlmModelRow {
                    provider_id: provider.id,
                    model_id: model.model_id.clone(),
                    display_name,
                    capabilities: vec![],
                    is_default: false,
                    is_favorite: false,
                    source: "discovered".to_string(),
                    provider_metadata: Some(metadata),
                };

                self.db.create_llm_model(provider.org_id, input).await?;
                created += 1;
            }
        }

        // Mark models not seen in this sync as stale (set last_seen_at to NULL won't work,
        // instead we just leave them - they're stale if last_seen_at < provider.last_synced_at)
        // Models with source='discovered' that have last_seen_at < last_synced_at are considered stale
        let stale = existing
            .iter()
            .filter(|m| m.source == "discovered" && !seen_model_ids.contains(&m.model_id))
            .count();

        Ok(SyncResult::Success {
            created,
            updated,
            stale,
        })
    }

    /// Resolve API key for a provider (database or env fallback)
    async fn resolve_api_key(
        &self,
        provider_row: &crate::storage::models::LlmProviderRow,
    ) -> Result<Option<String>> {
        // Try database first (if encryption service available)
        if provider_row.api_key_encrypted.is_some() {
            // The database has an encrypted key, but we need the encryption service to decrypt it
            // For now, fall through to env vars
            // TODO: Integrate with EncryptionService when available in this context
        }

        // Fall back to environment variables
        let env_var = match provider_row.provider_type.as_str() {
            "openai" => "DEFAULT_OPENAI_API_KEY",
            "anthropic" => "DEFAULT_ANTHROPIC_API_KEY",
            "gemini" => "DEFAULT_GEMINI_API_KEY",
            "openrouter" => "DEFAULT_OPENROUTER_API_KEY",
            _ => return Ok(None),
        };

        Ok(std::env::var(env_var).ok().filter(|s| !s.is_empty()))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_sync_result_success_serialization() {
        let result = SyncResult::Success {
            created: 5,
            updated: 10,
            stale: 2,
        };

        let json = serde_json::to_string(&result).unwrap();
        assert!(json.contains(r#""status":"success""#));
        assert!(json.contains(r#""created":5"#));
        assert!(json.contains(r#""updated":10"#));
        assert!(json.contains(r#""stale":2"#));

        let deserialized: SyncResult = serde_json::from_str(&json).unwrap();
        match deserialized {
            SyncResult::Success {
                created,
                updated,
                stale,
            } => {
                assert_eq!(created, 5);
                assert_eq!(updated, 10);
                assert_eq!(stale, 2);
            }
            _ => panic!("Expected Success variant"),
        }
    }

    #[test]
    fn test_sync_result_not_supported_serialization() {
        let result = SyncResult::NotSupported;

        let json = serde_json::to_string(&result).unwrap();
        assert_eq!(json, r#"{"status":"not_supported"}"#);

        let deserialized: SyncResult = serde_json::from_str(&json).unwrap();
        assert!(matches!(deserialized, SyncResult::NotSupported));
    }

    #[test]
    fn test_sync_result_failed_serialization() {
        let result = SyncResult::Failed {
            error: "API error".to_string(),
        };

        let json = serde_json::to_string(&result).unwrap();
        assert!(json.contains(r#""status":"failed""#));
        assert!(json.contains(r#""error":"API error""#));

        let deserialized: SyncResult = serde_json::from_str(&json).unwrap();
        match deserialized {
            SyncResult::Failed { error } => {
                assert_eq!(error, "API error");
            }
            _ => panic!("Expected Failed variant"),
        }
    }

    /// Testable version with injectable env lookup (test-only).
    fn resolve_api_key_with_lookup<F>(provider_type: &str, env_lookup: F) -> Option<String>
    where
        F: Fn(&str) -> Option<String>,
    {
        let env_var = match provider_type {
            "openai" => "DEFAULT_OPENAI_API_KEY",
            "anthropic" => "DEFAULT_ANTHROPIC_API_KEY",
            "gemini" => "DEFAULT_GEMINI_API_KEY",
            "openrouter" => "DEFAULT_OPENROUTER_API_KEY",
            _ => return None,
        };

        env_lookup(env_var).filter(|s| !s.is_empty())
    }

    fn mock_env<'a>(vars: &'a [(&'a str, &'a str)]) -> impl Fn(&str) -> Option<String> + 'a {
        move |name| {
            vars.iter()
                .find(|(k, _)| *k == name)
                .map(|(_, v)| v.to_string())
        }
    }

    #[test]
    fn test_resolve_api_key_openai() {
        // Not set
        assert!(resolve_api_key_with_lookup("openai", mock_env(&[])).is_none());

        // Set
        let env = mock_env(&[("DEFAULT_OPENAI_API_KEY", "sk-test-key")]);
        assert_eq!(
            resolve_api_key_with_lookup("openai", &env),
            Some("sk-test-key".to_string())
        );
    }

    #[test]
    fn test_resolve_api_key_anthropic() {
        // Not set
        assert!(resolve_api_key_with_lookup("anthropic", mock_env(&[])).is_none());

        // Set
        let env = mock_env(&[("DEFAULT_ANTHROPIC_API_KEY", "sk-ant-test")]);
        assert_eq!(
            resolve_api_key_with_lookup("anthropic", &env),
            Some("sk-ant-test".to_string())
        );
    }

    #[test]
    fn test_resolve_api_key_unknown_provider() {
        let env = mock_env(&[
            ("DEFAULT_OPENAI_API_KEY", "sk-test"),
            ("DEFAULT_ANTHROPIC_API_KEY", "sk-ant-test"),
        ]);
        // Unknown providers return None (no default key)
        assert!(resolve_api_key_with_lookup("unknown", &env).is_none());
        // openai_completions also has no default - shares with OpenAI
        assert!(resolve_api_key_with_lookup("openai_completions", &env).is_none());
    }

    #[test]
    fn test_resolve_api_key_empty_value() {
        let env = mock_env(&[("DEFAULT_OPENAI_API_KEY", "")]);
        assert!(resolve_api_key_with_lookup("openai", &env).is_none());
    }
}
