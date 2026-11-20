// Model presets domain types.
//
// Decision: a preset is a named set of models, one per job. Built-in presets
// (Automatic and one per connected provider) are computed from the org's
// models and the drivers' curated picks on every read and never stored, so a
// refreshed pick reaches every org at once. Custom presets are rows.

use chrono::{DateTime, Utc};
use everruns_contracts::typed_id::ModelId;
use serde::{Deserialize, Serialize};
use utoipa::ToSchema;

/// Id of the built-in Automatic preset.
pub const AUTOMATIC_PRESET_ID: &str = "automatic";
/// Prefix of the built-in provider presets (`provider:gemini`).
pub const PROVIDER_PRESET_PREFIX: &str = "provider:";

/// A job a preset assigns a model to.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize, ToSchema)]
#[serde(rename_all = "snake_case")]
pub enum ModelJob {
    /// Agents and sessions with no model of their own.
    Default,
    /// The built-in Everruns Chat.
    Chat,
    /// Coding-harness agents and `coding` helpers.
    Coding,
    /// Agents set to the research role and `deep` helpers.
    Research,
    /// Titles, summaries and `fast` helpers.
    Small,
    /// Quick gate checks and the Auto helper pick.
    Decision,
}

impl ModelJob {
    pub const ALL: [ModelJob; 6] = [
        Self::Default,
        Self::Chat,
        Self::Coding,
        Self::Research,
        Self::Small,
        Self::Decision,
    ];
}

/// Where a preset comes from.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, ToSchema)]
#[serde(rename_all = "snake_case")]
pub enum ModelPresetKind {
    /// Built in: the best usable model per job across connected providers.
    Automatic,
    /// Built in and read only: one provider's curated picks.
    Provider,
    /// Created in the organization.
    Custom,
}

/// The model for each job. A missing job uses the organization default for it.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize, ToSchema)]
pub struct ModelPresetJobs {
    /// Model for agents and sessions with no model of their own.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[schema(value_type = Option<String>)]
    pub default_model_id: Option<ModelId>,
    /// Model for the built-in Everruns Chat.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[schema(value_type = Option<String>)]
    pub chat_model_id: Option<ModelId>,
    /// Model for coding-harness agents and coding helpers.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[schema(value_type = Option<String>)]
    pub coding_model_id: Option<ModelId>,
    /// Model for research-role agents and deep helpers.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[schema(value_type = Option<String>)]
    pub research_model_id: Option<ModelId>,
    /// Model for titles, summaries and fast helpers.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[schema(value_type = Option<String>)]
    pub small_model_id: Option<ModelId>,
    /// Model for quick gate checks and the Auto helper pick.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[schema(value_type = Option<String>)]
    pub decision_model_id: Option<ModelId>,
}

impl ModelPresetJobs {
    pub fn get(&self, job: ModelJob) -> Option<ModelId> {
        match job {
            ModelJob::Default => self.default_model_id,
            ModelJob::Chat => self.chat_model_id,
            ModelJob::Coding => self.coding_model_id,
            ModelJob::Research => self.research_model_id,
            ModelJob::Small => self.small_model_id,
            ModelJob::Decision => self.decision_model_id,
        }
    }

    pub fn set(&mut self, job: ModelJob, model: Option<ModelId>) {
        let slot = match job {
            ModelJob::Default => &mut self.default_model_id,
            ModelJob::Chat => &mut self.chat_model_id,
            ModelJob::Coding => &mut self.coding_model_id,
            ModelJob::Research => &mut self.research_model_id,
            ModelJob::Small => &mut self.small_model_id,
            ModelJob::Decision => &mut self.decision_model_id,
        };
        *slot = model;
    }
}

impl From<&crate::storage::ModelPresetRow> for ModelPresetJobs {
    fn from(row: &crate::storage::ModelPresetRow) -> Self {
        Self {
            default_model_id: row.default_model_id,
            chat_model_id: row.chat_model_id,
            coding_model_id: row.coding_model_id,
            research_model_id: row.research_model_id,
            small_model_id: row.small_model_id,
            decision_model_id: row.decision_model_id,
        }
    }
}

impl From<&ModelPresetJobs> for crate::storage::ModelPresetJobsInput {
    fn from(jobs: &ModelPresetJobs) -> Self {
        Self {
            default_model_id: jobs.default_model_id,
            chat_model_id: jobs.chat_model_id,
            coding_model_id: jobs.coding_model_id,
            research_model_id: jobs.research_model_id,
            small_model_id: jobs.small_model_id,
            decision_model_id: jobs.decision_model_id,
        }
    }
}

/// A named set of models, one per job.
#[derive(Debug, Clone, Serialize, Deserialize, ToSchema)]
pub struct ModelPreset {
    /// Stable id: `automatic`, `provider:<driver>` or the custom preset's key.
    #[schema(example = "provider:gemini")]
    pub id: String,
    /// Display name.
    pub name: String,
    /// What the preset is for.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub description: Option<String>,
    /// Where the preset comes from: automatic, provider or custom.
    pub kind: ModelPresetKind,
    /// Driver of a provider preset (`gemini`).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub provider_type: Option<String>,
    /// Built-in presets cannot be changed; duplicate one to customize it.
    pub read_only: bool,
    /// The model for each job.
    #[serde(flatten)]
    pub jobs: ModelPresetJobs,
    /// Jobs of a provider preset whose curated model is not usable in the
    /// organization and that use the provider's next recommended model.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub fallback_jobs: Vec<ModelJob>,
    /// Live agents whose models come from this preset.
    pub agent_count: i64,
    /// When the custom preset was created; absent for built-ins.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub created_at: Option<DateTime<Utc>>,
    /// When the custom preset last changed; absent for built-ins.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub updated_at: Option<DateTime<Utc>>,
}

/// Create a custom preset, blank or as a copy of another preset.
#[derive(Debug, Clone, Default, Serialize, Deserialize, ToSchema)]
pub struct CreateModelPresetRequest {
    /// Display name, 1 to 100 characters.
    #[schema(example = "Cheap council")]
    pub name: String,
    /// What the preset is for.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub description: Option<String>,
    /// Preset to copy jobs from (`automatic`, `provider:gemini` or a custom
    /// id). Jobs given in this request override the copied ones.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[schema(example = "provider:gemini")]
    pub from: Option<String>,
    /// The model for each job.
    #[serde(flatten)]
    pub jobs: ModelPresetJobs,
}

/// Change a custom preset. Omit a field to keep it; null clears a job back to
/// the organization default.
#[derive(Debug, Clone, Default, Serialize, Deserialize, ToSchema)]
pub struct UpdateModelPresetRequest {
    /// Display name.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub name: Option<String>,
    /// What the preset is for.
    #[serde(
        default,
        skip_serializing_if = "Option::is_none",
        deserialize_with = "double_option"
    )]
    #[schema(value_type = Option<String>)]
    pub description: Option<Option<String>>,
    /// Model for agents and sessions with no model of their own.
    #[serde(
        default,
        skip_serializing_if = "Option::is_none",
        deserialize_with = "double_option"
    )]
    #[schema(value_type = Option<String>)]
    pub default_model_id: Option<Option<ModelId>>,
    /// Model for the built-in Everruns Chat.
    #[serde(
        default,
        skip_serializing_if = "Option::is_none",
        deserialize_with = "double_option"
    )]
    #[schema(value_type = Option<String>)]
    pub chat_model_id: Option<Option<ModelId>>,
    /// Model for coding-harness agents and coding helpers.
    #[serde(
        default,
        skip_serializing_if = "Option::is_none",
        deserialize_with = "double_option"
    )]
    #[schema(value_type = Option<String>)]
    pub coding_model_id: Option<Option<ModelId>>,
    /// Model for research-role agents and deep helpers.
    #[serde(
        default,
        skip_serializing_if = "Option::is_none",
        deserialize_with = "double_option"
    )]
    #[schema(value_type = Option<String>)]
    pub research_model_id: Option<Option<ModelId>>,
    /// Model for titles, summaries and fast helpers.
    #[serde(
        default,
        skip_serializing_if = "Option::is_none",
        deserialize_with = "double_option"
    )]
    #[schema(value_type = Option<String>)]
    pub small_model_id: Option<Option<ModelId>>,
    /// Model for quick gate checks and the Auto helper pick.
    #[serde(
        default,
        skip_serializing_if = "Option::is_none",
        deserialize_with = "double_option"
    )]
    #[schema(value_type = Option<String>)]
    pub decision_model_id: Option<Option<ModelId>>,
}

impl UpdateModelPresetRequest {
    /// Apply the job changes on top of `jobs`.
    pub fn apply_jobs(&self, jobs: &mut ModelPresetJobs) {
        for (job, change) in [
            (ModelJob::Default, self.default_model_id),
            (ModelJob::Chat, self.chat_model_id),
            (ModelJob::Coding, self.coding_model_id),
            (ModelJob::Research, self.research_model_id),
            (ModelJob::Small, self.small_model_id),
            (ModelJob::Decision, self.decision_model_id),
        ] {
            if let Some(model) = change {
                jobs.set(job, model);
            }
        }
    }
}

/// What copying a preset into the organization defaults changed.
#[derive(Debug, Clone, Serialize, ToSchema)]
pub struct ApplyModelPresetResult {
    /// Jobs whose organization default now uses the preset's model.
    pub applied_jobs: Vec<ModelJob>,
    /// Jobs left as they were: the preset has no model for them, or the
    /// model cannot serve that default (decision defaults need a calibrated
    /// decision model).
    pub skipped_jobs: Vec<ModelJob>,
    /// The organization after the copy.
    pub organization: crate::domains::organizations::types::OrganizationResponse,
}

fn double_option<'de, T, D>(de: D) -> Result<Option<Option<T>>, D::Error>
where
    T: Deserialize<'de>,
    D: serde::Deserializer<'de>,
{
    Option::<T>::deserialize(de).map(Some)
}
