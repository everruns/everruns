// Helper model resolution: turns the helper key a spawn named into one of the
// organization's models for the child session
// (knowledge/foundations/model-presets.md, "Helper models").
//
// Decision: resolution happens on the control plane when the child session is
// created, not in the tool, so every plane resolves the same way and a target
// can only land on a usable model of the org's own catalog. `None` means the
// child takes the ordinary turn resolution (its agent's model or preset, then
// org defaults), which for `same` without an explicit parent choice is
// exactly what the parent runs on.

use super::types::*;
use crate::domains::messages::types::MessageRole;
use crate::storage::{ModelWithProviderRow, SessionRow, StorageBackend};
use anyhow::Result;
use everruns_contracts::runtime::helper_models::{
    CatalogModel, CatalogPreset, HelperCatalog, resolve_helper_model,
};
use everruns_contracts::typed_id::ModelId;
use everruns_core::helper_models::{HelperModelTarget, SpawnHelperModel};

/// Recent message events read to find the parent's latest per-message model.
const PARENT_MESSAGE_WINDOW: i32 = 200;

fn usable(model: &ModelWithProviderRow) -> bool {
    crate::domains::organizations::research_model::usable(model)
}

/// The model the person explicitly chose for the parent: the session model,
/// else the model on its latest user message. `None` when the parent runs on
/// its agent's or the org's default.
pub async fn parent_explicit_model(
    db: &StorageBackend,
    parent: &SessionRow,
) -> Result<Option<ModelId>> {
    if let Some(id) = parent.model_id {
        return Ok(Some(id));
    }
    let events = db
        .list_message_events_limited(parent.id, Some(PARENT_MESSAGE_WINDOW))
        .await?;
    for event in events.iter().rev() {
        let Ok(message) = crate::domains::messages::service::MessageService::event_to_message(
            parent.id.uuid(),
            &event.data,
            &event.event_type,
            event.sequence,
        ) else {
            continue;
        };
        if message.role == MessageRole::User {
            return Ok(message.controls.and_then(|controls| controls.model_id));
        }
    }
    Ok(None)
}

/// The organization default for a job (the Defaults tab).
async fn org_job_default(
    db: &StorageBackend,
    org_id: i64,
    job: ModelJob,
) -> Result<Option<ModelId>> {
    if job == ModelJob::Research {
        return crate::domains::organizations::research_model::org_research_model_id(db, org_id)
            .await;
    }
    let Some(settings) = db.get_organization_settings(org_id).await? else {
        return Ok(None);
    };
    Ok(match job {
        ModelJob::Default => settings.default_model_id,
        ModelJob::Chat => settings.chat_model_id,
        ModelJob::Coding => settings.coding_model_id.or(settings.default_model_id),
        ModelJob::Small => settings.small_model_id,
        ModelJob::Research | ModelJob::Decision => None,
    })
}

/// The model the parent most likely runs on, for picking another provider.
async fn parent_effective_model(
    db: &StorageBackend,
    org_id: i64,
    parent: &SessionRow,
    agent_model: Option<ModelId>,
) -> Result<Option<ModelId>> {
    if let Some(id) = parent_explicit_model(db, parent).await?.or(agent_model) {
        return Ok(Some(id));
    }
    org_job_default(db, org_id, ModelJob::Default).await
}

fn job_name(job: ModelJob) -> String {
    serde_json::to_value(job)
        .ok()
        .and_then(|value| value.as_str().map(str::to_owned))
        .unwrap_or_default()
}

/// The organization's helper model catalog: every model, every preset
/// (built-ins first, then custom) and the job defaults. A data plane gets
/// the same catalog pushed (`data_plane::org_settings`), so both planes
/// resolve a helper key with one function.
pub async fn catalog(db: &StorageBackend, org_id: i64) -> Result<HelperCatalog> {
    let models = db.list_all_models(org_id).await?;
    let mut presets = super::service::all_presets(db, org_id, &models).await?;
    let mut defaults = std::collections::BTreeMap::new();
    for job in ModelJob::ALL {
        if let Some(id) = org_job_default(db, org_id, job).await? {
            defaults.insert(job_name(job), id.to_string());
        }
    }
    Ok(HelperCatalog {
        models: models
            .iter()
            .map(|model| CatalogModel {
                id: model.id.to_string(),
                provider: everruns_contracts::runtime_provider::ProviderKey::new(
                    &model.provider_name,
                )
                .as_str()
                .to_string(),
                provider_type: model.provider_type.clone(),
                model: model.model_id.clone(),
                usable: usable(model),
            })
            .collect(),
        presets: presets
            .drain(..)
            .map(|preset| CatalogPreset {
                provider_type: (preset.kind == ModelPresetKind::Provider)
                    .then(|| preset.provider_type.clone().unwrap_or_default()),
                jobs: ModelJob::ALL
                    .into_iter()
                    .filter_map(|job| Some((job_name(job), preset.jobs.get(job)?.to_string())))
                    .collect(),
                id: preset.id,
            })
            .collect(),
        defaults,
    })
}

/// Resolve a spawn's helper model to a model of the organization.
pub async fn resolve(
    db: &StorageBackend,
    org_id: i64,
    parent: &SessionRow,
    helper: &SpawnHelperModel,
) -> Result<Option<ModelId>> {
    if helper.target == HelperModelTarget::Same {
        return parent_explicit_model(db, parent).await;
    }
    let agent = match parent.agent_id {
        Some(agent_id) => db.get_agent(org_id, agent_id).await?,
        None => None,
    };
    let agent_preset = agent.as_ref().and_then(|a| a.model_preset.clone());
    let agent_model = agent.as_ref().and_then(|a| a.default_model_id);
    let catalog = catalog(db, org_id).await?;
    let parent_provider = match helper.target {
        HelperModelTarget::OtherProvider { .. } => {
            parent_effective_model(db, org_id, parent, agent_model)
                .await?
                .and_then(|id| catalog.model(&id.to_string()))
                .map(|model| model.provider_type.clone())
        }
        _ => None,
    };
    Ok(resolve_helper_model(
        &catalog,
        &helper.target,
        agent_preset.as_deref(),
        parent_provider.as_deref(),
    )
    .and_then(|model| model.id.parse().ok()))
}
