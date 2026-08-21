// Model and provider resolver service.
//
// Model resolution returns credential-free identity. Provider configuration,
// including decrypted credentials, is resolved independently by exact public
// provider id for worker communication and non-chat services.
//
// Decision: In-process moka cache keyed on (org_id, model_id) with 1-hour TTL.
// Providers/models change rarely but resolution is called per LLM request.
// Cache is invalidated explicitly via invalidate_cache() on provider/model CRUD.
//
// API key resolution order (handled at service layer):
// 1. Decrypted key from database (if set)
// 2. None — fail closed; no environment variable fallback in the tenant path.
//
// The env-var helpers (get_default_api_key_from_env) remain available for
// explicit standalone/dev entrypoints (CLI, InMemoryProviderStore) but must
// NOT be called from any org-scoped execution path.

use crate::kernel_imports::{
    everruns_provider::driver_registry::DriverRegistry,
    everruns_provider::driver_registry::ServiceKind, everruns_provider::provider::DriverId,
    everruns_provider::typed_id::ProviderId,
};
use crate::storage::{EncryptionService, StorageBackend, models::ProviderRow};
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
    match provider_type.to_lowercase().as_str() {
        "openai" => env_lookup("DEFAULT_OPENAI_API_KEY").filter(|s| !s.is_empty()),
        "openrouter" => env_lookup("DEFAULT_OPENROUTER_API_KEY").filter(|s| !s.is_empty()),
        "azure_openai" => env_lookup("DEFAULT_AZURE_OPENAI_API_KEY").filter(|s| !s.is_empty()),
        "anthropic" => env_lookup("DEFAULT_ANTHROPIC_API_KEY").filter(|s| !s.is_empty()),
        "gemini" => env_lookup("DEFAULT_GEMINI_API_KEY").filter(|s| !s.is_empty()),
        "fireworks" => env_lookup("DEFAULT_FIREWORKS_API_KEY").filter(|s| !s.is_empty()),
        "meta" => env_lookup("DEFAULT_META_API_KEY").filter(|s| !s.is_empty()),
        "bedrock" => {
            // Construct JSON credentials from Bedrock-specific or generic AWS env vars.
            let access_key_id = env_lookup("AWS_BEDROCK_ACCESS_KEY_ID")
                .or_else(|| env_lookup("AWS_ACCESS_KEY_ID"))
                .filter(|s| !s.is_empty())?;
            let secret_access_key = env_lookup("AWS_BEDROCK_SECRET_ACCESS_KEY")
                .or_else(|| env_lookup("AWS_SECRET_ACCESS_KEY"))
                .filter(|s| !s.is_empty())?;
            let region = env_lookup("AWS_BEDROCK_REGION")
                .or_else(|| env_lookup("AWS_REGION"))
                .filter(|s| !s.is_empty())
                .unwrap_or_else(|| "us-east-1".to_string());
            let session_token = env_lookup("AWS_BEDROCK_SESSION_TOKEN")
                .or_else(|| env_lookup("AWS_SESSION_TOKEN"))
                .filter(|s| !s.is_empty());

            let mut cred = serde_json::json!({
                "access_key_id": access_key_id,
                "secret_access_key": secret_access_key,
                "region": region,
            });
            if let Some(token) = session_token {
                cred["session_token"] = serde_json::Value::String(token);
            }
            Some(cred.to_string())
        }
        _ => None,
    }
}

/// Resolve API key for a provider (fail-closed).
///
/// Shared logic used by both ProviderResolverService and ModelSyncService.
///
/// Resolution order:
/// 1. Decrypt from database if encryption is available and key is set
/// 2. None — never falls back to environment variables
///
/// Callers must treat None as "no provider configured" and surface an error.
/// This prevents tenant execution from silently spending platform-level env keys.
pub fn resolve_provider_api_key(
    db: &StorageBackend,
    encryption: Option<&EncryptionService>,
    provider: &ProviderRow,
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
                "Provider has encrypted API key but encryption service is not configured."
            );
        }
    }

    Ok(None)
}

/// Read a provider row's connection-level request options from its stored
/// settings.
///
/// A malformed or absent `request_options` blob yields the empty options rather
/// than failing resolution: a bad settings value must not take a provider
/// offline.
pub fn provider_request_options(
    settings: &serde_json::Value,
) -> everruns_provider::provider::ProviderRequestOptions {
    settings
        .get("request_options")
        .and_then(|value| serde_json::from_value(value.clone()).ok())
        .unwrap_or_default()
}

/// Credential-free resolved model identity.
#[derive(Debug, Clone)]
pub struct ResolvedModel {
    /// The model identifier (e.g., "gpt-4", "claude-3-opus")
    pub model_id: String,
    /// Provider type (e.g., "openai", "anthropic")
    pub provider_type: String,
    /// Public persisted provider identity; credentials resolve independently.
    pub provider_id: String,
}

/// Resolved provider credentials for tool-side API clients.
#[derive(Debug, Clone)]
pub struct ResolvedProviderCredentials {
    pub api_key: String,
    pub base_url: Option<String>,
}

/// A provider connection resolved for a specific non-chat [`ServiceKind`].
#[derive(Debug, Clone)]
pub struct ResolvedServiceProvider {
    /// Driver/provider type string of the selected provider (e.g. "openai").
    pub provider_type: String,
    /// Public id of the selected provider connection.
    pub provider_id: String,
    /// Decrypted credentials for the provider connection.
    pub credentials: ResolvedProviderCredentials,
    /// Connection-level request options stored on the provider row.
    pub request_options: everruns_provider::provider::ProviderRequestOptions,
}

/// Exact provider construction state for chat/runtime drivers.
///
/// Unlike service clients, local and simulated chat drivers may not require an
/// API key, so credential absence is represented without hiding the provider.
#[derive(Debug, Clone)]
pub(crate) struct ResolvedRuntimeProviderConfig {
    pub provider_type: String,
    pub api_key: Option<String>,
    pub base_url: Option<String>,
    /// Connection-level request options stored on the provider row.
    pub request_options: everruns_provider::provider::ProviderRequestOptions,
}

/// Cache key: (org_id, model_uuid). Default-model lookups use DEFAULT_MODEL_SENTINEL.
type CacheKey = (i64, Uuid);

pub struct ProviderResolverService {
    db: Arc<StorageBackend>,
    encryption: Option<Arc<EncryptionService>>,
    cache: Cache<CacheKey, Option<ResolvedModel>>,
    /// Driver registry powering service-bound resolution (`resolve_service`):
    /// it declares which drivers implement which [`ServiceKind`]. Empty by
    /// default; the server composition root wires in the platform registry.
    driver_registry: DriverRegistry,
}

impl ProviderResolverService {
    pub fn new(db: Arc<StorageBackend>, encryption: Option<Arc<EncryptionService>>) -> Self {
        let cache = Cache::builder()
            .max_capacity(CACHE_MAX_ENTRIES)
            .time_to_live(CACHE_TTL)
            .build();
        Self {
            db,
            encryption,
            cache,
            driver_registry: DriverRegistry::new(),
        }
    }

    /// Attach the driver registry that powers [`Self::resolve_service`].
    ///
    /// Without it, service-bound resolution fails closed (no driver declares
    /// any service), so the server composition root must call this.
    pub fn with_driver_registry(mut self, driver_registry: DriverRegistry) -> Self {
        self.driver_registry = driver_registry;
        self
    }

    /// Resolve a model by ID without provider credentials or endpoint details.
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

    /// Resolve the default model without provider credentials or endpoint details.
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

    /// Resolve default credentials for a provider type (fail-closed).
    ///
    /// Preference order:
    /// 1. Active providers matching the requested type, newest first
    ///
    /// Returns None when no provider with a configured key is found.
    /// Never falls back to environment variables — callers surface a
    /// "no provider configured" error on None.
    pub async fn resolve_provider_credentials(
        &self,
        org_id: i64,
        provider_type: &str,
    ) -> Result<Option<ResolvedProviderCredentials>> {
        let providers = self.db.list_providers(org_id).await?;
        let provider_type_lower = provider_type.to_lowercase();

        let matching: Vec<_> = providers
            .into_iter()
            .filter(|provider| {
                provider
                    .provider_type
                    .eq_ignore_ascii_case(&provider_type_lower)
            })
            .collect();

        for provider in matching
            .iter()
            .filter(|provider| provider.status.eq_ignore_ascii_case("active"))
        {
            if let Some(api_key) = self.resolve_api_key(provider)? {
                return Ok(Some(ResolvedProviderCredentials {
                    api_key,
                    base_url: provider.base_url.clone(),
                }));
            }
        }

        Ok(None)
    }

    /// Service-bound resolution: select a provider connection that serves the
    /// requested [`ServiceKind`], fail-closed (knowledge/foundations/providers.md).
    ///
    /// Selection order:
    /// 1. An explicit `binding` (a provider public id supplied by the consumer,
    ///    e.g. a voice connection's provider) wins — but only when that
    ///    provider is active and its driver declares the service.
    /// 2. An org default provider pinned for this service.
    /// 3. Otherwise the first active provider whose driver declares the service.
    ///
    /// Returns a structured "no provider configured for {service}" error when
    /// nothing matches. Like chat resolution, this never falls back to
    /// environment-only credentials in tenant paths (the fail-closed key
    /// contract in knowledge/foundations/llm-drivers.md): a provider row without a usable key
    /// is skipped, not satisfied from the host environment.
    pub async fn resolve_service(
        &self,
        org_id: i64,
        service: ServiceKind,
        binding: Option<&str>,
    ) -> Result<ResolvedServiceProvider> {
        let providers = self.db.list_providers(org_id).await?;

        // Tier 1: an explicit provider binding wins, but only if the provider is
        // active and its driver actually declares the requested service.
        if let Some(binding) = binding {
            let binding_id: ProviderId = binding
                .parse()
                .map_err(|_| anyhow::anyhow!("malformed provider binding: {binding}"))?;
            let provider = providers
                .iter()
                .find(|provider| provider.id == binding_id)
                .ok_or_else(|| anyhow::anyhow!("provider {binding} not found for org"))?;
            if !provider.status.eq_ignore_ascii_case("active") {
                return Err(anyhow::anyhow!("provider {binding} is not active"));
            }
            if !self.driver_supports(&provider.provider_type, service) {
                return Err(anyhow::anyhow!(
                    "provider {binding} does not provide the {service} service"
                ));
            }
            let api_key = self.resolve_api_key(provider)?.ok_or_else(|| {
                anyhow::anyhow!("no credentials configured for provider {binding}")
            })?;
            return Ok(ResolvedServiceProvider {
                provider_type: provider.provider_type.clone(),
                provider_id: provider.id.to_string(),
                credentials: ResolvedProviderCredentials {
                    api_key,
                    base_url: provider.base_url.clone(),
                },
                request_options: provider_request_options(&provider.settings),
            });
        }

        // Tier 2: an org-level default provider pinned for this service. When a
        // default is configured it is authoritative and fail-closed — a missing,
        // inactive, or service-incompatible default surfaces an error rather than
        // silently falling through to the active-provider scan
        // (knowledge/foundations/providers.md, EVE-569).
        if let Some(settings) = self.db.get_organization_settings(org_id).await?
            && let Some(default_id) = settings
                .default_provider_per_service
                .0
                .get(&service)
                .copied()
        {
            let provider = providers
                .iter()
                .find(|provider| provider.id == default_id)
                .ok_or_else(|| {
                    anyhow::anyhow!(
                        "org default provider {default_id} for the {service} service not found"
                    )
                })?;
            if !provider.status.eq_ignore_ascii_case("active") {
                return Err(anyhow::anyhow!(
                    "org default provider {default_id} for the {service} service is not active"
                ));
            }
            if !self.driver_supports(&provider.provider_type, service) {
                return Err(anyhow::anyhow!(
                    "org default provider {default_id} does not provide the {service} service"
                ));
            }
            let api_key = self.resolve_api_key(provider)?.ok_or_else(|| {
                anyhow::anyhow!("no credentials configured for org default provider {default_id}")
            })?;
            return Ok(ResolvedServiceProvider {
                provider_type: provider.provider_type.clone(),
                provider_id: provider.id.to_string(),
                credentials: ResolvedProviderCredentials {
                    api_key,
                    base_url: provider.base_url.clone(),
                },
                request_options: provider_request_options(&provider.settings),
            });
        }

        // Tier 3: the first active provider whose driver declares the service.
        for provider in providers
            .iter()
            .filter(|provider| provider.status.eq_ignore_ascii_case("active"))
            .filter(|provider| self.driver_supports(&provider.provider_type, service))
        {
            if let Some(api_key) = self.resolve_api_key(provider)? {
                return Ok(ResolvedServiceProvider {
                    provider_type: provider.provider_type.clone(),
                    provider_id: provider.id.to_string(),
                    credentials: ResolvedProviderCredentials {
                        api_key,
                        base_url: provider.base_url.clone(),
                    },
                    request_options: provider_request_options(&provider.settings),
                });
            }
        }

        Err(anyhow::anyhow!(
            "no provider configured for the {service} service"
        ))
    }

    /// Whether the driver behind a provider-type string declares `service`.
    ///
    /// The registry is the source of truth: a type string maps to a [`DriverId`]
    /// ([`DriverId::from_str`] is infallible — unknown strings become
    /// [`DriverId::External`]), and `supports` returns `true` only when that id
    /// is registered *and* its descriptor declares the service. An unregistered
    /// id (external or otherwise) therefore never matches.
    fn driver_supports(&self, provider_type: &str, service: ServiceKind) -> bool {
        let driver_id: DriverId = provider_type
            .parse()
            .expect("DriverId::from_str is infallible");
        self.driver_registry.supports(&driver_id, service)
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
        let model_row = self.db.get_model(org_id, model_id).await?;

        let model_row = match model_row {
            Some(row) => row,
            None => return Ok(None),
        };

        let provider_row = self
            .db
            .get_provider(org_id, model_row.provider_id.uuid())
            .await?;

        let provider_row = match provider_row {
            Some(row) => row,
            None => return Ok(None),
        };

        Ok(Some(ResolvedModel {
            model_id: model_row.model_id,
            provider_type: provider_row.provider_type.clone(),
            provider_id: provider_row.id.to_string(),
        }))
    }

    /// Uncached default model resolution.
    async fn resolve_default_model_uncached(&self, org_id: i64) -> Result<Option<ResolvedModel>> {
        let model_row = self.db.get_default_model(org_id).await?;

        let model_row = match model_row {
            Some(row) => row,
            None => return Ok(None),
        };

        let provider_row = self
            .db
            .get_provider(org_id, model_row.provider_id.uuid())
            .await?;

        let provider_row = match provider_row {
            Some(row) => row,
            None => return Ok(None),
        };

        Ok(Some(ResolvedModel {
            model_id: model_row.model_id,
            provider_type: provider_row.provider_type.clone(),
            provider_id: provider_row.id.to_string(),
        }))
    }

    /// Resolve one exact persisted provider for model execution.
    pub async fn resolve_runtime_provider(
        &self,
        org_id: i64,
        provider_id: &str,
    ) -> Result<Option<ResolvedServiceProvider>> {
        let id: ProviderId = provider_id
            .parse()
            .map_err(|_| anyhow::anyhow!("invalid provider id"))?;
        let Some(provider) = self.db.get_provider(org_id, id.uuid()).await? else {
            return Ok(None);
        };
        let Some(api_key) = self.resolve_api_key(&provider)? else {
            return Ok(None);
        };
        Ok(Some(ResolvedServiceProvider {
            provider_type: provider.provider_type,
            provider_id: provider.id.to_string(),
            credentials: ResolvedProviderCredentials {
                api_key,
                base_url: provider.base_url,
            },
            request_options: provider_request_options(&provider.settings),
        }))
    }

    /// Resolve one exact persisted provider for chat/runtime construction.
    ///
    /// Provider identity is returned even when no API key is configured. The
    /// driver registry remains responsible for rejecting missing credentials
    /// when its selected driver requires them.
    pub(crate) async fn resolve_runtime_provider_config(
        &self,
        org_id: i64,
        provider_id: &str,
    ) -> Result<Option<ResolvedRuntimeProviderConfig>> {
        let id: ProviderId = provider_id
            .parse()
            .map_err(|_| anyhow::anyhow!("invalid provider id"))?;
        let Some(provider) = self.db.get_provider(org_id, id.uuid()).await? else {
            return Ok(None);
        };
        let api_key = self.resolve_api_key(&provider)?;
        let request_options = provider_request_options(&provider.settings);
        Ok(Some(ResolvedRuntimeProviderConfig {
            provider_type: provider.provider_type,
            api_key,
            base_url: provider.base_url,
            request_options,
        }))
    }

    /// Resolve API key for a provider (delegates to shared helper).
    fn resolve_api_key(&self, provider: &ProviderRow) -> Result<Option<String>> {
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
            ("DEFAULT_OPENROUTER_API_KEY", "sk-or-test"),
            ("DEFAULT_ANTHROPIC_API_KEY", "sk-ant-test"),
        ]);
        // Unknown providers and providers without defaults return None
        assert_eq!(get_default_api_key_with_lookup("unknown", &env), None);
        assert_eq!(
            get_default_api_key_with_lookup("openai_completions", &env),
            None
        );
        assert_eq!(
            get_default_api_key_with_lookup("openrouter", &env),
            Some("sk-or-test".to_string())
        );
    }

    #[test]
    fn test_get_default_api_key_meta() {
        let explicit = mock_env(&[("DEFAULT_META_API_KEY", "meta-default")]);
        assert_eq!(
            get_default_api_key_with_lookup("meta", explicit),
            Some("meta-default".to_string())
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
    use crate::storage::models::{CreateModelRow, CreateProviderRow};

    /// Helper: create resolver with in-memory storage and seed a provider + model.
    /// Returns (resolver, model_uuid).
    async fn setup_resolver_with_model() -> (ProviderResolverService, Uuid) {
        let db = Arc::new(StorageBackend::in_memory());
        let resolver = ProviderResolverService::new(db.clone(), None);
        let org_id = DEFAULT_ORG_ID;

        let provider_row = db
            .create_provider(
                org_id,
                CreateProviderRow {
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
            .create_model(
                org_id,
                CreateModelRow {
                    provider_id: provider_row.id,
                    model_id: "gpt-4o".to_string(),
                    display_name: "GPT-4o".to_string(),
                    capabilities: vec!["chat".to_string()],
                    // Resolver paths require `enabled = TRUE`; these tests
                    // exercise successful resolution, so create as enabled.
                    enabled: true,
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
        let resolver = ProviderResolverService::new(db, None);

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
        let resolver = ProviderResolverService::new(db.clone(), None);
        let org_id = DEFAULT_ORG_ID;

        let provider_row = db
            .create_provider(
                org_id,
                CreateProviderRow {
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
            .create_model(
                org_id,
                CreateModelRow {
                    provider_id: provider_row.id,
                    model_id: "claude-3-opus".to_string(),
                    display_name: "Claude 3 Opus".to_string(),
                    capabilities: vec![],
                    // Resolver paths require `enabled = TRUE`.
                    enabled: true,
                    is_favorite: false,
                    source: "manual".to_string(),
                    provider_metadata: None,
                },
            )
            .await
            .unwrap();

        let model_b = db
            .create_model(
                org_id,
                CreateModelRow {
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
        let resolver = ProviderResolverService::new(db.clone(), None);
        let org_id = DEFAULT_ORG_ID;

        let provider_row = db
            .create_provider(
                org_id,
                CreateProviderRow {
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
            .create_model(
                org_id,
                CreateModelRow {
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
        let resolver = ProviderResolverService::new(db.clone(), None);
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
            EncryptionService::new("kek-v1:AAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAA=", &[])
                .unwrap(),
        )
    }

    #[tokio::test]
    async fn resolve_provider_api_key_decrypts_from_db() {
        let db = Arc::new(StorageBackend::in_memory());
        let encryption = test_encryption();

        let encrypted = encryption.encrypt_string("sk-from-db").unwrap();
        let provider = db
            .create_provider(
                DEFAULT_ORG_ID,
                CreateProviderRow {
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
            .create_provider(
                DEFAULT_ORG_ID,
                CreateProviderRow {
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
    async fn resolve_provider_api_key_no_db_key_returns_none() {
        let db = Arc::new(StorageBackend::in_memory());

        let provider = db
            .create_provider(
                DEFAULT_ORG_ID,
                CreateProviderRow {
                    name: "Anthropic".to_string(),
                    provider_type: "anthropic".to_string(),
                    base_url: None,
                    api_key_encrypted: None,
                    settings: None,
                },
            )
            .await
            .unwrap();

        // No encrypted key in DB -> None, regardless of env
        let result = resolve_provider_api_key(&db, None, &provider).unwrap();
        assert!(result.is_none());
    }

    /// EVE-511: resolver must not spend platform env keys for tenant execution.
    /// Sets DEFAULT_OPENAI_API_KEY in the process env so the test would fail
    /// against the old env-fallback implementation.
    #[tokio::test]
    async fn resolve_provider_api_key_env_key_set_does_not_leak() {
        let db = Arc::new(StorageBackend::in_memory());

        let provider = db
            .create_provider(
                DEFAULT_ORG_ID,
                CreateProviderRow {
                    name: "OpenAI".to_string(),
                    provider_type: "openai".to_string(),
                    base_url: None,
                    api_key_encrypted: None,
                    settings: None,
                },
            )
            .await
            .unwrap();

        // Safety: test-only, single-threaded assertion.
        // set_var/remove_var are unsafe in Rust 2024 because they are not
        // thread-safe; this test serialises the env mutation via the
        // variable going out of scope before any assertion.
        unsafe {
            std::env::set_var("DEFAULT_OPENAI_API_KEY", "sk-platform-key-must-not-leak");
        }
        let result = resolve_provider_api_key(&db, None, &provider).unwrap();
        unsafe {
            std::env::remove_var("DEFAULT_OPENAI_API_KEY");
        }

        assert!(
            result.is_none(),
            "resolve_provider_api_key must not fall back to DEFAULT_OPENAI_API_KEY"
        );
    }

    /// EVE-511: resolve_provider_credentials must also fail closed.
    /// Sets DEFAULT_OPENAI_API_KEY to verify it is never consulted.
    #[tokio::test]
    async fn resolve_provider_credentials_env_key_set_does_not_leak() {
        let db = Arc::new(StorageBackend::in_memory());
        let resolver = ProviderResolverService::new(db.clone(), None);

        // No provider configured for this org at all.
        // Safety: test-only env mutation, same rationale as above.
        unsafe {
            std::env::set_var("DEFAULT_OPENAI_API_KEY", "sk-platform-key-must-not-leak");
        }
        let result = resolver
            .resolve_provider_credentials(DEFAULT_ORG_ID, "openai")
            .await
            .unwrap();
        unsafe {
            std::env::remove_var("DEFAULT_OPENAI_API_KEY");
        }

        assert!(
            result.is_none(),
            "resolve_provider_credentials must not fall back to DEFAULT_OPENAI_API_KEY"
        );
    }

    #[tokio::test]
    async fn resolve_provider_credentials_ignores_disabled_provider() {
        let db = Arc::new(StorageBackend::in_memory());
        let encryption = test_encryption();
        let provider = seed_active_provider(&db, &encryption, "azure_openai").await;
        db.update_provider(
            DEFAULT_ORG_ID,
            provider.uuid(),
            crate::storage::models::UpdateProvider {
                status: Some("disabled".to_string()),
                ..Default::default()
            },
        )
        .await
        .unwrap();
        let resolver = ProviderResolverService::new(db, Some(encryption));

        let result = resolver
            .resolve_provider_credentials(DEFAULT_ORG_ID, "azure_openai")
            .await
            .unwrap();

        assert!(result.is_none(), "disabled providers must not be resolved");
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
        let resolver = ProviderResolverService::new(db.clone(), None);
        let org_id = DEFAULT_ORG_ID;

        let provider_row = db
            .create_provider(
                org_id,
                CreateProviderRow {
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
            .create_model(
                org_id,
                CreateModelRow {
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

    // --- resolve_service (service-bound resolution) tests ---

    /// Resolver wired with the real OSS driver registry: `openai` declares
    /// `Realtime`/`Chat`, `openrouter` is chat-only — exactly the asymmetry the
    /// service-kind selection must respect.
    fn service_resolver(
        db: Arc<StorageBackend>,
        encryption: Option<Arc<EncryptionService>>,
    ) -> ProviderResolverService {
        ProviderResolverService::new(db, encryption)
            .with_driver_registry(everruns_worker::create_driver_registry())
    }

    async fn seed_active_provider(
        db: &StorageBackend,
        encryption: &EncryptionService,
        provider_type: &str,
    ) -> everruns_provider::typed_id::ProviderId {
        use crate::storage::models::CreateProviderRow;
        let encrypted = encryption.encrypt_string("sk-test").unwrap();
        db.create_provider(
            DEFAULT_ORG_ID,
            CreateProviderRow {
                name: provider_type.to_string(),
                provider_type: provider_type.to_string(),
                base_url: None,
                api_key_encrypted: Some(encrypted),
                settings: None,
            },
        )
        .await
        .unwrap()
        .id
    }

    #[tokio::test]
    async fn exact_runtime_provider_resolution_is_org_scoped() {
        let db = Arc::new(StorageBackend::in_memory());
        let encryption = test_encryption();
        let provider = seed_active_provider(&db, &encryption, "openai").await;
        let resolver = ProviderResolverService::new(db, Some(encryption));

        let own = resolver
            .resolve_runtime_provider(DEFAULT_ORG_ID, &provider.to_string())
            .await
            .unwrap();
        assert!(own.is_some());

        let cross_org = resolver
            .resolve_runtime_provider(999, &provider.to_string())
            .await
            .unwrap();
        assert!(
            cross_org.is_none(),
            "provider must not cross org boundaries"
        );
    }

    #[tokio::test]
    async fn runtime_provider_config_preserves_credentialless_drivers() {
        use crate::storage::models::CreateProviderRow;

        let db = Arc::new(StorageBackend::in_memory());
        let provider = db
            .create_provider(
                DEFAULT_ORG_ID,
                CreateProviderRow {
                    name: "llmsim".to_string(),
                    provider_type: "llmsim".to_string(),
                    base_url: None,
                    api_key_encrypted: None,
                    settings: None,
                },
            )
            .await
            .unwrap();
        let resolver = ProviderResolverService::new(db, Some(test_encryption()));

        let resolved = resolver
            .resolve_runtime_provider_config(DEFAULT_ORG_ID, &provider.id.to_string())
            .await
            .unwrap()
            .expect("credentialless provider remains resolvable");
        assert_eq!(resolved.provider_type, "llmsim");
        assert!(resolved.api_key.is_none());

        assert!(
            resolver
                .resolve_runtime_provider_config(999, &provider.id.to_string())
                .await
                .unwrap()
                .is_none(),
            "provider config must remain org-scoped"
        );
    }

    #[tokio::test]
    async fn resolve_service_selects_active_provider_declaring_service() {
        let db = Arc::new(StorageBackend::in_memory());
        let encryption = test_encryption();
        seed_active_provider(&db, &encryption, "openai").await;
        let resolver = service_resolver(db, Some(encryption));

        let resolved = resolver
            .resolve_service(DEFAULT_ORG_ID, ServiceKind::Realtime, None)
            .await
            .expect("openai declares Realtime and has a key");
        assert_eq!(resolved.provider_type, "openai");
        assert_eq!(resolved.credentials.api_key, "sk-test");
    }

    #[tokio::test]
    async fn resolve_service_fails_closed_when_no_provider() {
        let db = Arc::new(StorageBackend::in_memory());
        let resolver = service_resolver(db, Some(test_encryption()));

        let err = resolver
            .resolve_service(DEFAULT_ORG_ID, ServiceKind::Realtime, None)
            .await
            .expect_err("no provider configured");
        assert!(
            err.to_string().contains("no provider configured"),
            "got: {err}"
        );
    }

    #[tokio::test]
    async fn resolve_service_skips_driver_without_service() {
        // OpenRouter is chat-only; it must not satisfy a Realtime request,
        // but it must still serve Chat.
        let db = Arc::new(StorageBackend::in_memory());
        let encryption = test_encryption();
        seed_active_provider(&db, &encryption, "openrouter").await;
        let resolver = service_resolver(db, Some(encryption));

        let err = resolver
            .resolve_service(DEFAULT_ORG_ID, ServiceKind::Realtime, None)
            .await
            .expect_err("openrouter does not declare Realtime");
        assert!(
            err.to_string().contains("no provider configured"),
            "got: {err}"
        );

        resolver
            .resolve_service(DEFAULT_ORG_ID, ServiceKind::Chat, None)
            .await
            .expect("openrouter declares Chat");
    }

    #[tokio::test]
    async fn resolve_service_binding_requires_service_support() {
        let db = Arc::new(StorageBackend::in_memory());
        let encryption = test_encryption();
        let openrouter = seed_active_provider(&db, &encryption, "openrouter").await;
        let resolver = service_resolver(db, Some(encryption));

        let err = resolver
            .resolve_service(
                DEFAULT_ORG_ID,
                ServiceKind::Realtime,
                Some(&openrouter.to_string()),
            )
            .await
            .expect_err("bound provider's driver lacks Realtime");
        assert!(err.to_string().contains("does not provide"), "got: {err}");
    }

    #[tokio::test]
    async fn resolve_service_binding_fails_closed_when_provider_disabled() {
        let db = Arc::new(StorageBackend::in_memory());
        let encryption = test_encryption();
        let provider = seed_active_provider(&db, &encryption, "openai").await;
        db.update_provider(
            DEFAULT_ORG_ID,
            provider.uuid(),
            crate::storage::models::UpdateProvider {
                status: Some("disabled".to_string()),
                ..Default::default()
            },
        )
        .await
        .unwrap();
        let resolver = service_resolver(db, Some(encryption));

        let err = resolver
            .resolve_service(
                DEFAULT_ORG_ID,
                ServiceKind::Realtime,
                Some(&provider.to_string()),
            )
            .await
            .expect_err("disabled explicit binding fails closed");
        assert!(err.to_string().contains("not active"), "got: {err}");
    }

    #[tokio::test]
    async fn resolve_service_binding_selects_explicit_provider() {
        let db = Arc::new(StorageBackend::in_memory());
        let encryption = test_encryption();
        // Two realtime-capable providers; the binding must pick the named one,
        // not just the first active match.
        let first = seed_active_provider(&db, &encryption, "openai").await;
        let second = seed_active_provider(&db, &encryption, "openai").await;
        let resolver = service_resolver(db, Some(encryption));

        let resolved = resolver
            .resolve_service(
                DEFAULT_ORG_ID,
                ServiceKind::Realtime,
                Some(&second.to_string()),
            )
            .await
            .expect("explicit binding resolves");
        assert_eq!(resolved.provider_id, second.to_string());
        assert_ne!(resolved.provider_id, first.to_string());
    }

    // --- Tier 2: org-level default provider per service (EVE-569) ---

    /// Pin `provider` as the org default for `service`.
    async fn set_service_default(
        db: &StorageBackend,
        service: ServiceKind,
        provider: everruns_provider::typed_id::ProviderId,
    ) {
        let mut defaults = crate::storage::models::ServiceProviderDefaults::new();
        defaults.insert(service, provider);
        db.patch_organization_settings(
            DEFAULT_ORG_ID,
            crate::storage::models::UpdateOrganizationSettings {
                default_provider_per_service: everruns_durable::UpdateField::Set(defaults),
                ..Default::default()
            },
        )
        .await
        .unwrap();
    }

    #[test]
    fn service_provider_defaults_json_round_trips() {
        // The Postgres path stores this map as JSONB; assert ServiceKind keys
        // serialize snake_case and ProviderId values round-trip as strings.
        let mut map = crate::storage::models::ServiceProviderDefaults::new();
        let pid = everruns_provider::typed_id::ProviderId::new();
        map.insert(ServiceKind::Realtime, pid);
        let value = serde_json::to_value(&map).unwrap();
        assert_eq!(value, serde_json::json!({ "realtime": pid.to_string() }));
        let back: crate::storage::models::ServiceProviderDefaults =
            serde_json::from_value(value).unwrap();
        assert_eq!(back.get(&ServiceKind::Realtime), Some(&pid));
    }

    #[tokio::test]
    async fn resolve_service_uses_org_default_before_active_fallback() {
        // Two realtime-capable providers; the org default (tier 2) must win over
        // the first-active scan (tier 3).
        let db = Arc::new(StorageBackend::in_memory());
        let encryption = test_encryption();
        let _first = seed_active_provider(&db, &encryption, "openai").await;
        let second = seed_active_provider(&db, &encryption, "openai").await;
        set_service_default(&db, ServiceKind::Realtime, second).await;
        let resolver = service_resolver(db, Some(encryption));

        let resolved = resolver
            .resolve_service(DEFAULT_ORG_ID, ServiceKind::Realtime, None)
            .await
            .expect("org default resolves");
        assert_eq!(resolved.provider_id, second.to_string());
    }

    #[tokio::test]
    async fn resolve_service_binding_overrides_org_default() {
        // Precedence: explicit binding (tier 1) wins over the org default (tier 2).
        let db = Arc::new(StorageBackend::in_memory());
        let encryption = test_encryption();
        let bound = seed_active_provider(&db, &encryption, "openai").await;
        let default = seed_active_provider(&db, &encryption, "openai").await;
        set_service_default(&db, ServiceKind::Realtime, default).await;
        let resolver = service_resolver(db, Some(encryption));

        let resolved = resolver
            .resolve_service(
                DEFAULT_ORG_ID,
                ServiceKind::Realtime,
                Some(&bound.to_string()),
            )
            .await
            .expect("binding resolves");
        assert_eq!(resolved.provider_id, bound.to_string());
        assert_ne!(resolved.provider_id, default.to_string());
    }

    #[tokio::test]
    async fn resolve_service_org_default_fails_closed_when_missing() {
        // A default that points at a non-existent provider must error, not
        // silently fall through to an otherwise-usable active provider.
        let db = Arc::new(StorageBackend::in_memory());
        let encryption = test_encryption();
        seed_active_provider(&db, &encryption, "openai").await;
        set_service_default(
            &db,
            ServiceKind::Realtime,
            everruns_provider::typed_id::ProviderId::new(),
        )
        .await;
        let resolver = service_resolver(db, Some(encryption));

        let err = resolver
            .resolve_service(DEFAULT_ORG_ID, ServiceKind::Realtime, None)
            .await
            .expect_err("missing org default fails closed");
        assert!(err.to_string().contains("not found"), "got: {err}");
    }

    #[tokio::test]
    async fn resolve_service_org_default_fails_closed_when_inactive() {
        let db = Arc::new(StorageBackend::in_memory());
        let encryption = test_encryption();
        let provider = seed_active_provider(&db, &encryption, "openai").await;
        set_service_default(&db, ServiceKind::Realtime, provider).await;
        db.update_provider(
            DEFAULT_ORG_ID,
            provider.uuid(),
            crate::storage::models::UpdateProvider {
                status: Some("inactive".to_string()),
                ..Default::default()
            },
        )
        .await
        .unwrap();
        let resolver = service_resolver(db, Some(encryption));

        let err = resolver
            .resolve_service(DEFAULT_ORG_ID, ServiceKind::Realtime, None)
            .await
            .expect_err("inactive org default fails closed");
        assert!(err.to_string().contains("not active"), "got: {err}");
    }

    #[tokio::test]
    async fn resolve_service_org_default_fails_closed_when_service_unsupported() {
        // openrouter is chat-only; pinning it as the Realtime default is invalid.
        let db = Arc::new(StorageBackend::in_memory());
        let encryption = test_encryption();
        let provider = seed_active_provider(&db, &encryption, "openrouter").await;
        set_service_default(&db, ServiceKind::Realtime, provider).await;
        let resolver = service_resolver(db, Some(encryption));

        let err = resolver
            .resolve_service(DEFAULT_ORG_ID, ServiceKind::Realtime, None)
            .await
            .expect_err("incompatible org default fails closed");
        assert!(err.to_string().contains("does not provide"), "got: {err}");
    }
}
