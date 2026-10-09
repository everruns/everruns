// LLM models domain types.
//
// Decision: request/response DTOs are defined here, not in the HTTP layer, so
// the domain never imports `api`. The `api` module re-exports them, keeping
// OpenAPI schema names and JSON shapes unchanged.

use crate::kernel_imports::contracts::model::ModelSource;
use serde::Deserialize;
use utoipa::{IntoParams, ToSchema};

/// Request to create a new LLM model for a provider
#[derive(Debug, Deserialize, ToSchema)]
pub struct CreateModelRequest {
    /// The model identifier used by the provider's API (e.g., "gpt-5.6-sol", "claude-opus-5").
    #[schema(example = "gpt-5.2")]
    pub model_id: String,
    /// Human-readable display name for the model.
    #[schema(example = "GPT-4o")]
    pub display_name: String,
    /// Selected service; omitted values infer it from the profile or capabilities.
    #[serde(default)]
    pub service: Option<everruns_contracts::ServiceKind>,
    /// Stable curated profile key; omitted values infer a provider-specific binding.
    #[serde(default)]
    pub profile_key: Option<String>,
    /// List of capabilities this model supports (e.g., "chat", "vision", "tools").
    #[serde(default)]
    #[schema(example = json!(["chat", "vision", "tools"]))]
    pub capabilities: Vec<String>,
    /// Whether this model should be enabled (visible in UI model pickers).
    #[serde(default)]
    #[schema(example = false)]
    pub enabled: bool,
    /// Whether this model should be marked as a favorite for quick access.
    #[serde(default)]
    #[schema(example = false)]
    pub is_favorite: bool,
}

/// Query parameters for filtering models list
#[derive(Debug, Default, Deserialize, IntoParams)]
pub struct ListModelsQuery {
    /// Filter by typed model service.
    pub service: Option<everruns_contracts::ServiceKind>,
    /// Filter by model source (manual, discovered, predefined)
    pub source: Option<ModelSource>,
    /// Include models that are stale (not seen in recent sync). Default: true
    #[serde(default = "default_true")]
    pub include_stale: bool,
    /// Only return favorite models. Default: false
    #[serde(default)]
    pub favorites_only: bool,
}

/// Request to update an LLM model. Only provided fields will be updated.
#[derive(Debug, Deserialize, ToSchema)]
pub struct UpdateModelRequest {
    /// Provider that owns this model.
    #[serde(skip_serializing_if = "Option::is_none")]
    #[schema(example = "provider_019df670b5af7db7a5685a4ad18a544a")]
    pub provider_id: Option<String>,
    /// The model identifier used by the provider's API.
    #[serde(skip_serializing_if = "Option::is_none")]
    #[schema(example = "gpt-5.4-mini")]
    pub model_id: Option<String>,
    /// Human-readable display name for the model.
    #[serde(skip_serializing_if = "Option::is_none")]
    #[schema(example = "GPT-4o Mini")]
    pub display_name: Option<String>,
    /// Change the selected service; it must match the assigned profile.
    pub service: Option<everruns_contracts::ServiceKind>,
    /// Explicitly reassign the stable profile; preference-only edits preserve it.
    pub profile_key: Option<String>,
    /// List of capabilities this model supports.
    #[serde(skip_serializing_if = "Option::is_none")]
    #[schema(example = json!(["chat", "tools"]))]
    pub capabilities: Option<Vec<String>>,
    /// Whether this model should be enabled (visible in UI model pickers).
    #[serde(skip_serializing_if = "Option::is_none")]
    #[schema(example = true)]
    pub enabled: Option<bool>,
    /// Whether this model should be marked as a favorite for quick access.
    #[serde(skip_serializing_if = "Option::is_none")]
    #[schema(example = true)]
    pub is_favorite: Option<bool>,
}

fn default_true() -> bool {
    true
}
