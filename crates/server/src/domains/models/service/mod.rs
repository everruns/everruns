// LLM Model service for business logic
//
// On create/update/delete, the LLM resolver cache is invalidated so that
// subsequent model resolutions pick up the new model config.

use crate::errors::ResourceNotFoundError;
use crate::kernel_imports::{
    Caller, Permission, Policy, Rule, contracts::model::Model, contracts::model::ModelProfile,
    contracts::model::ModelSource, contracts::model::ModelWithProvider,
    contracts::model_profiles::get_model_profile, contracts::provider::DriverId,
    contracts::typed_id::ProviderId,
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

use crate::domains::models::types::{CreateModelRequest, UpdateModelRequest};

pub const LLM_MODEL_VIEW: Policy = Policy {
    id: "model.view",
    rules: &[Rule::UserHasPermission(Permission::OrgProvidersView)],
};
pub const LLM_MODEL_MANAGE: Policy = Policy {
    id: "model.manage",
    rules: &[Rule::UserHasPermission(Permission::OrgProvidersManage)],
};

/// What [`ModelService::bootstrap_intelligence`] changed. All-zero means the org
/// was already usable and nothing was touched.
#[derive(Debug, Default, Clone, PartialEq, Eq)]
pub struct IntelligenceBootstrap {
    /// How many of the provider's models were switched on.
    pub enabled_models: usize,
    /// How many of them were starred as favourites.
    pub favorited_models: usize,
    /// The model elected as the org default, when one was elected.
    pub default_model_id: Option<Uuid>,
}

impl IntelligenceBootstrap {
    pub fn changed(&self) -> bool {
        self.enabled_models > 0 || self.favorited_models > 0 || self.default_model_id.is_some()
    }
}

/// How many of a provider's ranked chat models the bootstrap switches on, and
/// how many of those it stars. Shortlist sizes, not limits on what a user may
/// enable afterwards in Settings.
const BOOTSTRAP_ENABLE_LIMIT: usize = 8;
const BOOTSTRAP_FAVORITE_LIMIT: usize = 3;

/// Curated first pick per vendor, by model family. Recency alone elects the
/// newest model the day it ships, which is the wrong default for everyday
/// agent work: GPT-6 Astra costs 100x GPT-6 Luna on input without being the
/// better fit for most runs. Luna is also the platform fallback
/// (`platform::PLATFORM_DEFAULT_MODEL_ID`), so a freshly credentialed org lands
/// on the same model the default org already uses. Families not listed here
/// keep falling back to the recency order below.
const PREFERRED_DEFAULT_FAMILIES: &[&str] = &["gpt-6-luna"];

/// Default-model preference order: the curated pick first, then newest release,
/// then the model with the fewest missing agent-relevant traits. Lower sorts
/// better.
#[derive(Debug, PartialEq, Eq, PartialOrd, Ord)]
struct ModelRank {
    /// 0 for a curated family, 1 otherwise, so the curated pick sorts first.
    curated: u8,
    /// `Reverse` so a later date — and a known date over an unknown one — wins.
    release: std::cmp::Reverse<String>,
    capability_gaps: u8,
}

impl ModelRank {
    fn of(profile: &ModelProfile) -> Self {
        let gaps = u8::from(!profile.reasoning)
            + u8::from(!profile.attachment)
            + u8::from(!profile.structured_output);
        Self {
            curated: u8::from(!PREFERRED_DEFAULT_FAMILIES.contains(&profile.family.as_str())),
            release: std::cmp::Reverse(profile.release_date.clone().unwrap_or_default()),
            capability_gaps: gaps,
        }
    }
}

mod catalog_helpers;

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
        let provider = self.get_visible_provider(caller, provider_id).await?;
        Self::require_unmanaged_provider(&provider)?;
        let metadata = crate::domains::models::catalog::assign(
            &provider.provider_type,
            &req.model_id,
            &req.capabilities,
            req.profile_key.as_deref(),
            req.service,
            None,
        )?;
        self.validate_model_service(
            &provider.provider_type,
            crate::domains::models::catalog::service(Some(&metadata)),
        )?;

        // Discovery populates a provider's catalog the moment it gains a
        // credential, so an explicit create now routinely names a model the
        // system itself just wrote. The caller means "this model should exist
        // with these settings", so adopt the discovered row instead of
        // answering 409 for something they never created. A collision with a
        // row the user added themselves is a real duplicate and still conflicts
        // on the unique index.
        if let Some(discovered) = self
            .db
            .list_models_for_provider(caller.org_id, provider_id)
            .await?
            .into_iter()
            .find(|row| {
                row.model_id == req.model_id
                    && matches!(row.source.as_str(), "discovered" | "predefined")
            })
        {
            let update = UpdateModel {
                provider_metadata: Some(metadata),
                display_name: Some(req.display_name),
                // An omitted `capabilities` must not blank what discovery
                // learned from the provider — resolution depends on it.
                capabilities: (!req.capabilities.is_empty()).then_some(req.capabilities),
                enabled: Some(req.enabled),
                is_favorite: Some(req.is_favorite),
                ..Default::default()
            };
            let row = self
                .db
                .update_model(caller.org_id, discovered.id.uuid(), update)
                .await?
                .ok_or_else(|| anyhow::anyhow!("discovered model vanished while being adopted"))?;
            self.invalidate_resolver_cache(caller.org_id).await;
            return Ok(Self::row_to_model(&row));
        }

        let input = CreateModelRow {
            provider_id: provider.id,
            model_id: req.model_id,
            display_name: req.display_name,
            capabilities: req.capabilities,
            enabled: req.enabled,
            is_favorite: req.is_favorite,
            source: "manual".to_string(), // User-created models are always manual
            provider_metadata: Some(metadata),
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
        if let Some(ref row) = row {
            self.get_visible_provider(caller, row.provider_id.uuid())
                .await?;
        }
        Ok(row.as_ref().map(Self::row_to_model_with_provider))
    }

    pub async fn list_for_provider(
        &self,
        caller: &Caller,
        provider_id: Uuid,
    ) -> Result<Vec<Model>> {
        self.get_visible_provider(caller, provider_id).await?;
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
        let mut rows = self
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
        Self::order_personal_catalog(&mut rows);
        let visible = self.visible_provider_ids(caller).await?;
        Ok(rows
            .iter()
            .filter(|r| visible.contains(&r.provider_id))
            .map(Self::row_to_model_with_provider)
            .collect())
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
        let mut rows = self
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
        Self::order_personal_catalog(&mut rows);

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

        let visible: std::collections::HashSet<_> = providers
            .iter()
            .filter(|p| crate::domains::user_connections::chatgpt::visible(&p.settings, caller))
            .map(|p| p.id)
            .collect();
        let models: Vec<ModelWithProvider> = rows
            .iter()
            .filter(|row| {
                if !visible.contains(&row.provider_id) {
                    return false;
                }
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
        // Admin reads must see disabled rows: `get_model` is the resolution
        // path and filters `enabled = TRUE`, so using it here made enabling a
        // disabled model 404 (every discovered model starts disabled).
        let existing = match self.db.get_model_for_mutation(caller.org_id, id).await? {
            Some(row) => row,
            None => return Ok(None),
        };
        let existing_provider = self
            .get_visible_provider(caller, existing.provider_id.uuid())
            .await?;

        // THREAT[TM-AUTHZ]: managed providers own their model catalog. Tenant
        // admins may change only org preferences on those catalog rows.
        if existing_provider.managed
            && (req.provider_id.is_some()
                || req.model_id.is_some()
                || req.display_name.is_some()
                || req.capabilities.is_some()
                || req.service.is_some()
                || req.profile_key.is_some())
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
            let provider = self.get_visible_provider(caller, provider_id).await?;
            Self::require_unmanaged_provider(&provider)?;
            Some(provider.id)
        } else {
            None
        };

        let identity_changed = req
            .model_id
            .as_ref()
            .is_some_and(|id| id != &existing.model_id)
            || provider_id.is_some_and(|id| id != existing.provider_id);
        let metadata = if identity_changed
            || req.profile_key.is_some()
            || req.service.is_some_and(|service| {
                service
                    != crate::domains::models::catalog::service(existing.provider_metadata.as_ref())
            }) {
            let target_provider = if let Some(id) = provider_id {
                self.get_visible_provider(caller, id.uuid()).await?
            } else {
                existing_provider.clone()
            };
            let caps: Vec<String> =
                serde_json::from_value(existing.capabilities.clone()).unwrap_or_default();
            let metadata = crate::domains::models::catalog::assign(
                &target_provider.provider_type,
                req.model_id.as_deref().unwrap_or(&existing.model_id),
                req.capabilities.as_ref().unwrap_or(&caps),
                req.profile_key.as_deref(),
                req.service,
                existing.provider_metadata.clone(),
            )?;
            self.validate_model_service(
                &target_provider.provider_type,
                crate::domains::models::catalog::service(Some(&metadata)),
            )?;
            if crate::domains::models::catalog::service(Some(&metadata))
                != crate::domains::models::catalog::service(existing.provider_metadata.as_ref())
            {
                anyhow::bail!(crate::errors::BadRequestError::new(
                    "Cannot change the service of an existing model; add a separate model"
                ));
            }
            Some(metadata)
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
            provider_metadata: metadata,
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
        // Read the model independently of its provider so a missing or foreign
        // provider cannot hide an org-owned row from the policy precondition.
        let model = match self.db.get_model_for_mutation(caller.org_id, id).await? {
            Some(row) => row,
            None => return Ok(false),
        };
        let provider = self
            .get_visible_provider(caller, model.provider_id.uuid())
            .await?;
        Self::require_unmanaged_provider(&provider)?;

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
        if let Some(ref row) = row {
            self.get_visible_provider(caller, row.provider_id.uuid())
                .await?;
        }
        Ok(row.as_ref().map(Self::row_to_model_with_provider))
    }

    /// Set the org default model
    pub async fn set_default(&self, org_id: i64, model_id: Uuid) -> Result<()> {
        let model = self
            .db
            .get_model_for_mutation(org_id, model_id)
            .await?
            .ok_or_else(|| ResourceNotFoundError::new("Model"))?;
        let provider = self.get_provider(org_id, model.provider_id.uuid()).await?;
        anyhow::ensure!(
            provider.provider_type != "chatgpt",
            "A personal ChatGPT model cannot be the organization default"
        );
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

    /// Elect a new default model from enabled models.
    ///
    /// Chat models only: an embedding model is enabled like any other row, and
    /// electing one leaves the org with a default that chat cannot use while
    /// still *resolving* — so `bootstrap_intelligence` sees a healthy default
    /// and never repairs it. Found by disabling the org default on a stack
    /// whose only other enabled model was `text-embedding-3-small`.
    async fn elect_new_default(&self, org_id: i64) -> Result<()> {
        let all_models = self.db.list_all_models(org_id).await?;
        let new_default = all_models.iter().find(|m| {
            m.provider_type != "chatgpt"
                && m.enabled
                && Self::row_is_chat_model(&m.capabilities, m.provider_metadata.as_ref())
        });

        let new_default_id = new_default.map(|m| m.id.uuid());
        self.db
            .upsert_organization_settings(org_id, new_default_id)
            .await?;
        self.invalidate_resolver_cache(org_id).await;
        Ok(())
    }

    // ============================================================
    // First-run intelligence bootstrap
    // ============================================================
    //
    // Decision: configuring a provider credential is the moment an org gains
    // intelligence, so it must also leave the org *usable*. Discovery creates
    // models disabled and never elects an org default, which left a freshly
    // onboarded org with a keyed provider, zero selectable models and a chat
    // that could not resolve a model. Bootstrapping lives here, at the domain
    // layer, so every entry path (onboarding, Settings, CLI, MCP) behaves the
    // same.
    //
    // It only ever adds: models are enabled solely when the org has no enabled
    // chat model at all, and the default is elected solely when none resolves.
    // An org that deliberately disabled everything is left alone.

    /// Make a freshly credentialed provider usable: enable its chat models when
    /// the org has none, and elect an org default model when none resolves.
    pub async fn bootstrap_intelligence(
        &self,
        org_id: i64,
        provider_id: Uuid,
    ) -> Result<IntelligenceBootstrap> {
        let provider = self.get_provider(org_id, provider_id).await?;
        let provider_type: DriverId = provider.provider_type.parse().unwrap_or(DriverId::OpenAI);
        let mut outcome = IntelligenceBootstrap::default();

        // A provider that cannot serve a request bootstraps nothing.
        if provider.provider_type == "chatgpt"
            || !provider.api_key_set
            || provider.status != "active"
        {
            return Ok(outcome);
        }

        let org_has_enabled_chat = self.db.list_all_models(org_id).await?.iter().any(|row| {
            row.enabled
                && Self::row_is_chat_model(&row.capabilities, row.provider_metadata.as_ref())
        });

        if !org_has_enabled_chat {
            let candidates = self
                .chat_candidates(org_id, &provider, &provider_type)
                .await?;
            // A shortlist, not the whole catalog: the picker stays readable, and
            // the strongest few are starred so the composer has something to
            // offer before anyone visits Settings.
            for (rank, candidate) in candidates.iter().take(BOOTSTRAP_ENABLE_LIMIT).enumerate() {
                let favorite = rank < BOOTSTRAP_FAVORITE_LIMIT && !candidate.is_favorite;
                if candidate.enabled && !favorite {
                    continue;
                }
                self.db
                    .update_model(
                        org_id,
                        candidate.id.uuid(),
                        UpdateModel {
                            enabled: Some(true),
                            is_favorite: favorite.then_some(true),
                            ..Default::default()
                        },
                    )
                    .await?;
                if !candidate.enabled {
                    outcome.enabled_models += 1;
                }
                if favorite {
                    outcome.favorited_models += 1;
                }
            }
        }

        // `get_default_model` fails closed on a disabled model or an inactive
        // provider, so "resolves to nothing" — not merely a NULL setting — is
        // the condition that makes chat unusable.
        if self.db.get_default_model(org_id).await?.is_none() {
            let pick = self
                .chat_candidates(org_id, &provider, &provider_type)
                .await?
                .into_iter()
                .find(|row| row.enabled);
            if let Some(pick) = pick {
                self.db
                    .upsert_organization_settings(org_id, Some(pick.id.uuid()))
                    .await?;
                outcome.default_model_id = Some(pick.id.uuid());
            }
        }

        if outcome.changed() {
            self.invalidate_resolver_cache(org_id).await;
            tracing::info!(
                org_id,
                provider_id = %provider_id,
                enabled_models = outcome.enabled_models,
                favorited_models = outcome.favorited_models,
                default_model_set = outcome.default_model_id.is_some(),
                "Bootstrapped org intelligence from provider credential"
            );
        }
        Ok(outcome)
    }

    /// This provider's chat models, best default first.
    ///
    /// Only catalog-known models that can call tools are candidates: a provider
    /// catalog also lists transcription, image and legacy ids that make a poor
    /// default, and an agent runtime needs tool calling. When the static catalog
    /// knows none of them — a self-hosted or aggregator endpoint — the single
    /// best-guess chat model still keeps the org usable.
    async fn chat_candidates(
        &self,
        org_id: i64,
        provider: &crate::storage::models::ProviderRow,
        provider_type: &DriverId,
    ) -> Result<Vec<ModelRow>> {
        let mut chat: Vec<ModelRow> = self
            .db
            .list_models_for_provider(org_id, provider.id.uuid())
            .await?
            .into_iter()
            .filter(|row| {
                Self::row_is_chat_model(&row.capabilities, row.provider_metadata.as_ref())
            })
            .collect();

        let mut known: Vec<(ModelRank, ModelRow)> = chat
            .iter()
            .filter_map(|row| {
                let profile = get_model_profile(provider_type, &row.model_id)?;
                profile
                    .tool_call
                    .then(|| (ModelRank::of(&profile), row.clone()))
            })
            .collect();

        if !known.is_empty() {
            known.sort_by(|(left_rank, left), (right_rank, right)| {
                left_rank
                    .cmp(right_rank)
                    .then_with(|| left.model_id.cmp(&right.model_id))
            });
            return Ok(known.into_iter().map(|(_, row)| row).collect());
        }

        chat.sort_by(|left, right| left.model_id.cmp(&right.model_id));
        chat.truncate(1);
        Ok(chat)
    }

    pub(crate) fn validate_model_service(
        &self,
        driver: &str,
        service: everruns_contracts::ServiceKind,
    ) -> Result<()> {
        if let Some(resolver) = &self.provider_resolver
            && !resolver.driver_supports(driver, service)
        {
            anyhow::bail!(crate::errors::BadRequestError::new(
                "Provider does not support the selected model service"
            ));
        }
        Ok(())
    }

    /// Mirrors the UI rule: an embedding
    /// model is the one kind a chat cannot use.
    fn row_is_chat_model(
        capabilities: &sqlx::types::JsonValue,
        metadata: Option<&serde_json::Value>,
    ) -> bool {
        if crate::domains::models::catalog::service(metadata)
            != everruns_contracts::ServiceKind::Chat
        {
            return false;
        }
        let capabilities: Vec<String> =
            serde_json::from_value(capabilities.clone()).unwrap_or_default();
        !capabilities.iter().any(|capability| {
            matches!(
                capability.to_lowercase().as_str(),
                "embeddings" | "decisions" | "realtime" | "images" | "rerank"
            )
        })
    }
}

#[cfg(test)]
mod tests;
