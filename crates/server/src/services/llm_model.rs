// LLM Model service for business logic
//
// On create/update/delete, the LLM resolver cache is invalidated so that
// subsequent model resolutions pick up the new model config.

use crate::services::LlmResolverService;
use crate::storage::{
    StorageBackend,
    models::{CreateLlmModelRow, LlmModelRow, LlmModelWithProviderRow, UpdateLlmModel},
};
use anyhow::Result;
use everruns_core::{
    Caller, LlmModel, LlmModelSource, LlmModelStatus, LlmModelWithProvider, LlmProviderType,
    Permission, Policy, Rule, get_model_profile,
};
use everruns_macros::policy;
use std::sync::Arc;
use uuid::Uuid;

use crate::api::llm_models::{CreateLlmModelRequest, UpdateLlmModelRequest};

pub const LLM_MODEL_VIEW: Policy = Policy {
    id: "llm_model.view",
    rules: &[Rule::UserHasPermission(Permission::OrgLlmProvidersView)],
};
pub const LLM_MODEL_MANAGE: Policy = Policy {
    id: "llm_model.manage",
    rules: &[Rule::UserHasPermission(Permission::OrgLlmProvidersManage)],
};

pub struct LlmModelService {
    db: Arc<StorageBackend>,
    llm_resolver: Option<Arc<LlmResolverService>>,
}

impl LlmModelService {
    pub fn new(db: Arc<StorageBackend>) -> Self {
        Self {
            db,
            llm_resolver: None,
        }
    }

    pub fn with_resolver(db: Arc<StorageBackend>, resolver: Arc<LlmResolverService>) -> Self {
        Self {
            db,
            llm_resolver: Some(resolver),
        }
    }

    /// Invalidate resolver cache after model mutation.
    async fn invalidate_resolver_cache(&self, org_id: i64) {
        if let Some(ref resolver) = self.llm_resolver {
            resolver.invalidate_cache(org_id).await;
        }
    }

    #[policy(LLM_MODEL_MANAGE)]
    pub async fn create(
        &self,
        caller: &Caller,
        provider_id: Uuid,
        req: CreateLlmModelRequest,
    ) -> Result<LlmModel> {
        let input = CreateLlmModelRow {
            provider_id: provider_id.into(),
            model_id: req.model_id,
            display_name: req.display_name,
            capabilities: req.capabilities,
            installed: req.installed,
            is_favorite: req.is_favorite,
            source: "manual".to_string(), // User-created models are always manual
            provider_metadata: None,
        };

        let row = self.db.create_llm_model(caller.org_id, input).await?;
        self.invalidate_resolver_cache(caller.org_id).await;
        Ok(Self::row_to_model(&row))
    }

    #[policy(LLM_MODEL_VIEW)]
    pub async fn get_with_provider(
        &self,
        caller: &Caller,
        id: Uuid,
    ) -> Result<Option<LlmModelWithProvider>> {
        let row = self
            .db
            .get_llm_model_with_provider(caller.org_id, id)
            .await?;
        Ok(row.as_ref().map(Self::row_to_model_with_provider))
    }

    #[policy(LLM_MODEL_VIEW)]
    pub async fn list_for_provider(
        &self,
        caller: &Caller,
        provider_id: Uuid,
    ) -> Result<Vec<LlmModel>> {
        let rows = self
            .db
            .list_llm_models_for_provider(caller.org_id, provider_id)
            .await?;
        Ok(rows.iter().map(Self::row_to_model).collect())
    }

    #[policy(LLM_MODEL_VIEW)]
    pub async fn list_all(&self, caller: &Caller) -> Result<Vec<LlmModelWithProvider>> {
        let rows = self.db.list_all_llm_models(caller.org_id).await?;
        Ok(rows.iter().map(Self::row_to_model_with_provider).collect())
    }

    /// List all models with optional filters
    #[policy(LLM_MODEL_VIEW)]
    pub async fn list_all_with_filters(
        &self,
        caller: &Caller,
        source: Option<LlmModelSource>,
        include_stale: bool,
        favorites_only: bool,
    ) -> Result<Vec<LlmModelWithProvider>> {
        let rows = self.db.list_all_llm_models(caller.org_id).await?;

        // Get provider last_synced_at timestamps for stale detection
        let providers = self.db.list_llm_providers(caller.org_id).await?;
        let provider_sync_times: std::collections::HashMap<
            Uuid,
            Option<chrono::DateTime<chrono::Utc>>,
        > = providers
            .iter()
            .map(|p| (p.id.uuid(), p.last_synced_at))
            .collect();

        let models: Vec<LlmModelWithProvider> = rows
            .iter()
            .filter(|row| {
                // Filter by source
                if let Some(ref filter_source) = source {
                    let row_source: LlmModelSource =
                        row.source.parse().unwrap_or(LlmModelSource::Manual);
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

    #[policy(LLM_MODEL_MANAGE)]
    pub async fn update(
        &self,
        caller: &Caller,
        id: Uuid,
        req: UpdateLlmModelRequest,
    ) -> Result<Option<LlmModel>> {
        let input = UpdateLlmModel {
            model_id: req.model_id,
            display_name: req.display_name,
            capabilities: req.capabilities,
            installed: req.installed,
            is_favorite: req.is_favorite,
            status: req.status.map(|s| match s {
                LlmModelStatus::Active => "active".to_string(),
                LlmModelStatus::Disabled => "disabled".to_string(),
            }),
            last_seen_at: None,
            provider_metadata: None,
        };

        let row = self.db.update_llm_model(caller.org_id, id, input).await?;

        // If uninstalling a model, check if it was the org default and elect a new one
        if req.installed == Some(false)
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

    #[policy(LLM_MODEL_MANAGE)]
    pub async fn delete(&self, caller: &Caller, id: Uuid) -> Result<bool> {
        // Before deleting, check if this was the org default
        let was_default = self.is_org_default(caller.org_id, id).await?;
        let deleted = self.db.delete_llm_model(caller.org_id, id).await?;
        if deleted {
            if was_default {
                self.elect_new_default(caller.org_id).await?;
            }
            self.invalidate_resolver_cache(caller.org_id).await;
        }
        Ok(deleted)
    }

    /// Get the default model
    #[policy(LLM_MODEL_VIEW)]
    pub async fn get_default(&self, caller: &Caller) -> Result<Option<LlmModelWithProvider>> {
        let row = self.db.get_default_llm_model(caller.org_id).await?;
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

    /// Elect a new default model from installed models
    async fn elect_new_default(&self, org_id: i64) -> Result<()> {
        let all_models = self.db.list_all_llm_models(org_id).await?;
        let new_default = all_models
            .iter()
            .find(|m| m.installed && m.status == "active");

        let new_default_id = new_default.map(|m| m.id.uuid());
        self.db
            .upsert_organization_settings(org_id, new_default_id)
            .await?;
        self.invalidate_resolver_cache(org_id).await;
        Ok(())
    }

    fn row_to_model(row: &LlmModelRow) -> LlmModel {
        let capabilities: Vec<String> =
            serde_json::from_value(row.capabilities.clone()).unwrap_or_default();
        LlmModel {
            id: row.id,
            provider_id: row.provider_id,
            model_id: row.model_id.clone(),
            display_name: row.display_name.clone(),
            capabilities,
            installed: row.installed,
            is_favorite: row.is_favorite,
            status: match row.status.as_str() {
                "active" => LlmModelStatus::Active,
                _ => LlmModelStatus::Disabled,
            },
            source: row.source.parse().unwrap_or(LlmModelSource::Manual),
            created_at: row.created_at,
            updated_at: row.updated_at,
        }
    }

    fn row_to_model_with_provider(row: &LlmModelWithProviderRow) -> LlmModelWithProvider {
        let capabilities: Vec<String> =
            serde_json::from_value(row.capabilities.clone()).unwrap_or_default();
        let provider_type: LlmProviderType =
            row.provider_type.parse().unwrap_or(LlmProviderType::Openai);

        // Look up profile based on provider_type and model_id (readonly, not from DB)
        let profile = get_model_profile(&provider_type, &row.model_id);

        LlmModelWithProvider {
            id: row.id,
            provider_id: row.provider_id,
            model_id: row.model_id.clone(),
            display_name: row.display_name.clone(),
            capabilities,
            installed: row.installed,
            is_favorite: row.is_favorite,
            status: match row.status.as_str() {
                "active" => LlmModelStatus::Active,
                _ => LlmModelStatus::Disabled,
            },
            source: row.source.parse().unwrap_or(LlmModelSource::Manual),
            created_at: row.created_at,
            updated_at: row.updated_at,
            provider_name: row.provider_name.clone(),
            provider_type,
            profile,
        }
    }
}
