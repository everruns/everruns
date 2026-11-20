// Rows the model presets repository reads and writes.

use chrono::{DateTime, Utc};
use everruns_contracts::typed_id::ModelId;
use uuid::Uuid;

/// A custom model preset (maps to `model_presets`). Built-in presets are
/// computed, never stored.
#[derive(Debug, Clone, sqlx::FromRow)]
pub struct ModelPresetRow {
    pub id: Uuid,
    pub org_id: i64,
    pub key: String,
    pub name: String,
    pub description: Option<String>,
    pub default_model_id: Option<ModelId>,
    pub chat_model_id: Option<ModelId>,
    pub coding_model_id: Option<ModelId>,
    pub research_model_id: Option<ModelId>,
    pub small_model_id: Option<ModelId>,
    pub decision_model_id: Option<ModelId>,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
}

/// The model for each job of a custom preset; `None` = organization default.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct ModelPresetJobsInput {
    pub default_model_id: Option<ModelId>,
    pub chat_model_id: Option<ModelId>,
    pub coding_model_id: Option<ModelId>,
    pub research_model_id: Option<ModelId>,
    pub small_model_id: Option<ModelId>,
    pub decision_model_id: Option<ModelId>,
}
