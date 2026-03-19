// Model Sync Service
//
// Handles synchronization of models from provider APIs into the database.
// Supports both manual sync (via API endpoint) and background sync (periodic).
//
// Decision: Reuses the same API key resolution logic as LlmResolverService
// (decrypt from DB first, then env fallback). Enumerates providers across
// all orgs for background sync, not just DEFAULT_ORG_ID.

use crate::services::llm_resolver::resolve_provider_api_key;
use crate::storage::{
    EncryptionService, StorageBackend,
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
    encryption: Option<Arc<EncryptionService>>,
}

impl ModelSyncService {
    pub fn new(
        db: Arc<StorageBackend>,
        driver_registry: Arc<DriverRegistry>,
        encryption: Option<Arc<EncryptionService>>,
    ) -> Self {
        Self {
            db,
            driver_registry,
            encryption,
        }
    }

    /// Sync models for a single provider
    pub async fn sync_provider(&self, org_id: i64, provider_id: Uuid) -> Result<SyncResult> {
        // Get the provider
        let provider_row = self
            .db
            .get_llm_provider(org_id, provider_id)
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

        // Get API key (decrypt from DB first, then env fallback)
        let api_key = self.resolve_api_key(&provider_row)?;
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
            LlmProviderType::LlmSim => {
                // LlmSim doesn't support model discovery
                return Ok(SyncResult::NotSupported);
            }
        };

        let config = ProviderConfig {
            provider_type: driver_type,
            api_key: Some(api_key),
            base_url: None, // We already checked for custom URLs above
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
            .update_provider_last_synced(org_id, provider_id, Utc::now())
            .await?;

        Ok(sync_result)
    }

    /// Sync all providers across all orgs (called by background job)
    pub async fn sync_all(&self) -> Result<Vec<(ProviderId, SyncResult)>> {
        let orgs = self.db.list_organizations().await?;
        let mut results = Vec::new();

        for org in &orgs {
            let providers = self.db.list_llm_providers(org.org_id).await?;
            for provider in providers {
                let result = self
                    .sync_provider(provider.org_id, provider.id.uuid())
                    .await
                    .unwrap_or_else(|e| SyncResult::Failed {
                        error: e.to_string(),
                    });
                results.push((provider.id, result));
            }
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
            .list_llm_models_for_provider(provider.org_id, provider.id.uuid())
            .await?;
        let existing_ids: std::collections::HashSet<_> =
            existing.iter().map(|m| m.model_id.as_str()).collect();

        // Track which model_ids we've seen in this sync
        let mut seen_model_ids: std::collections::HashSet<String> =
            std::collections::HashSet::new();

        for model in discovered {
            seen_model_ids.insert(model.model_id.clone());

            // Build provider metadata (includes discovered_profile when available)
            let metadata = serde_json::json!({
                "display_name": model.display_name,
                "created_at": model.created_at,
                "owned_by": model.owned_by,
                "discovered_profile": model.discovered_profile,
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
                        .update_llm_model(provider.org_id, existing_model.id.uuid(), update)
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
                    installed: false,
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

    /// Resolve API key for a provider (delegates to shared helper).
    fn resolve_api_key(&self, provider_row: &LlmProviderRow) -> Result<Option<String>> {
        resolve_provider_api_key(&self.db, self.encryption.as_deref(), provider_row)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::storage::models::{CreateLlmProviderRow, CreateOrganizationRow};

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

    // --- resolve_api_key tests ---

    #[tokio::test]
    async fn test_resolve_api_key_with_encrypted_key() {
        use everruns_core::DEFAULT_ORG_ID;

        let db = Arc::new(StorageBackend::in_memory());
        let encryption = Arc::new(
            EncryptionService::new("kek-v1:8B3uCQ4Znx45hl5nB+PKVriRrj/KtEVM+wBZ2VGa9vY=", &[])
                .unwrap(),
        );
        let registry = Arc::new(DriverRegistry::new());
        let service = ModelSyncService::new(db.clone(), registry, Some(encryption.clone()));

        // Create provider with encrypted API key (DEFAULT_ORG pre-exists)
        let encrypted_key = encryption.encrypt_string("sk-secret-from-db").unwrap();

        let provider = db
            .create_llm_provider(
                DEFAULT_ORG_ID,
                CreateLlmProviderRow {
                    name: "OpenAI".to_string(),
                    provider_type: "openai".to_string(),
                    base_url: None,
                    api_key_encrypted: Some(encrypted_key),
                    settings: None,
                },
            )
            .await
            .unwrap();

        // resolve_api_key should decrypt the DB key
        let resolved = service.resolve_api_key(&provider).unwrap();
        assert_eq!(resolved, Some("sk-secret-from-db".to_string()));
    }

    #[tokio::test]
    async fn test_resolve_api_key_falls_back_to_env_without_encryption() {
        use everruns_core::DEFAULT_ORG_ID;

        let db = Arc::new(StorageBackend::in_memory());
        let registry = Arc::new(DriverRegistry::new());
        // No encryption service
        let service = ModelSyncService::new(db.clone(), registry, None);

        let provider = db
            .create_llm_provider(
                DEFAULT_ORG_ID,
                CreateLlmProviderRow {
                    name: "OpenAI".to_string(),
                    provider_type: "openai".to_string(),
                    base_url: None,
                    api_key_encrypted: Some(vec![1, 2, 3]), // encrypted but no service
                    settings: None,
                },
            )
            .await
            .unwrap();

        // Should fall back to env (which won't be set in test) -> None
        let resolved = service.resolve_api_key(&provider).unwrap();
        assert!(resolved.is_none());
    }

    #[tokio::test]
    async fn test_resolve_api_key_no_encrypted_key_falls_to_env() {
        use everruns_core::DEFAULT_ORG_ID;

        let db = Arc::new(StorageBackend::in_memory());
        let registry = Arc::new(DriverRegistry::new());
        let service = ModelSyncService::new(db.clone(), registry, None);

        let provider = db
            .create_llm_provider(
                DEFAULT_ORG_ID,
                CreateLlmProviderRow {
                    name: "Anthropic".to_string(),
                    provider_type: "anthropic".to_string(),
                    base_url: None,
                    api_key_encrypted: None,
                    settings: None,
                },
            )
            .await
            .unwrap();

        // No encrypted key, no env var -> None
        let resolved = service.resolve_api_key(&provider).unwrap();
        assert!(resolved.is_none());
    }

    // --- sync_all multi-org tests ---

    #[tokio::test]
    async fn test_sync_all_enumerates_providers_across_orgs() {
        use everruns_core::DEFAULT_ORG_ID;

        let db = Arc::new(StorageBackend::in_memory());
        let registry = Arc::new(DriverRegistry::new());
        let service = ModelSyncService::new(db.clone(), registry, None);

        // DEFAULT_ORG_ID (1) is pre-created by in-memory store. Create a second org.
        let org2 = db
            .create_organization_with_id(
                2,
                CreateOrganizationRow {
                    public_id: "org-2".to_string(),
                    name: "Org 2".to_string(),
                    created_by: None,
                },
            )
            .await
            .unwrap()
            .unwrap();

        // Create providers in each org
        let _p1 = db
            .create_llm_provider(
                DEFAULT_ORG_ID,
                CreateLlmProviderRow {
                    name: "OpenAI Org1".to_string(),
                    provider_type: "openai".to_string(),
                    base_url: None,
                    api_key_encrypted: None,
                    settings: None,
                },
            )
            .await
            .unwrap();

        let _p2 = db
            .create_llm_provider(
                org2.org_id,
                CreateLlmProviderRow {
                    name: "Anthropic Org2".to_string(),
                    provider_type: "anthropic".to_string(),
                    base_url: None,
                    api_key_encrypted: None,
                    settings: None,
                },
            )
            .await
            .unwrap();

        // sync_all should hit providers from both orgs
        let results = service.sync_all().await.unwrap();
        assert_eq!(
            results.len(),
            2,
            "Expected 2 providers (one per org), got {}",
            results.len()
        );

        // Both should fail (no API key available) but the point is they were enumerated
        for (pid, result) in &results {
            match result {
                SyncResult::Failed { error } => {
                    assert!(
                        error.contains("No API key"),
                        "Expected 'No API key' error for {pid}, got: {error}"
                    );
                }
                other => panic!("Expected Failed for {pid}, got: {other:?}"),
            }
        }
    }

    #[tokio::test]
    async fn test_sync_all_no_providers_returns_empty() {
        // In-memory store has DEFAULT_ORG pre-created but no providers
        let db = Arc::new(StorageBackend::in_memory());
        let registry = Arc::new(DriverRegistry::new());
        let service = ModelSyncService::new(db, registry, None);

        let results = service.sync_all().await.unwrap();
        assert!(results.is_empty());
    }

    #[tokio::test]
    async fn test_sync_all_with_encrypted_keys_across_orgs() {
        use everruns_core::DEFAULT_ORG_ID;

        let db = Arc::new(StorageBackend::in_memory());
        let encryption = Arc::new(
            EncryptionService::new("kek-v1:8B3uCQ4Znx45hl5nB+PKVriRrj/KtEVM+wBZ2VGa9vY=", &[])
                .unwrap(),
        );
        let registry = Arc::new(DriverRegistry::new());
        let service = ModelSyncService::new(db.clone(), registry, Some(encryption.clone()));

        // DEFAULT_ORG_ID pre-exists; create second org
        let org2 = db
            .create_organization_with_id(
                2,
                CreateOrganizationRow {
                    public_id: "org-2".to_string(),
                    name: "Org 2".to_string(),
                    created_by: None,
                },
            )
            .await
            .unwrap()
            .unwrap();

        let enc_key1 = encryption.encrypt_string("sk-org1-key").unwrap();
        let enc_key2 = encryption.encrypt_string("sk-org2-key").unwrap();

        let p1 = db
            .create_llm_provider(
                DEFAULT_ORG_ID,
                CreateLlmProviderRow {
                    name: "OpenAI Org1".to_string(),
                    provider_type: "openai".to_string(),
                    base_url: None,
                    api_key_encrypted: Some(enc_key1),
                    settings: None,
                },
            )
            .await
            .unwrap();

        let p2 = db
            .create_llm_provider(
                org2.org_id,
                CreateLlmProviderRow {
                    name: "Anthropic Org2".to_string(),
                    provider_type: "anthropic".to_string(),
                    base_url: None,
                    api_key_encrypted: Some(enc_key2),
                    settings: None,
                },
            )
            .await
            .unwrap();

        // Verify resolve_api_key decrypts for both providers
        let key1 = service.resolve_api_key(&p1).unwrap();
        assert_eq!(key1, Some("sk-org1-key".to_string()));

        let key2 = service.resolve_api_key(&p2).unwrap();
        assert_eq!(key2, Some("sk-org2-key".to_string()));

        // sync_all should enumerate both orgs (actual sync will fail since
        // we don't have real API endpoints, but the providers are enumerated)
        let results = service.sync_all().await.unwrap();
        assert_eq!(results.len(), 2);
    }

    // =========================================================================
    // Encrypted DB key model sync negative tests (EVE-57 / EVE-61)
    // =========================================================================

    #[tokio::test]
    async fn resolve_api_key_corrupted_encrypted_data_fails() {
        use everruns_core::DEFAULT_ORG_ID;
        let db = Arc::new(StorageBackend::in_memory());
        let encryption = Arc::new(
            EncryptionService::new("kek-v1:8B3uCQ4Znx45hl5nB+PKVriRrj/KtEVM+wBZ2VGa9vY=", &[])
                .unwrap(),
        );
        let registry = Arc::new(DriverRegistry::new());
        let service = ModelSyncService::new(db.clone(), registry, Some(encryption));
        let provider = db
            .create_llm_provider(
                DEFAULT_ORG_ID,
                CreateLlmProviderRow {
                    name: "Corrupt Provider".to_string(),
                    provider_type: "openai".to_string(),
                    base_url: None,
                    api_key_encrypted: Some(b"not-valid-encrypted-data".to_vec()),
                    settings: None,
                },
            )
            .await
            .unwrap();
        assert!(
            service.resolve_api_key(&provider).is_err(),
            "corrupted data must error"
        );
    }

    #[tokio::test]
    async fn resolve_api_key_wrong_key_id_fails() {
        use everruns_core::DEFAULT_ORG_ID;
        let db = Arc::new(StorageBackend::in_memory());
        let enc_v1 = Arc::new(
            EncryptionService::new("kek-v1:8B3uCQ4Znx45hl5nB+PKVriRrj/KtEVM+wBZ2VGa9vY=", &[])
                .unwrap(),
        );
        let encrypted = enc_v1.encrypt_string("sk-secret").unwrap();
        let enc_v2 = Arc::new(
            EncryptionService::new(
                &crate::storage::encryption::generate_encryption_key("kek-v2"),
                &[],
            )
            .unwrap(),
        );
        let registry = Arc::new(DriverRegistry::new());
        let service = ModelSyncService::new(db.clone(), registry, Some(enc_v2));
        let provider = db
            .create_llm_provider(
                DEFAULT_ORG_ID,
                CreateLlmProviderRow {
                    name: "Wrong Key Provider".to_string(),
                    provider_type: "openai".to_string(),
                    base_url: None,
                    api_key_encrypted: Some(encrypted),
                    settings: None,
                },
            )
            .await
            .unwrap();
        assert!(
            service.resolve_api_key(&provider).is_err(),
            "wrong key must error"
        );
    }

    #[tokio::test]
    async fn sync_all_provider_not_visible_cross_org() {
        use everruns_core::DEFAULT_ORG_ID;
        let db = Arc::new(StorageBackend::in_memory());
        let registry = Arc::new(DriverRegistry::new());
        let service = ModelSyncService::new(db.clone(), registry, None);
        let _p1 = db
            .create_llm_provider(
                DEFAULT_ORG_ID,
                CreateLlmProviderRow {
                    name: "OpenAI".to_string(),
                    provider_type: "openai".to_string(),
                    base_url: None,
                    api_key_encrypted: None,
                    settings: None,
                },
            )
            .await
            .unwrap();
        let _org2 = db
            .create_organization_with_id(
                2,
                CreateOrganizationRow {
                    public_id: "org-2".to_string(),
                    name: "Org 2".to_string(),
                    created_by: None,
                },
            )
            .await
            .unwrap();
        let results = service.sync_all().await.unwrap();
        assert_eq!(results.len(), 1, "org 2 has no providers");
    }
}
