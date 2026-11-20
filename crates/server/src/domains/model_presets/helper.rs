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

use super::service::job_model;
use super::types::*;
use crate::domains::messages::types::MessageRole;
use crate::storage::{ModelWithProviderRow, SessionRow, StorageBackend};
use anyhow::Result;
use everruns_contracts::typed_id::{ModelId, SessionId};
use everruns_core::helper_models::{HelperModelTarget, SpawnHelperModel};

/// Recent message events read to find the parent's latest per-message model.
const PARENT_MESSAGE_WINDOW: i32 = 200;

fn parse_job(job: &str) -> Option<ModelJob> {
    serde_json::from_value(serde_json::Value::String(job.to_string())).ok()
}

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

/// Resolve a spawn's helper model to a model of the organization.
pub async fn resolve(
    db: &StorageBackend,
    org_id: i64,
    parent: &SessionRow,
    helper: &SpawnHelperModel,
) -> Result<Option<ModelId>> {
    let agent = match parent.agent_id {
        Some(agent_id) => db.get_agent(org_id, agent_id).await?,
        None => None,
    };
    let agent_preset = agent.as_ref().and_then(|a| a.model_preset.clone());
    let agent_model = agent.as_ref().and_then(|a| a.default_model_id);
    match &helper.target {
        HelperModelTarget::Same => parent_explicit_model(db, parent).await,
        HelperModelTarget::Job { job } => {
            let Some(job) = parse_job(job) else {
                return Ok(None);
            };
            if let Some(preset) = agent_preset.as_deref()
                && let Some(id) = job_model(db, org_id, preset, job).await?
            {
                return Ok(Some(id));
            }
            org_job_default(db, org_id, job).await
        }
        HelperModelTarget::Preset { preset, job } => match parse_job(job) {
            Some(job) => job_model(db, org_id, preset, job).await,
            None => Ok(None),
        },
        HelperModelTarget::Model { model_id } => {
            let Ok(id) = model_id.parse::<ModelId>() else {
                return Ok(None);
            };
            let models = db.list_all_models(org_id).await?;
            Ok(models
                .iter()
                .find(|model| model.id == id && usable(model))
                .map(|model| model.id))
        }
        HelperModelTarget::OtherProvider { job } => {
            let Some(job) = parse_job(job) else {
                return Ok(None);
            };
            let models = db.list_all_models(org_id).await?;
            let parent_provider = parent_effective_model(db, org_id, parent, agent_model)
                .await?
                .and_then(|id| models.iter().find(|model| model.id == id))
                .map(|model| model.provider_type.clone());
            let presets =
                super::builtin::all(&crate::domains::common::default_driver_registry(), &models);
            Ok(presets
                .iter()
                .filter(|preset| preset.kind == ModelPresetKind::Provider)
                .filter(|preset| preset.provider_type != parent_provider)
                .find_map(|preset| preset.jobs.get(job).or(preset.jobs.get(ModelJob::Default))))
        }
    }
}

/// What a data plane writes into a child's definition: the provider key and
/// model name an agent package's `model` names.
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize, utoipa::ToSchema)]
pub struct HelperModelChoice {
    /// The organization model.
    #[schema(value_type = String)]
    pub model_id: ModelId,
    /// The provider key (its normalized name).
    pub provider: String,
    /// The provider's model name.
    pub model: String,
}

/// Internal (host call `helper_model`): the model a spawned subagent's
/// helper key resolves to, for a data plane building the child.
#[derive(Debug, serde::Deserialize, serde::Serialize, utoipa::ToSchema)]
pub struct WorkerResolveHelperModel {
    /// The parent session.
    pub session_id: String,
    /// The helper model the spawn asked for.
    #[schema(value_type = Object)]
    pub helper: SpawnHelperModel,
}

use crate::domains::common::*;

#[command(
    name = "worker_resolve_helper_model",
    category = "models",
    description = "Internal: resolve a subagent's helper model key to an organization model.",
    method = "POST",
    path = "/internal/sessions/{session_id}/helper-model",
    policy = crate::domains::sessions::SESSION_VIEW
)]
impl Command for WorkerResolveHelperModel {
    type Output = Option<HelperModelChoice>;

    async fn execute(self, ctx: &Ctx) -> Result<Self::Output, CommandError> {
        let session_id: SessionId = self
            .session_id
            .parse()
            .map_err(|_| CommandError::bad_request("invalid session_id"))?;
        let parent = ctx
            .db
            .get_session(ctx.org_id(), session_id)
            .await?
            .ok_or_else(|| CommandError::not_found("Session"))?;
        let Some(model_id) = resolve(&ctx.db, ctx.org_id(), &parent, &self.helper).await? else {
            return Ok(None);
        };
        let Some(model) = ctx
            .db
            .get_model_with_provider(ctx.org_id(), model_id.uuid())
            .await?
        else {
            return Ok(None);
        };
        Ok(Some(HelperModelChoice {
            model_id,
            provider: everruns_contracts::runtime_provider::ProviderKey::new(&model.provider_name)
                .as_str()
                .to_string(),
            model: model.model_id,
        }))
    }
}
