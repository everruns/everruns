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
    contracts::driver_registry::DriverRegistry, contracts::driver_registry::ServiceKind,
    contracts::provider::DriverId, contracts::typed_id::ProviderId,
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

mod environment;
pub use environment::get_default_api_key_from_env;
#[cfg(test)]
use environment::get_default_api_key_with_lookup;

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
    // Personal refresh-token documents must never escape through org-wide key resolution.
    if provider.provider_type == "chatgpt" {
        return Ok(None);
    }
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
) -> everruns_contracts::provider::ProviderRequestOptions {
    settings
        .get("request_options")
        .and_then(|value| serde_json::from_value(value.clone()).ok())
        .unwrap_or_default()
}

/// Credential-free resolved model identity.
#[derive(Debug, Clone)]
pub struct ResolvedModel {
    /// The model identifier (e.g., "gpt-5.6-sol", "claude-opus-5")
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
    pub request_options: everruns_contracts::provider::ProviderRequestOptions,
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
    pub request_options: everruns_contracts::provider::ProviderRequestOptions,
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

    /// Resolve tenant decisions without reading deployment utility credentials.
    pub async fn resolve_decision_model(
        &self,
        org_id: i64,
        model_id: Option<&str>,
        session_id: Uuid,
    ) -> Result<Option<everruns_core::connection_services::DecisionModelBinding>> {
        let session = self
            .db
            .get_session(org_id, session_id.into())
            .await?
            .ok_or_else(|| anyhow::anyhow!("Session unavailable"))?;
        let _ = session;
        let id = match model_id {
            Some(id) => id.parse::<everruns_contracts::typed_id::ModelId>()?.uuid(),
            None => match self.db.get_decision_default(org_id).await? {
                Some(id) => id,
                None => return Ok(None),
            },
        };
        let row = self
            .db
            .get_model(org_id, id)
            .await?
            .ok_or_else(|| anyhow::anyhow!("Decision model unavailable"))?;
        if !row.enabled
            || row
                .provider_metadata
                .as_ref()
                .is_some_and(|m| m["healthy"].as_bool() == Some(false))
            || crate::services::model_catalog::service(row.provider_metadata.as_ref())
                != ServiceKind::Decisions
        {
            anyhow::bail!("Enabled decision model required");
        }
        let profile = crate::services::model_catalog::profile(row.provider_metadata.as_ref())
            .ok_or_else(|| anyhow::anyhow!("Decision profile unavailable"))?;
        if profile.decisions.as_ref().is_none_or(|p| {
            !p.calibrated
                || ["noul", "choice", "score"]
                    .iter()
                    .any(|kind| !p.primitives.iter().any(|p| p == kind))
        }) {
            anyhow::bail!("Jev calibrated primitives required");
        }
        let provider = self
            .resolve_runtime_provider_config_for_session(
                org_id,
                &row.provider_id.to_string(),
                Some(session_id),
            )
            .await?
            .ok_or_else(|| anyhow::anyhow!("Provider unavailable"))?;
        if !self.driver_supports(&provider.provider_type, ServiceKind::Decisions) {
            anyhow::bail!("Provider does not support decisions");
        }
        let api_key = provider
            .api_key
            .filter(|k| !k.is_empty())
            .ok_or_else(|| anyhow::anyhow!("Provider credentials required"))?;
        Ok(Some(
            everruns_core::connection_services::DecisionModelBinding {
                model_id: row.id.to_string(),
                provider_id: row.provider_id.to_string(),
                provider_type: provider.provider_type,
                model: row.model_id,
                profile_key: crate::services::model_catalog::key(row.provider_metadata.as_ref()),
                api_key,
                base_url: provider.base_url,
                headers: provider
                    .request_options
                    .headers
                    .into_iter()
                    .map(|h| (h.name, h.value))
                    .collect(),
            },
        ))
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
    pub(crate) fn driver_supports(&self, provider_type: &str, service: ServiceKind) -> bool {
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

        if crate::services::model_catalog::service(model_row.provider_metadata.as_ref())
            != ServiceKind::Chat
        {
            anyhow::bail!("Chat model required");
        }
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

        if crate::services::model_catalog::service(model_row.provider_metadata.as_ref())
            != ServiceKind::Chat
        {
            anyhow::bail!("Chat model required");
        }
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
        self.resolve_runtime_provider_config_for_session(org_id, provider_id, None)
            .await
    }
    pub(crate) async fn resolve_runtime_provider_config_for_session(
        &self,
        org_id: i64,
        provider_id: &str,
        session: Option<uuid::Uuid>,
    ) -> Result<Option<ResolvedRuntimeProviderConfig>> {
        let id: ProviderId = provider_id
            .parse()
            .map_err(|_| anyhow::anyhow!("invalid provider id"))?;
        let Some(provider) = self.db.get_provider(org_id, id.uuid()).await? else {
            return Ok(None);
        };
        crate::services::chatgpt::check_session(&self.db, &provider, session).await?;
        let api_key = if provider.provider_type == "chatgpt" {
            Some(
                crate::services::chatgpt::access_token(
                    self.db.clone(),
                    self.encryption.clone(),
                    &provider,
                )
                .await?,
            )
        } else {
            self.resolve_api_key(&provider)?
        };

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
mod tests;
