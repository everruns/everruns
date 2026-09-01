// LLM Model service for business logic
//
// On create/update/delete, the LLM resolver cache is invalidated so that
// subsequent model resolutions pick up the new model config.

use crate::errors::ResourceNotFoundError;
use crate::kernel_imports::{
    Caller, Permission, Policy, Rule, everruns_provider::model::Model,
    everruns_provider::model::ModelProfile, everruns_provider::model::ModelSource,
    everruns_provider::model::ModelWithProvider,
    everruns_provider::model_profiles::get_model_profile, everruns_provider::provider::DriverId,
    everruns_provider::typed_id::ProviderId,
};
use crate::services::ProviderResolverService;
use crate::storage::{
    StorageBackend,
    models::{CreateModelRow, ModelRow, ModelWithProviderRow, UpdateModel},
};
use anyhow::Result;
use std::sync::Arc;
use tracing::error;
use uuid::Uuid;

use crate::api::models::{CreateModelRequest, UpdateModelRequest};

pub const LLM_MODEL_VIEW: Policy = Policy {
    id: "model.view",
    rules: &[Rule::UserHasPermission(Permission::OrgProvidersView)],
};
pub const LLM_MODEL_MANAGE: Policy = Policy {
    id: "model.manage",
    rules: &[Rule::UserHasPermission(Permission::OrgProvidersManage)],
};

pub struct ModelService {
    db: Arc<StorageBackend>,
    provider_resolver: Option<Arc<ProviderResolverService>>,
}

impl ModelService {
    pub fn new(db: Arc<StorageBackend>) -> Self {
        Self {
            db,
            provider_resolver: None,
        }
    }

    pub fn with_resolver(db: Arc<StorageBackend>, resolver: Arc<ProviderResolverService>) -> Self {
        Self {
            db,
            provider_resolver: Some(resolver),
        }
    }

    /// Invalidate resolver cache after model mutation.
    async fn invalidate_resolver_cache(&self, org_id: i64) {
        if let Some(ref resolver) = self.provider_resolver {
            resolver.invalidate_cache(org_id).await;
        }
    }

    pub async fn create(
        &self,
        caller: &Caller,
        provider_id: Uuid,
        req: CreateModelRequest,
    ) -> Result<Model> {
        let provider = self.get_provider(caller.org_id, provider_id).await?;
        Self::require_unmanaged_provider(&provider)?;

        let input = CreateModelRow {
            provider_id: provider.id,
            model_id: req.model_id,
            display_name: req.display_name,
            capabilities: req.capabilities,
            enabled: req.enabled,
            is_favorite: req.is_favorite,
            source: "manual".to_string(), // User-created models are always manual
            provider_metadata: None,
        };

        let row = self.db.create_model(caller.org_id, input).await?;
        self.invalidate_resolver_cache(caller.org_id).await;
        Ok(Self::row_to_model(&row))
    }

    pub async fn get_with_provider(
        &self,
        caller: &Caller,
        id: Uuid,
    ) -> Result<Option<ModelWithProvider>> {
        // EVE-417: log the underlying DB error with org/model context so
        // operators can diagnose the org-scoped read failures that surface
        // through MCP as `internal: <message>`. Error still propagates.
        let row = self
            .db
            .get_model_with_provider(caller.org_id, id)
            .await
            .inspect_err(|err| {
                error!(
                    org_id = caller.org_id,
                    model_id = %id,
                    error = %err,
                    "Failed to read llm model"
                );
            })?;
        Ok(row.as_ref().map(Self::row_to_model_with_provider))
    }

    pub async fn list_for_provider(
        &self,
        caller: &Caller,
        provider_id: Uuid,
    ) -> Result<Vec<Model>> {
        let rows = self
            .db
            .list_models_for_provider(caller.org_id, provider_id)
            .await
            .inspect_err(|err| {
                error!(
                    org_id = caller.org_id,
                    provider_id = %provider_id,
                    error = %err,
                    "Failed to list llm models for provider"
                );
            })?;
        Ok(rows.iter().map(Self::row_to_model).collect())
    }

    pub async fn list_all(&self, caller: &Caller) -> Result<Vec<ModelWithProvider>> {
        let rows = self
            .db
            .list_all_models(caller.org_id)
            .await
            .inspect_err(|err| {
                error!(
                    org_id = caller.org_id,
                    error = %err,
                    "Failed to list llm models"
                );
            })?;
        Ok(rows.iter().map(Self::row_to_model_with_provider).collect())
    }

    /// List all models with optional filters
    pub async fn list_all_with_filters(
        &self,
        caller: &Caller,
        source: Option<ModelSource>,
        include_stale: bool,
        favorites_only: bool,
    ) -> Result<Vec<ModelWithProvider>> {
        // EVE-417: same diagnostic logging as `list_all`/`list_for_provider`.
        let rows = self
            .db
            .list_all_models(caller.org_id)
            .await
            .inspect_err(|err| {
                error!(
                    org_id = caller.org_id,
                    error = %err,
                    "Failed to list llm models for filtering"
                );
            })?;

        // Get provider last_synced_at timestamps for stale detection
        let providers = self
            .db
            .list_providers(caller.org_id)
            .await
            .inspect_err(|err| {
                error!(
                    org_id = caller.org_id,
                    error = %err,
                    "Failed to list llm providers for stale detection"
                );
            })?;
        let provider_sync_times: std::collections::HashMap<
            Uuid,
            Option<chrono::DateTime<chrono::Utc>>,
        > = providers
            .iter()
            .map(|p| (p.id.uuid(), p.last_synced_at))
            .collect();

        let models: Vec<ModelWithProvider> = rows
            .iter()
            .filter(|row| {
                // Filter by source
                if let Some(ref filter_source) = source {
                    let row_source: ModelSource = row.source.parse().unwrap_or(ModelSource::Manual);
                    if row_source != *filter_source {
                        return false;
                    }
                }

                // Filter by favorites
                if favorites_only && !row.is_favorite {
                    return false;
                }

                // Filter stale models (discovered models not seen in most recent sync)
                // Only discovered models can be stale
                if !include_stale
                    && row.source == "discovered"
                    && let Some(Some(last_synced)) =
                        provider_sync_times.get(&row.provider_id.uuid())
                {
                    // Model is stale if last_seen_at < provider.last_synced_at
                    if let Some(last_seen) = row.last_seen_at {
                        if last_seen < *last_synced {
                            return false;
                        }
                    } else {
                        // No last_seen_at means never seen in sync - stale
                        return false;
                    }
                }

                true
            })
            .map(Self::row_to_model_with_provider)
            .collect();

        Ok(models)
    }

    pub async fn update(
        &self,
        caller: &Caller,
        id: Uuid,
        req: UpdateModelRequest,
    ) -> Result<Option<Model>> {
        let existing = match self.db.get_model(caller.org_id, id).await? {
            Some(row) => row,
            None => return Ok(None),
        };
        let existing_provider = self
            .get_provider(caller.org_id, existing.provider_id.uuid())
            .await?;

        // THREAT[TM-AUTHZ]: managed providers own their model catalog. Tenant
        // admins may change only org preferences on those catalog rows.
        if existing_provider.managed
            && (req.provider_id.is_some()
                || req.model_id.is_some()
                || req.display_name.is_some()
                || req.capabilities.is_some())
        {
            return Err(Self::managed_catalog_error());
        }

        let provider_id = match req.provider_id.as_deref() {
            Some(provider_id) => Some(
                provider_id
                    .parse::<ProviderId>()
                    .map(|id| id.uuid())
                    .map_err(|err| anyhow::anyhow!("Invalid provider ID: {err}"))?,
            ),
            None => None,
        };
        let provider_id = if let Some(provider_id) = provider_id {
            let provider = self.get_provider(caller.org_id, provider_id).await?;
            Self::require_unmanaged_provider(&provider)?;
            Some(provider.id)
        } else {
            None
        };

        let input = UpdateModel {
            provider_id,
            model_id: req.model_id,
            display_name: req.display_name,
            capabilities: req.capabilities,
            enabled: req.enabled,
            is_favorite: req.is_favorite,
            last_seen_at: None,
            provider_metadata: None,
        };

        let row = self.db.update_model(caller.org_id, id, input).await?;

        // If disabling a model, check if it was the org default and elect a new one
        if req.enabled == Some(false)
            && let Some(ref row) = row
        {
            self.maybe_elect_new_default(caller.org_id, row.id.uuid())
                .await?;
        }
        if row.is_some() {
            self.invalidate_resolver_cache(caller.org_id).await;
        }
        Ok(row.as_ref().map(Self::row_to_model))
    }

    pub async fn delete(&self, caller: &Caller, id: Uuid) -> Result<bool> {
        if let Some(model) = self.db.get_model(caller.org_id, id).await? {
            let provider = self
                .get_provider(caller.org_id, model.provider_id.uuid())
                .await?;
            Self::require_unmanaged_provider(&provider)?;
        }

        // Before deleting, check if this was the org default
        let was_default = self.is_org_default(caller.org_id, id).await?;
        let deleted = self.db.delete_model(caller.org_id, id).await?;
        if deleted {
            if was_default {
                self.elect_new_default(caller.org_id).await?;
            }
            self.invalidate_resolver_cache(caller.org_id).await;
        }
        Ok(deleted)
    }

    /// Get the default model
    pub async fn get_default(&self, caller: &Caller) -> Result<Option<ModelWithProvider>> {
        let row = self.db.get_default_model(caller.org_id).await?;
        Ok(row.as_ref().map(Self::row_to_model_with_provider))
    }

    /// Set the org default model
    pub async fn set_default(&self, org_id: i64, model_id: Uuid) -> Result<()> {
        self.db
            .upsert_organization_settings(org_id, Some(model_id))
            .await?;
        self.invalidate_resolver_cache(org_id).await;
        Ok(())
    }

    /// Check if a model is the current org default
    async fn is_org_default(&self, org_id: i64, model_id: Uuid) -> Result<bool> {
        if let Some(settings) = self.db.get_organization_settings(org_id).await?
            && let Some(default_id) = settings.default_model_id
        {
            return Ok(default_id.uuid() == model_id);
        }
        Ok(false)
    }

    /// If the given model_id is the org default, elect a new one
    async fn maybe_elect_new_default(&self, org_id: i64, model_id: Uuid) -> Result<()> {
        if self.is_org_default(org_id, model_id).await? {
            self.elect_new_default(org_id).await?;
        }
        Ok(())
    }

    /// Elect a new default model from enabled models
    async fn elect_new_default(&self, org_id: i64) -> Result<()> {
        let all_models = self.db.list_all_models(org_id).await?;
        let new_default = all_models.iter().find(|m| m.enabled);

        let new_default_id = new_default.map(|m| m.id.uuid());
        self.db
            .upsert_organization_settings(org_id, new_default_id)
            .await?;
        self.invalidate_resolver_cache(org_id).await;
        Ok(())
    }

    async fn get_provider(
        &self,
        org_id: i64,
        provider_id: Uuid,
    ) -> Result<crate::storage::models::ProviderRow> {
        self.db
            .get_provider(org_id, provider_id)
            .await?
            .ok_or_else(|| ResourceNotFoundError::new("Provider").into())
    }

    fn require_unmanaged_provider(provider: &crate::storage::models::ProviderRow) -> Result<()> {
        if provider.managed {
            return Err(Self::managed_catalog_error());
        }
        Ok(())
    }

    fn managed_catalog_error() -> anyhow::Error {
        everruns_core::PolicyError::denied(
            "provider_managed",
            "This provider's model catalog is managed by the host and cannot be modified.",
        )
        .into()
    }

    fn row_to_model(row: &ModelRow) -> Model {
        let capabilities: Vec<String> =
            serde_json::from_value(row.capabilities.clone()).unwrap_or_default();
        Model {
            id: row.id,
            provider_id: row.provider_id,
            model_id: row.model_id.clone(),
            display_name: row.display_name.clone(),
            capabilities,
            enabled: row.enabled,
            is_favorite: row.is_favorite,
            source: row.source.parse().unwrap_or(ModelSource::Manual),
            created_at: row.created_at,
            updated_at: row.updated_at,
        }
    }

    fn row_to_model_with_provider(row: &ModelWithProviderRow) -> ModelWithProvider {
        let capabilities: Vec<String> =
            serde_json::from_value(row.capabilities.clone()).unwrap_or_default();
        let provider_type: DriverId = row.provider_type.parse().unwrap_or(DriverId::OpenAI);

        // Look up hardcoded profile, then try discovered profile from provider_metadata.
        // Hardcoded profiles take precedence; discovered provider catalog data fills gaps.
        let hardcoded = get_model_profile(&provider_type, &row.model_id);
        let profile = if hardcoded.is_some() {
            // Merge: hardcoded base with discovered limits/capabilities as fallback
            let discovered = Self::extract_discovered_profile(row);
            match (hardcoded, discovered) {
                (Some(h), Some(d)) => Some(Self::merge_profiles(h, d)),
                (Some(h), None) => Some(h),
                _ => unreachable!(),
            }
        } else {
            // No hardcoded profile — use discovered if available
            Self::extract_discovered_profile(row)
        };

        // A model is healthy when its provider is active and has an API key
        // configured. This will likely grow to include live reachability
        // checks; keep the derivation in one place.
        let healthy = row.provider_status == "active" && row.provider_api_key_set;

        // Vendor/brand tag from the model registry (drives UI branding),
        // independent of the configured provider type.
        let model_vendor =
            everruns_provider::model_profiles::get_model_vendor(&provider_type, &row.model_id);

        ModelWithProvider {
            id: row.id,
            provider_id: row.provider_id,
            model_id: row.model_id.clone(),
            display_name: row.display_name.clone(),
            capabilities,
            enabled: row.enabled,
            is_favorite: row.is_favorite,
            source: row.source.parse().unwrap_or(ModelSource::Manual),
            created_at: row.created_at,
            updated_at: row.updated_at,
            provider_name: row.provider_name.clone(),
            provider_type,
            healthy,
            profile,
            model_vendor,
        }
    }

    /// Extract the discovered profile from provider_metadata JSON.
    fn extract_discovered_profile(row: &ModelWithProviderRow) -> Option<ModelProfile> {
        let metadata = row.provider_metadata.as_ref()?;
        let profile_val = metadata.get("discovered_profile")?;
        serde_json::from_value(profile_val.clone()).ok()
    }

    /// Merge a hardcoded profile with a discovered profile.
    /// Hardcoded values take precedence; discovered values fill gaps.
    fn merge_profiles(hardcoded: ModelProfile, discovered: ModelProfile) -> ModelProfile {
        ModelProfile {
            // Hardcoded always wins for curated fields
            name: hardcoded.name,
            family: hardcoded.family,
            description: hardcoded.description.or(discovered.description),
            release_date: hardcoded.release_date.or(discovered.release_date),
            last_updated: hardcoded.last_updated.or(discovered.last_updated),
            attachment: hardcoded.attachment,
            reasoning: hardcoded.reasoning,
            temperature: hardcoded.temperature,
            knowledge: hardcoded.knowledge.or(discovered.knowledge),
            tool_call: hardcoded.tool_call,
            structured_output: hardcoded.structured_output,
            open_weights: hardcoded.open_weights,
            cost: hardcoded.cost.or(discovered.cost),
            // Limits: hardcoded values are authoritative; use discovered values only as fallback
            limits: hardcoded.limits.or(discovered.limits),
            modalities: hardcoded.modalities.or(discovered.modalities),
            reasoning_effort: hardcoded.reasoning_effort.or(discovered.reasoning_effort),
            speed: hardcoded.speed.or(discovered.speed),
            verbosity: hardcoded.verbosity.or(discovered.verbosity),
            tool_search: hardcoded.tool_search,
            supported_parameters: if hardcoded.supported_parameters.is_empty() {
                discovered.supported_parameters
            } else {
                hardcoded.supported_parameters
            },
            supports_phases: hardcoded.supports_phases,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::storage::{CreateOrganizationRow, CreateProviderRow};
    use everruns_core::{DEFAULT_ORG_ID, PolicyError};

    fn build_create_request() -> CreateModelRequest {
        CreateModelRequest {
            model_id: "test-model".to_string(),
            display_name: "Test Model".to_string(),
            capabilities: vec!["chat".to_string()],
            enabled: true,
            is_favorite: false,
        }
    }

    async fn create_second_org(db: &StorageBackend) -> i64 {
        db.create_organization_with_id(
            2,
            CreateOrganizationRow {
                public_id: "org_2".to_string(),
                name: "Org 2".to_string(),
                created_by: None,
            },
        )
        .await
        .unwrap()
        .unwrap()
        .org_id
    }

    async fn create_provider(
        db: &StorageBackend,
        org_id: i64,
    ) -> everruns_provider::typed_id::ProviderId {
        db.create_provider(
            org_id,
            CreateProviderRow {
                name: format!("Provider {org_id}"),
                provider_type: "openai".to_string(),
                base_url: None,
                api_key_encrypted: None,
                settings: None,
            },
        )
        .await
        .unwrap()
        .id
    }

    async fn mark_provider_managed(db: &StorageBackend, provider_id: ProviderId) {
        assert!(
            db.set_provider_managed(DEFAULT_ORG_ID, provider_id.uuid(), true)
                .await
                .unwrap()
        );
    }

    fn assert_managed_policy_error(err: anyhow::Error) {
        assert!(err.downcast_ref::<PolicyError>().is_some(), "{err:#}");
    }

    #[tokio::test]
    async fn create_rejects_managed_provider() {
        let db = Arc::new(StorageBackend::in_memory());
        let service = ModelService::new(db.clone());
        let caller = Caller::internal(DEFAULT_ORG_ID);
        let provider_id = create_provider(&db, DEFAULT_ORG_ID).await;
        mark_provider_managed(&db, provider_id).await;

        let err = service
            .create(&caller, provider_id.uuid(), build_create_request())
            .await
            .unwrap_err();

        assert_managed_policy_error(err);
    }

    #[tokio::test]
    async fn managed_model_allows_preference_updates_only() {
        let db = Arc::new(StorageBackend::in_memory());
        let service = ModelService::new(db.clone());
        let caller = Caller::internal(DEFAULT_ORG_ID);
        let provider_id = create_provider(&db, DEFAULT_ORG_ID).await;
        let model = service
            .create(&caller, provider_id.uuid(), build_create_request())
            .await
            .unwrap();
        mark_provider_managed(&db, provider_id).await;

        let updated = service
            .update(
                &caller,
                model.id.uuid(),
                UpdateModelRequest {
                    provider_id: None,
                    model_id: None,
                    display_name: None,
                    capabilities: None,
                    enabled: Some(false),
                    is_favorite: Some(true),
                },
            )
            .await
            .unwrap()
            .unwrap();

        assert!(!updated.enabled);
        assert!(updated.is_favorite);
    }

    #[tokio::test]
    async fn managed_model_rejects_catalog_update_and_delete() {
        let db = Arc::new(StorageBackend::in_memory());
        let service = ModelService::new(db.clone());
        let caller = Caller::internal(DEFAULT_ORG_ID);
        let provider_id = create_provider(&db, DEFAULT_ORG_ID).await;
        let model = service
            .create(&caller, provider_id.uuid(), build_create_request())
            .await
            .unwrap();
        mark_provider_managed(&db, provider_id).await;

        let err = service
            .update(
                &caller,
                model.id.uuid(),
                UpdateModelRequest {
                    provider_id: None,
                    model_id: Some("unauthorized-model".to_string()),
                    display_name: None,
                    capabilities: None,
                    enabled: None,
                    is_favorite: None,
                },
            )
            .await
            .unwrap_err();
        assert_managed_policy_error(err);

        let err = service.delete(&caller, model.id.uuid()).await.unwrap_err();
        assert_managed_policy_error(err);
        assert!(
            db.get_model(DEFAULT_ORG_ID, model.id.uuid())
                .await
                .unwrap()
                .is_some()
        );
    }

    #[tokio::test]
    async fn update_cannot_move_model_to_managed_provider() {
        let db = Arc::new(StorageBackend::in_memory());
        let service = ModelService::new(db.clone());
        let caller = Caller::internal(DEFAULT_ORG_ID);
        let first_provider_id = create_provider(&db, DEFAULT_ORG_ID).await;
        let managed_provider_id = create_provider(&db, DEFAULT_ORG_ID).await;
        mark_provider_managed(&db, managed_provider_id).await;
        let model = service
            .create(&caller, first_provider_id.uuid(), build_create_request())
            .await
            .unwrap();

        let err = service
            .update(
                &caller,
                model.id.uuid(),
                UpdateModelRequest {
                    provider_id: Some(managed_provider_id.to_string()),
                    model_id: None,
                    display_name: None,
                    capabilities: None,
                    enabled: None,
                    is_favorite: None,
                },
            )
            .await
            .unwrap_err();

        assert_managed_policy_error(err);
    }

    #[tokio::test]
    async fn create_rejects_provider_from_another_org() {
        let db = Arc::new(StorageBackend::in_memory());
        let service = ModelService::new(db.clone());
        let caller = Caller::internal(DEFAULT_ORG_ID);
        let other_org_id = create_second_org(&db).await;
        let other_provider_id = create_provider(&db, other_org_id).await;

        let err = service
            .create(&caller, other_provider_id.uuid(), build_create_request())
            .await
            .unwrap_err();

        assert_eq!(err.to_string(), "Provider not found");
    }

    #[tokio::test]
    async fn update_can_move_model_to_another_provider_in_same_org() {
        let db = Arc::new(StorageBackend::in_memory());
        let service = ModelService::new(db.clone());
        let caller = Caller::internal(DEFAULT_ORG_ID);
        let first_provider_id = create_provider(&db, DEFAULT_ORG_ID).await;
        let second_provider_id = create_provider(&db, DEFAULT_ORG_ID).await;
        let model = service
            .create(&caller, first_provider_id.uuid(), build_create_request())
            .await
            .unwrap();

        let updated = service
            .update(
                &caller,
                model.id.uuid(),
                UpdateModelRequest {
                    provider_id: Some(second_provider_id.to_string()),
                    model_id: None,
                    display_name: None,
                    capabilities: None,
                    enabled: None,
                    is_favorite: None,
                },
            )
            .await
            .unwrap()
            .unwrap();

        assert_eq!(updated.provider_id, second_provider_id);
    }

    #[tokio::test]
    async fn update_rejects_provider_from_another_org() {
        let db = Arc::new(StorageBackend::in_memory());
        let service = ModelService::new(db.clone());
        let caller = Caller::internal(DEFAULT_ORG_ID);
        let provider_id = create_provider(&db, DEFAULT_ORG_ID).await;
        let other_org_id = create_second_org(&db).await;
        let other_provider_id = create_provider(&db, other_org_id).await;
        let model = service
            .create(&caller, provider_id.uuid(), build_create_request())
            .await
            .unwrap();

        let err = service
            .update(
                &caller,
                model.id.uuid(),
                UpdateModelRequest {
                    provider_id: Some(other_provider_id.to_string()),
                    model_id: None,
                    display_name: None,
                    capabilities: None,
                    enabled: None,
                    is_favorite: None,
                },
            )
            .await
            .unwrap_err();

        assert_eq!(err.to_string(), "Provider not found");
    }

    // ========================================================================
    // Profile merge tests
    // ========================================================================

    fn base_profile() -> ModelProfile {
        ModelProfile {
            name: "Test".into(),
            family: "test".into(),
            description: None,
            release_date: None,
            last_updated: None,
            attachment: true,
            reasoning: false,
            temperature: true,
            knowledge: None,
            tool_call: true,
            structured_output: true,
            open_weights: false,
            cost: None,
            limits: None,
            modalities: None,
            reasoning_effort: None,
            speed: None,
            verbosity: None,
            tool_search: false,
            supported_parameters: Vec::new(),
            supports_phases: false,
        }
    }

    #[test]
    fn merge_hardcoded_wins_for_curated_fields() {
        let hardcoded = ModelProfile {
            name: "Hardcoded Name".into(),
            family: "hardcoded-family".into(),
            cost: Some(everruns_provider::model::ModelCost {
                input: 5.0,
                output: 25.0,
                cache_read: None,
                cost_tiers: vec![],
            }),
            ..base_profile()
        };
        let discovered = ModelProfile {
            name: "Discovered Name".into(),
            family: "discovered-family".into(),
            knowledge: Some("2025-01-01".into()),
            cost: Some(everruns_provider::model::ModelCost {
                input: 0.5,
                output: 1.0,
                cache_read: Some(0.1),
                cost_tiers: vec![],
            }),
            ..base_profile()
        };

        let merged = ModelService::merge_profiles(hardcoded, discovered);
        assert_eq!(merged.name, "Hardcoded Name");
        assert_eq!(merged.family, "hardcoded-family");
        assert_eq!(merged.knowledge.as_deref(), Some("2025-01-01"));
        assert_eq!(merged.cost.unwrap().input, 5.0);
    }

    #[test]
    fn merge_discovered_fills_gaps() {
        use everruns_provider::model::ModelLimits;

        let hardcoded = ModelProfile {
            limits: None,
            ..base_profile()
        };
        let discovered = ModelProfile {
            limits: Some(ModelLimits {
                context: 200_000,
                input: None,
                output: 64_000,
                max_media: None,
            }),
            knowledge: Some("2025-02-01".into()),
            cost: Some(everruns_provider::model::ModelCost {
                input: 0.5,
                output: 1.0,
                cache_read: Some(0.1),
                cost_tiers: vec![],
            }),
            supported_parameters: vec!["tools".into(), "temperature".into()],
            ..base_profile()
        };

        let merged = ModelService::merge_profiles(hardcoded, discovered);
        assert!(merged.limits.is_some());
        assert_eq!(merged.limits.unwrap().context, 200_000);
        assert_eq!(merged.knowledge.as_deref(), Some("2025-02-01"));
        assert_eq!(merged.cost.unwrap().output, 1.0);
        assert_eq!(
            merged.supported_parameters,
            vec!["tools".to_string(), "temperature".to_string()]
        );
    }

    #[test]
    fn merge_hardcoded_limits_take_precedence() {
        use everruns_provider::model::ModelLimits;

        let hardcoded = ModelProfile {
            limits: Some(ModelLimits {
                context: 128_000,
                input: None,
                output: 16_384,
                max_media: None,
            }),
            ..base_profile()
        };
        let discovered = ModelProfile {
            limits: Some(ModelLimits {
                context: 200_000,
                input: None,
                output: 64_000,
                max_media: None,
            }),
            ..base_profile()
        };

        let merged = ModelService::merge_profiles(hardcoded, discovered);
        assert_eq!(merged.limits.unwrap().context, 128_000);
    }

    #[test]
    fn merge_preserves_hardcoded_verbosity() {
        use everruns_provider::model::{Verbosity, VerbosityConfig, VerbosityValue};

        let hardcoded = ModelProfile {
            verbosity: Some(VerbosityConfig {
                values: vec![VerbosityValue {
                    value: Verbosity::Medium,
                    name: "Medium".into(),
                }],
                default: Verbosity::Medium,
            }),
            ..base_profile()
        };

        let merged = ModelService::merge_profiles(hardcoded, base_profile());

        assert_eq!(merged.verbosity.unwrap().default, Verbosity::Medium);
    }

    #[test]
    fn extract_discovered_profile_from_metadata() {
        use crate::storage::models::ModelWithProviderRow;
        use chrono::Utc;

        let profile = base_profile();
        let metadata = serde_json::json!({
            "discovered_profile": profile,
        });

        let row = ModelWithProviderRow {
            id: everruns_provider::typed_id::ModelId::new(),
            org_id: 1,
            provider_id: everruns_provider::typed_id::ProviderId::new(),
            model_id: "test-model".into(),
            display_name: "Test".into(),
            capabilities: serde_json::json!([]),
            is_favorite: false,
            enabled: true,
            source: "discovered".into(),
            last_seen_at: None,
            provider_metadata: Some(metadata),
            created_at: Utc::now(),
            updated_at: Utc::now(),
            provider_name: "TestProvider".into(),
            provider_type: "anthropic".into(),
            provider_api_key_set: true,
            provider_status: "active".into(),
        };

        let extracted = ModelService::extract_discovered_profile(&row);
        assert!(extracted.is_some());
        assert_eq!(extracted.unwrap().name, "Test");
    }

    #[test]
    fn extract_discovered_profile_returns_none_without_metadata() {
        use crate::storage::models::ModelWithProviderRow;
        use chrono::Utc;

        let row = ModelWithProviderRow {
            id: everruns_provider::typed_id::ModelId::new(),
            org_id: 1,
            provider_id: everruns_provider::typed_id::ProviderId::new(),
            model_id: "test-model".into(),
            display_name: "Test".into(),
            capabilities: serde_json::json!([]),
            is_favorite: false,
            enabled: true,
            source: "manual".into(),
            last_seen_at: None,
            provider_metadata: None,
            created_at: Utc::now(),
            updated_at: Utc::now(),
            provider_name: "TestProvider".into(),
            provider_type: "openai".into(),
            provider_api_key_set: true,
            provider_status: "active".into(),
        };

        let extracted = ModelService::extract_discovered_profile(&row);
        assert!(extracted.is_none());
    }
}
