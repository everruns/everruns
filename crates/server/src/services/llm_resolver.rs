// LLM Resolver service for resolving models with decrypted provider credentials
//
// This service handles the resolution of LLM models with their provider credentials,
// including API key decryption. Used by gRPC service for worker communication.
//
// Decision: In-process moka cache keyed on (org_id, model_id) with 1-hour TTL.
// Providers/models change rarely but resolution is called per LLM request.
// Cache is invalidated explicitly via invalidate_cache() on provider/model CRUD.
//
// API key resolution order (handled at service layer):
// 1. Decrypted key from database (if set)
// 2. Environment variable fallback (for development convenience):
//    - openai: DEFAULT_OPENAI_API_KEY
//    - anthropic: DEFAULT_ANTHROPIC_API_KEY

use crate::storage::{EncryptionService, StorageBackend, models::LlmProviderRow};
use anyhow::Result;
use moka::future::Cache;
use std::sync::Arc;
use std::time::Duration;
use uuid::Uuid;

/// Cache TTL for resolved models (1 hour).
const CACHE_TTL: Duration = Duration::from_secs(3600);

/// Max cache entries. Each org+model combo is one entry.
const CACHE_MAX_ENTRIES: u64 = 1_000;

/// Sentinel UUID for default-model cache key (all zeros is unused by uuidv7).
const DEFAULT_MODEL_SENTINEL: Uuid = Uuid::nil();

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
        "gemini" => "DEFAULT_GEMINI_API_KEY",
        _ => return None,
    };

    env_lookup(env_var).filter(|s| !s.is_empty())
}

/// Resolve API key for a provider.
///
/// Shared logic used by both LlmResolverService and ModelSyncService.
///
/// Resolution order:
/// 1. Decrypt from database if encryption is available and key is set
/// 2. Fall back to environment variable (DEFAULT_OPENAI_API_KEY, DEFAULT_ANTHROPIC_API_KEY, etc.)
///
/// If provider has an encrypted key but encryption service is not available,
/// logs a warning and falls back to environment variable.
pub fn resolve_provider_api_key(
    db: &StorageBackend,
    encryption: Option<&EncryptionService>,
    provider: &LlmProviderRow,
) -> Result<Option<String>> {
    if provider.api_key_encrypted.is_some() {
        if let Some(encryption) = encryption {
            let provider_with_key = db.get_provider_with_api_key(provider, encryption)?;
            if provider_with_key.api_key.is_some() {
                return Ok(provider_with_key.api_key);
            }
        } else {
            tracing::warn!(
                provider_id = %provider.id,
                provider_type = %provider.provider_type,
                "Provider has encrypted API key but encryption service is not configured. \
                 Falling back to environment variable."
            );
        }
    }

    Ok(get_default_api_key_from_env(&provider.provider_type))
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

/// Cache key: (org_id, model_uuid). Default-model lookups use DEFAULT_MODEL_SENTINEL.
type CacheKey = (i64, Uuid);

pub struct LlmResolverService {
    db: Arc<StorageBackend>,
    encryption: Option<Arc<EncryptionService>>,
    cache: Cache<CacheKey, Option<ResolvedModel>>,
}

impl LlmResolverService {
    pub fn new(db: Arc<StorageBackend>, encryption: Option<Arc<EncryptionService>>) -> Self {
        let cache = Cache::builder()
            .max_capacity(CACHE_MAX_ENTRIES)
            .time_to_live(CACHE_TTL)
            .build();
        Self {
            db,
            encryption,
            cache,
        }
    }

    /// Resolve a model by ID with decrypted provider credentials.
    /// Results are cached per (org_id, model_id) with 1-hour TTL.
    pub async fn resolve_model(
        &self,
        org_id: i64,
        model_id: Uuid,
    ) -> Result<Option<ResolvedModel>> {
        let key = (org_id, model_id);

        if let Some(cached) = self.cache.get(&key).await {
            return Ok(cached);
        }

        let result = self.resolve_model_uncached(org_id, model_id).await?;
        self.cache.insert(key, result.clone()).await;
        Ok(result)
    }

    /// Resolve the default model with decrypted provider credentials.
    /// Cached under sentinel key (org_id, nil UUID).
    pub async fn resolve_default_model(&self, org_id: i64) -> Result<Option<ResolvedModel>> {
        let key = (org_id, DEFAULT_MODEL_SENTINEL);

        if let Some(cached) = self.cache.get(&key).await {
            return Ok(cached);
        }

        let result = self.resolve_default_model_uncached(org_id).await?;
        self.cache.insert(key, result.clone()).await;
        Ok(result)
    }

    /// Invalidate all cached resolutions for an org.
    /// Call on provider/model create, update, or delete.
    pub async fn invalidate_cache(&self, _org_id: i64) {
        // moka doesn't support prefix invalidation; full invalidation is fine
        // given the small cache size and 1-hour TTL.
        self.cache.invalidate_all();
        tracing::debug!("LLM resolver cache invalidated");
    }

    /// Uncached model resolution by UUID.
    async fn resolve_model_uncached(
        &self,
        org_id: i64,
        model_id: Uuid,
    ) -> Result<Option<ResolvedModel>> {
        let model_row = self.db.get_llm_model(org_id, model_id).await?;

        let model_row = match model_row {
            Some(row) => row,
            None => return Ok(None),
        };

        let provider_row = self
            .db
            .get_llm_provider(org_id, model_row.provider_id.uuid())
            .await?;

        let provider_row = match provider_row {
            Some(row) => row,
            None => return Ok(None),
        };

        let api_key = self.resolve_api_key(&provider_row)?;

        Ok(Some(ResolvedModel {
            model_id: model_row.model_id,
            provider_type: provider_row.provider_type.clone(),
            api_key,
            base_url: provider_row.base_url.clone(),
        }))
    }

    /// Uncached default model resolution.
    async fn resolve_default_model_uncached(&self, org_id: i64) -> Result<Option<ResolvedModel>> {
        let model_row = self.db.get_default_llm_model(org_id).await?;

        let model_row = match model_row {
            Some(row) => row,
            None => return Ok(None),
        };

        let provider_row = self
            .db
            .get_llm_provider(org_id, model_row.provider_id.uuid())
            .await?;

        let provider_row = match provider_row {
            Some(row) => row,
            None => return Ok(None),
        };

        let api_key = self.resolve_api_key(&provider_row)?;

        Ok(Some(ResolvedModel {
            model_id: model_row.model_id,
            provider_type: provider_row.provider_type.clone(),
            api_key,
            base_url: provider_row.base_url.clone(),
        }))
    }

    /// Resolve API key for a provider (delegates to shared helper).
    fn resolve_api_key(&self, provider: &LlmProviderRow) -> Result<Option<String>> {
        resolve_provider_api_key(&self.db, self.encryption.as_deref(), provider)
    }

    /// Check if encryption service is available
    pub fn has_encryption(&self) -> bool {
        self.encryption.is_some()
    }

    /// Return current cache entry count (for testing/metrics).
    pub fn cache_entry_count(&self) -> u64 {
        self.cache.entry_count()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use everruns_core::DEFAULT_ORG_ID;

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

    #[test]
    fn test_default_model_sentinel_is_nil() {
        assert!(DEFAULT_MODEL_SENTINEL.is_nil());
    }

    #[test]
    fn test_cache_key_different_org_ids() {
        let key_a: CacheKey = (1, Uuid::new_v4());
        let key_b: CacheKey = (2, key_a.1);
        assert_ne!(key_a, key_b);
    }

    #[test]
    fn test_cache_key_different_model_ids() {
        let key_a: CacheKey = (1, Uuid::new_v4());
        let key_b: CacheKey = (1, Uuid::new_v4());
        assert_ne!(key_a, key_b);
    }

    // --- Integration tests with in-memory storage ---

    use crate::storage::StorageBackend;
    use crate::storage::models::{CreateLlmModelRow, CreateLlmProviderRow};

    /// Helper: create resolver with in-memory storage and seed a provider + model.
    /// Returns (resolver, model_uuid).
    async fn setup_resolver_with_model() -> (LlmResolverService, Uuid) {
        let db = Arc::new(StorageBackend::in_memory());
        let resolver = LlmResolverService::new(db.clone(), None);
        let org_id = DEFAULT_ORG_ID;

        let provider_row = db
            .create_llm_provider(
                org_id,
                CreateLlmProviderRow {
                    name: "Test OpenAI".to_string(),
                    provider_type: "openai".to_string(),
                    base_url: None,
                    api_key_encrypted: None,
                    settings: None,
                },
            )
            .await
            .unwrap();

        let model_row = db
            .create_llm_model(
                org_id,
                CreateLlmModelRow {
                    provider_id: provider_row.id,
                    model_id: "gpt-4o".to_string(),
                    display_name: "GPT-4o".to_string(),
                    capabilities: vec!["chat".to_string()],
                    enabled: false,
                    is_favorite: false,
                    source: "manual".to_string(),
                    provider_metadata: None,
                },
            )
            .await
            .unwrap();

        (resolver, model_row.id.uuid())
    }

    #[tokio::test]
    async fn test_resolve_model_cache_miss_then_hit() {
        let (resolver, model_id) = setup_resolver_with_model().await;

        // Cache starts empty
        assert_eq!(resolver.cache_entry_count(), 0);

        // First call: cache miss -> populates cache
        let result = resolver
            .resolve_model(DEFAULT_ORG_ID, model_id)
            .await
            .unwrap();
        assert!(result.is_some());
        let resolved = result.unwrap();
        assert_eq!(resolved.model_id, "gpt-4o");
        assert_eq!(resolved.provider_type, "openai");

        // Run pending moka tasks so entry_count updates
        resolver.cache.run_pending_tasks().await;
        assert_eq!(resolver.cache_entry_count(), 1);

        // Second call: cache hit (same result)
        let result2 = resolver
            .resolve_model(DEFAULT_ORG_ID, model_id)
            .await
            .unwrap();
        assert!(result2.is_some());
        assert_eq!(result2.unwrap().model_id, "gpt-4o");

        // Still one entry
        assert_eq!(resolver.cache_entry_count(), 1);
    }

    #[tokio::test]
    async fn test_resolve_model_not_found_is_cached() {
        let db = Arc::new(StorageBackend::in_memory());
        let resolver = LlmResolverService::new(db, None);

        let missing_id = Uuid::new_v4();

        // First call: miss, returns None, caches it
        let result = resolver
            .resolve_model(DEFAULT_ORG_ID, missing_id)
            .await
            .unwrap();
        assert!(result.is_none());

        resolver.cache.run_pending_tasks().await;
        assert_eq!(resolver.cache_entry_count(), 1);

        // Second call: cache hit (still None)
        let result2 = resolver
            .resolve_model(DEFAULT_ORG_ID, missing_id)
            .await
            .unwrap();
        assert!(result2.is_none());
    }

    #[tokio::test]
    async fn test_invalidate_cache_clears_entries() {
        let (resolver, model_id) = setup_resolver_with_model().await;

        // Populate cache
        resolver
            .resolve_model(DEFAULT_ORG_ID, model_id)
            .await
            .unwrap();
        resolver.cache.run_pending_tasks().await;
        assert_eq!(resolver.cache_entry_count(), 1);

        // Invalidate
        resolver.invalidate_cache(DEFAULT_ORG_ID).await;
        resolver.cache.run_pending_tasks().await;
        assert_eq!(resolver.cache_entry_count(), 0);
    }

    #[tokio::test]
    async fn test_different_models_cached_independently() {
        let db = Arc::new(StorageBackend::in_memory());
        let resolver = LlmResolverService::new(db.clone(), None);
        let org_id = DEFAULT_ORG_ID;

        let provider_row = db
            .create_llm_provider(
                org_id,
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

        let model_a = db
            .create_llm_model(
                org_id,
                CreateLlmModelRow {
                    provider_id: provider_row.id,
                    model_id: "claude-3-opus".to_string(),
                    display_name: "Claude 3 Opus".to_string(),
                    capabilities: vec![],
                    enabled: false,
                    is_favorite: false,
                    source: "manual".to_string(),
                    provider_metadata: None,
                },
            )
            .await
            .unwrap();

        let model_b = db
            .create_llm_model(
                org_id,
                CreateLlmModelRow {
                    provider_id: provider_row.id,
                    model_id: "claude-3-sonnet".to_string(),
                    display_name: "Claude 3 Sonnet".to_string(),
                    capabilities: vec![],
                    enabled: true,
                    is_favorite: false,
                    source: "manual".to_string(),
                    provider_metadata: None,
                },
            )
            .await
            .unwrap();

        // Resolve both models
        let ra = resolver
            .resolve_model(DEFAULT_ORG_ID, model_a.id.uuid())
            .await
            .unwrap();
        let rb = resolver
            .resolve_model(DEFAULT_ORG_ID, model_b.id.uuid())
            .await
            .unwrap();

        assert_eq!(ra.unwrap().model_id, "claude-3-opus");
        assert_eq!(rb.unwrap().model_id, "claude-3-sonnet");

        resolver.cache.run_pending_tasks().await;
        assert_eq!(resolver.cache_entry_count(), 2);
    }

    #[tokio::test]
    async fn test_resolve_default_model_cached() {
        let db = Arc::new(StorageBackend::in_memory());
        let resolver = LlmResolverService::new(db.clone(), None);
        let org_id = DEFAULT_ORG_ID;

        let provider_row = db
            .create_llm_provider(
                org_id,
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

        let model = db
            .create_llm_model(
                org_id,
                CreateLlmModelRow {
                    provider_id: provider_row.id,
                    model_id: "gpt-4o".to_string(),
                    display_name: "GPT-4o".to_string(),
                    capabilities: vec![],
                    enabled: true,
                    is_favorite: false,
                    source: "manual".to_string(),
                    provider_metadata: None,
                },
            )
            .await
            .unwrap();

        // Set org default model
        db.upsert_organization_settings(org_id, Some(model.id.uuid()))
            .await
            .unwrap();

        // First call: populates cache
        let result = resolver
            .resolve_default_model(DEFAULT_ORG_ID)
            .await
            .unwrap();
        assert!(result.is_some());
        assert_eq!(result.unwrap().model_id, "gpt-4o");

        resolver.cache.run_pending_tasks().await;
        assert_eq!(resolver.cache_entry_count(), 1);

        // Second call: cache hit
        let result2 = resolver
            .resolve_default_model(DEFAULT_ORG_ID)
            .await
            .unwrap();
        assert!(result2.is_some());
        assert_eq!(result2.unwrap().model_id, "gpt-4o");
    }

    #[tokio::test]
    async fn test_invalidation_forces_fresh_resolution() {
        let db = Arc::new(StorageBackend::in_memory());
        let resolver = LlmResolverService::new(db.clone(), None);
        let org_id = DEFAULT_ORG_ID;

        // Resolve a missing model -> cached as None
        let missing_id = Uuid::new_v4();
        let result = resolver
            .resolve_model(DEFAULT_ORG_ID, missing_id)
            .await
            .unwrap();
        assert!(result.is_none());

        resolver.cache.run_pending_tasks().await;
        assert_eq!(resolver.cache_entry_count(), 1);

        // Invalidate cache
        resolver.invalidate_cache(org_id).await;
        resolver.cache.run_pending_tasks().await;
        assert_eq!(resolver.cache_entry_count(), 0);

        // Next resolve goes to DB again (still None since model doesn't exist)
        let result2 = resolver
            .resolve_model(DEFAULT_ORG_ID, missing_id)
            .await
            .unwrap();
        assert!(result2.is_none());

        // But entry is re-cached
        resolver.cache.run_pending_tasks().await;
        assert_eq!(resolver.cache_entry_count(), 1);
    }

    // --- resolve_provider_api_key shared function tests ---

    use crate::storage::EncryptionService;

    fn test_encryption() -> Arc<EncryptionService> {
        Arc::new(
            EncryptionService::new("kek-v1:8B3uCQ4Znx45hl5nB+PKVriRrj/KtEVM+wBZ2VGa9vY=", &[])
                .unwrap(),
        )
    }

    #[tokio::test]
    async fn resolve_provider_api_key_decrypts_from_db() {
        let db = Arc::new(StorageBackend::in_memory());
        let encryption = test_encryption();

        let encrypted = encryption.encrypt_string("sk-from-db").unwrap();
        let provider = db
            .create_llm_provider(
                DEFAULT_ORG_ID,
                CreateLlmProviderRow {
                    name: "OpenAI".to_string(),
                    provider_type: "openai".to_string(),
                    base_url: None,
                    api_key_encrypted: Some(encrypted),
                    settings: None,
                },
            )
            .await
            .unwrap();

        let result = resolve_provider_api_key(&db, Some(&*encryption), &provider).unwrap();
        assert_eq!(result, Some("sk-from-db".to_string()));
    }

    #[tokio::test]
    async fn resolve_provider_api_key_falls_back_without_encryption() {
        let db = Arc::new(StorageBackend::in_memory());

        let provider = db
            .create_llm_provider(
                DEFAULT_ORG_ID,
                CreateLlmProviderRow {
                    name: "OpenAI".to_string(),
                    provider_type: "openai".to_string(),
                    base_url: None,
                    api_key_encrypted: Some(vec![1, 2, 3]),
                    settings: None,
                },
            )
            .await
            .unwrap();

        // No encryption service, no env var -> None
        let result = resolve_provider_api_key(&db, None, &provider).unwrap();
        assert!(result.is_none());
    }

    #[tokio::test]
    async fn resolve_provider_api_key_no_key_falls_to_env() {
        let db = Arc::new(StorageBackend::in_memory());

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
        let result = resolve_provider_api_key(&db, None, &provider).unwrap();
        assert!(result.is_none());
    }

    // =========================================================================
    // Cross-org isolation regression tests (EVE-59)
    // =========================================================================

    /// Regression: resolve_model must scope lookups to the given org_id.
    /// A model created in org 1 must not be visible when resolved with org 999.
    #[tokio::test]
    async fn resolve_model_scoped_to_org() {
        let (resolver, model_id) = setup_resolver_with_model().await;

        // Model belongs to DEFAULT_ORG_ID — should resolve
        let result = resolver
            .resolve_model(DEFAULT_ORG_ID, model_id)
            .await
            .unwrap();
        assert!(result.is_some(), "model should resolve in its own org");

        // Same model UUID with a different org — should NOT resolve
        let result = resolver.resolve_model(999, model_id).await.unwrap();
        assert!(result.is_none(), "model must not resolve in another org");
    }

    /// Regression: resolve_default_model must scope to the given org_id.
    #[tokio::test]
    async fn resolve_default_model_scoped_to_org() {
        let db = Arc::new(StorageBackend::in_memory());
        let resolver = LlmResolverService::new(db.clone(), None);
        let org_id = DEFAULT_ORG_ID;

        let provider_row = db
            .create_llm_provider(
                org_id,
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

        let model = db
            .create_llm_model(
                org_id,
                CreateLlmModelRow {
                    provider_id: provider_row.id,
                    model_id: "gpt-4o".to_string(),
                    display_name: "GPT-4o".to_string(),
                    capabilities: vec![],
                    enabled: true,
                    is_favorite: false,
                    source: "manual".to_string(),
                    provider_metadata: None,
                },
            )
            .await
            .unwrap();

        // Set org default model
        db.upsert_organization_settings(org_id, Some(model.id.uuid()))
            .await
            .unwrap();

        // Default model belongs to DEFAULT_ORG_ID — should resolve
        let result = resolver
            .resolve_default_model(DEFAULT_ORG_ID)
            .await
            .unwrap();
        assert!(
            result.is_some(),
            "default model should resolve in its own org"
        );

        // Different org — should NOT resolve
        let result = resolver.resolve_default_model(999).await.unwrap();
        assert!(
            result.is_none(),
            "default model must not resolve in another org"
        );
    }
}
