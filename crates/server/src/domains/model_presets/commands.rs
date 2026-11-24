// Model preset commands: one declaration serves REST, MCP, the in-session
// platform capability and the `everruns model-presets` CLI.

use super::service;
use super::types::*;
use crate::domains::common::*;
use crate::domains::models::{LLM_MODEL_MANAGE, LLM_MODEL_VIEW};
use serde::Deserialize;
use utoipa::ToSchema;

const MAX_NAME: usize = 100;
const MAX_DESCRIPTION: usize = 500;

fn validate_text(name: &str, description: Option<&str>) -> Result<(), CommandError> {
    if name.trim().is_empty() || name.chars().count() > MAX_NAME {
        return Err(CommandError::bad_request(
            "Preset name must be 1 to 100 characters",
        ));
    }
    if description.is_some_and(|d| d.chars().count() > MAX_DESCRIPTION) {
        return Err(CommandError::bad_request(
            "Preset description must be at most 500 characters",
        ));
    }
    Ok(())
}

/// A preset job must name an enabled chat model of this org on a shared
/// provider: the same rule as the organization defaults.
async fn validate_jobs(ctx: &Ctx, jobs: &ModelPresetJobs) -> Result<(), CommandError> {
    for job in ModelJob::ALL {
        let Some(id) = jobs.get(job) else { continue };
        let model = ctx
            .db
            .get_model_with_provider(ctx.org_id(), id.uuid())
            .await?
            .ok_or_else(|| CommandError::bad_request(format!("Model {id} not found")))?;
        crate::domains::models::catalog::require_chat(model.provider_metadata.as_ref())
            .map_err(|_| CommandError::bad_request(format!("Model {id} is not a chat model")))?;
        if !model.enabled {
            return Err(CommandError::bad_request(format!(
                "Model {id} must be enabled"
            )));
        }
        if model.provider_type == "chatgpt" {
            return Err(CommandError::bad_request(
                "Personal providers cannot be used in presets",
            ));
        }
    }
    Ok(())
}

async fn require(ctx: &Ctx, id: &str) -> Result<ModelPreset, CommandError> {
    service::get(&ctx.db, ctx.org_id(), id)
        .await?
        .ok_or_else(|| CommandError::not_found("Model preset"))
}

fn require_custom(preset: &ModelPreset) -> Result<(), CommandError> {
    if preset.read_only {
        return Err(CommandError::bad_request(
            "Built-in presets are read only; duplicate one to customize it",
        ));
    }
    Ok(())
}

// ============================================
// List
// ============================================

#[derive(Debug, Default, Deserialize, ToSchema, serde::Serialize)]
pub struct ListModelPresets {}

#[command(
    name = "list_model_presets",
    category = "models",
    summary = "List model presets",
    description = "List model presets: built-in Automatic, one per connected provider, and custom presets, each with its model per job.",
    method = "GET",
    path = "/v1/platform/model-presets",
    policy = LLM_MODEL_VIEW,
    cli = CliRoute::new(&["model-presets"], "list").with_examples(&[CliExample::new("See which model each preset uses for coding, research and small work", "everruns model-presets list")]),
    http = list,
)]
impl Command for ListModelPresets {
    type Output = Vec<ModelPreset>;

    async fn execute(self, ctx: &Ctx) -> Result<Vec<ModelPreset>, CommandError> {
        Ok(service::list(&ctx.db, ctx.org_id()).await?)
    }
}

// ============================================
// Get
// ============================================

#[derive(Debug, Deserialize, ToSchema, serde::Serialize)]
pub struct GetModelPreset {
    /// Preset id: `automatic`, `provider:<driver>` or a custom preset id.
    pub id: String,
}

#[command(
    name = "get_model_preset",
    category = "models",
    summary = "Get a model preset",
    description = "Get one model preset with its model per job and the jobs that use a fallback model.",
    method = "GET",
    path = "/v1/platform/model-presets/{id}",
    policy = LLM_MODEL_VIEW,
    cli = CliRoute::new(&["model-presets"], "get").with_args(&[CliArg::new("id").at(1)]).with_examples(&[CliExample::new("Check which Gemini models the Gemini preset picks", "everruns model-presets get provider:gemini")]),
    positional = "id",
    http = plain,
    responses((status = 404, description = "Model preset not found")),
)]
impl Command for GetModelPreset {
    type Output = ModelPreset;

    async fn execute(self, ctx: &Ctx) -> Result<ModelPreset, CommandError> {
        require(ctx, &self.id).await
    }
}

// ============================================
// Create
// ============================================

#[derive(Debug, Deserialize, serde::Serialize)]
pub struct CreateModelPreset(pub CreateModelPresetRequest);

impl CommandSchema for CreateModelPreset {
    fn param_schema() -> serde_json::Value {
        delegated_param_schema::<CreateModelPresetRequest>()
    }
}

#[command(
    name = "create_model_preset",
    category = "models",
    summary = "Create a custom model preset",
    description = "Create a custom model preset, blank or copied from another preset with `from`.",
    method = "POST",
    path = "/v1/platform/model-presets",
    policy = LLM_MODEL_MANAGE,
    cli = CliRoute::new(&["model-presets"], "create").with_examples(&[CliExample::new("Start a custom preset from the Gemini picks", "everruns model-presets create --name 'Cheap council' --from provider:gemini --reason 'Cheaper helpers for review agents'")]),
    http = created,
    request_body(CreateModelPresetRequest),
)]
impl Command for CreateModelPreset {
    type Output = ModelPreset;

    async fn execute(self, ctx: &Ctx) -> Result<ModelPreset, CommandError> {
        let req = self.0;
        let name = req.name.trim().to_string();
        let description = req
            .description
            .map(|d| d.trim().to_string())
            .filter(|d| !d.is_empty());
        validate_text(&name, description.as_deref())?;
        let mut jobs = match &req.from {
            Some(from) => require(ctx, from).await?.jobs,
            None => ModelPresetJobs::default(),
        };
        for job in ModelJob::ALL {
            if let Some(id) = req.jobs.get(job) {
                jobs.set(job, Some(id));
            }
        }
        validate_jobs(ctx, &jobs).await?;
        let taken: Vec<String> = ctx
            .db
            .list_model_presets(ctx.org_id())
            .await?
            .into_iter()
            .map(|row| row.key)
            .collect();
        let key = service::key_from_name(&name, &taken);
        let row = ctx
            .db
            .create_model_preset(
                ctx.org_id(),
                &key,
                &name,
                description.as_deref(),
                &(&jobs).into(),
            )
            .await?;
        require(ctx, &row.key).await
    }
}

// ============================================
// Update
// ============================================

#[derive(Debug, Deserialize, ToSchema, serde::Serialize)]
pub struct UpdateModelPreset {
    /// Custom preset id.
    pub id: String,
    #[serde(flatten)]
    pub req: UpdateModelPresetRequest,
}

#[command(
    name = "update_model_preset",
    category = "models",
    summary = "Update a custom model preset",
    description = "Rename a custom model preset or change its model per job; null returns a job to the organization default.",
    method = "PATCH",
    path = "/v1/platform/model-presets/{id}",
    policy = LLM_MODEL_MANAGE,
    cli = CliRoute::new(&["model-presets"], "update").with_examples(&[CliExample::new("Use a stronger research model in a custom preset", "everruns model-presets update --id cheap-council --research-model-id model_01h9 --reason 'Research answers were too shallow'")]),
    http = plain,
    request_body(UpdateModelPresetRequest),
    responses((status = 404, description = "Model preset not found")),
)]
impl Command for UpdateModelPreset {
    type Output = ModelPreset;

    async fn execute(self, ctx: &Ctx) -> Result<ModelPreset, CommandError> {
        let current = require(ctx, &self.id).await?;
        require_custom(&current)?;
        let name = self
            .req
            .name
            .as_deref()
            .map(str::trim)
            .unwrap_or(&current.name)
            .to_string();
        let description = match &self.req.description {
            Some(value) => value
                .as_deref()
                .map(str::trim)
                .filter(|d| !d.is_empty())
                .map(str::to_string),
            None => current.description.clone(),
        };
        validate_text(&name, description.as_deref())?;
        let mut jobs = current.jobs.clone();
        self.req.apply_jobs(&mut jobs);
        validate_jobs(ctx, &jobs).await?;
        ctx.db
            .update_model_preset(
                ctx.org_id(),
                &self.id,
                &name,
                description.as_deref(),
                &(&jobs).into(),
            )
            .await?
            .ok_or_else(|| CommandError::not_found("Model preset"))?;
        require(ctx, &self.id).await
    }
}

// ============================================
// Delete
// ============================================

#[derive(Debug, Deserialize, ToSchema, serde::Serialize)]
pub struct DeleteModelPreset {
    /// Custom preset id.
    pub id: String,
}

#[command(
    name = "delete_model_preset",
    category = "models",
    description = "Delete a custom model preset. Refused while agents take their models from it.",
    method = "DELETE",
    path = "/v1/platform/model-presets/{id}",
    policy = LLM_MODEL_MANAGE,
    cli = CliRoute::new(&["model-presets"], "delete").with_examples(&[CliExample::new("Remove a preset no agent uses any more", "everruns model-presets delete --id cheap-council --reason 'Council experiment finished'")]),
    http = no_content,
    responses((status = 404, description = "Model preset not found")),
)]
impl Command for DeleteModelPreset {
    type Output = bool;

    async fn execute(self, ctx: &Ctx) -> Result<bool, CommandError> {
        let current = require(ctx, &self.id).await?;
        require_custom(&current)?;
        if current.agent_count > 0 {
            return Err(CommandError::conflict(format!(
                "{} agent(s) take their models from this preset; move them to another preset first",
                current.agent_count
            )));
        }
        if ctx.db.delete_model_preset(ctx.org_id(), &self.id).await? {
            Ok(true)
        } else {
            Err(CommandError::not_found("Model preset"))
        }
    }
}

// ============================================
// Apply to organization defaults
// ============================================

#[derive(Debug, Deserialize, ToSchema, serde::Serialize)]
pub struct ApplyModelPreset {
    /// Preset id to copy into the organization defaults.
    pub id: String,
}

#[command(
    name = "apply_model_preset",
    category = "models",
    description = "Copy a preset's models into the organization defaults. Jobs the preset leaves empty keep their current default.",
    method = "POST",
    path = "/v1/platform/model-presets/{id}/apply",
    policy = crate::domains::organizations::settings::ORGANIZATION_MANAGE,
    cli = CliRoute::new(&["model-presets"], "apply").with_args(&[CliArg::new("id").at(1)]).with_examples(&[CliExample::new("Make Gemini's picks the organization defaults", "everruns model-presets apply provider:gemini --reason 'Move the org to Gemini'")]),
    positional = "id",
    http = plain,
    responses((status = 404, description = "Model preset not found")),
)]
impl Command for ApplyModelPreset {
    type Output = ApplyModelPresetResult;

    async fn execute(self, ctx: &Ctx) -> Result<ApplyModelPresetResult, CommandError> {
        let preset = require(ctx, &self.id).await?;
        let set = |job| preset.jobs.get(job).map(Some);
        let mut applied = Vec::new();
        let mut skipped = Vec::new();
        for job in [
            ModelJob::Default,
            ModelJob::Chat,
            ModelJob::Coding,
            ModelJob::Research,
            ModelJob::Small,
        ] {
            if preset.jobs.get(job).is_some() {
                applied.push(job);
            } else {
                skipped.push(job);
            }
        }
        let request = crate::domains::organizations::types::UpdateOrganizationRequest {
            default_model_id: set(ModelJob::Default),
            chat_model_id: set(ModelJob::Chat),
            coding_model_id: set(ModelJob::Coding),
            research_model_id: set(ModelJob::Research),
            small_model_id: set(ModelJob::Small),
            ..Default::default()
        };
        let organization = crate::domains::organizations::settings::UpdateOrg {
            org: ctx.caller.org_public_id.clone(),
            request,
        }
        .execute(ctx)
        .await?;
        // The decision default is a calibrated decision-service model; a
        // preset's chat model for gate checks only applies when it qualifies.
        match preset.jobs.get(ModelJob::Decision) {
            Some(id) => {
                let decision = crate::domains::models::SetDefaultDecisionModel {
                    model_id: Some(id.to_string()),
                }
                .execute(ctx)
                .await;
                match decision {
                    Ok(_) => applied.push(ModelJob::Decision),
                    Err(_) => skipped.push(ModelJob::Decision),
                }
            }
            None => skipped.push(ModelJob::Decision),
        }
        Ok(ApplyModelPresetResult {
            applied_jobs: applied,
            skipped_jobs: skipped,
            organization,
        })
    }
}

/// Validate an agent's preset choice: `None` (organization defaults) or an
/// existing preset id.
pub async fn validate_agent_preset(
    ctx: &Ctx,
    preset: Option<&str>,
) -> Result<Option<String>, CommandError> {
    let Some(id) = preset.map(str::trim).filter(|id| !id.is_empty()) else {
        return Ok(None);
    };
    if service::get(&ctx.db, ctx.org_id(), id).await?.is_none() {
        return Err(CommandError::bad_request(format!(
            "Model preset {id} not found"
        )));
    }
    Ok(Some(id.to_string()))
}
