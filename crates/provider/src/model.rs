// Model entity and model-profile types (knowledge/foundations/providers.md)
//
// A Model is a specific model via a specific provider (provider FK + wire
// model id). ModelProfile is the model's identity and metadata; profile
// types and data live in the `everruns-model-profiles` crate and are
// re-exported below for source compatibility.

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};

pub use crate::provider::{DriverId, ProviderStatus};
use crate::typed_id::{ModelId, ProviderId};
pub use everruns_model_profiles::{
    CostTier, Modality, ModelCost, ModelLimits, ModelModalities, ModelProfile, ModelVendor,
    ReasoningEffort, ReasoningEffortConfig, ReasoningEffortValue, Speed, SpeedConfig, SpeedValue,
    Verbosity, VerbosityConfig, VerbosityValue,
};

#[cfg(feature = "openapi")]
use utoipa::ToSchema;

// LLM model "healthy" status is not persisted on the model row. It is
// derived at read time from the joined provider's state and exposed as a
// boolean on `ModelWithProvider`. The per-row `enabled` flag is the only
// persisted user-facing toggle, and it controls visibility in UI model
// pickers.

/// How the model was added to the system
#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq, Eq)]
#[cfg_attr(feature = "openapi", derive(ToSchema))]
#[cfg_attr(feature = "openapi", schema(example = "predefined"))]
#[serde(rename_all = "snake_case")]
pub enum ModelSource {
    /// User-created via API or UI
    #[default]
    Manual,
    /// Automatically discovered from provider's list_models API
    Discovered,
    /// From hardcoded seed data
    Predefined,
}

impl std::fmt::Display for ModelSource {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            ModelSource::Manual => write!(f, "manual"),
            ModelSource::Discovered => write!(f, "discovered"),
            ModelSource::Predefined => write!(f, "predefined"),
        }
    }
}

impl std::str::FromStr for ModelSource {
    type Err = String;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        match s {
            "manual" => Ok(ModelSource::Manual),
            "discovered" => Ok(ModelSource::Discovered),
            "predefined" => Ok(ModelSource::Predefined),
            _ => Err(format!("Unknown model source: {}", s)),
        }
    }
}

/// LLM Model entity
#[derive(Debug, Clone, Serialize, Deserialize)]
#[cfg_attr(feature = "openapi", derive(ToSchema))]
pub struct Model {
    /// Prefixed public identifier. See [ID Schema](https://docs.everruns.com/advanced/id-schema/).
    #[cfg_attr(feature = "openapi", schema(value_type = String, example = "model_01933b5a00007000800000000000001"))]
    pub id: ModelId,
    /// Owning provider's prefixed public identifier.
    #[cfg_attr(feature = "openapi", schema(value_type = String, example = "provider_01933b5a00007000800000000000001"))]
    pub provider_id: ProviderId,
    /// Provider-side model identifier as sent on the wire (e.g. `gpt-5.2`, `claude-sonnet-5`).
    pub model_id: String,
    /// Human-readable display name. Safe to render in user-facing messages.
    pub display_name: String,
    /// Capability tags supported by this model (e.g. `chat`, `tools`, `vision`).
    pub capabilities: Vec<String>,
    /// Whether this model is starred in the UI for quick access.
    pub is_favorite: bool,
    /// Whether this model is selectable. Controls UI visibility AND server-side resolution: `ProviderResolverService` requires `enabled = true`, and org default-model validation rejects disabled models. Disabled models stay visible in raw list endpoints (so admins can re-enable them) but cannot be used in active sessions or as a session/agent default.
    pub enabled: bool,
    /// How this model entry was added (manually, discovered, or seeded as predefined).
    pub source: ModelSource,
    /// Timestamp when this model was created (RFC 3339).
    pub created_at: DateTime<Utc>,
    /// Timestamp when this model was last updated (RFC 3339).
    pub updated_at: DateTime<Utc>,
}

/// LLM Model with provider info
#[derive(Debug, Clone, Serialize, Deserialize)]
#[cfg_attr(feature = "openapi", derive(ToSchema))]
pub struct ModelWithProvider {
    /// Prefixed public identifier. See [ID Schema](https://docs.everruns.com/advanced/id-schema/).
    #[cfg_attr(feature = "openapi", schema(value_type = String, example = "model_01933b5a00007000800000000000001"))]
    pub id: ModelId,
    /// Owning provider's prefixed public identifier.
    #[cfg_attr(feature = "openapi", schema(value_type = String, example = "provider_01933b5a00007000800000000000001"))]
    pub provider_id: ProviderId,
    /// Provider-side model identifier as sent on the wire (e.g. `gpt-5.2`).
    #[cfg_attr(feature = "openapi", schema(example = "claude-sonnet-4-5"))]
    pub model_id: String,
    /// Human-readable display name.
    #[cfg_attr(feature = "openapi", schema(example = "Claude Sonnet 4.5"))]
    pub display_name: String,
    /// Capability tags supported by this model.
    #[cfg_attr(feature = "openapi", schema(example = json!(["text", "tools", "vision", "thinking"])))]
    pub capabilities: Vec<String>,
    /// Whether this model is starred in the UI for quick access.
    #[cfg_attr(feature = "openapi", schema(example = true))]
    pub is_favorite: bool,
    /// Whether this model is selectable. Controls UI visibility AND server-side resolution: `ProviderResolverService` requires `enabled = true`, and org default-model validation rejects disabled models.
    #[cfg_attr(feature = "openapi", schema(example = true))]
    pub enabled: bool,
    /// How this model entry was added (manually, discovered, or seeded as predefined).
    #[cfg_attr(feature = "openapi", schema(example = "predefined"))]
    pub source: ModelSource,
    /// Timestamp when this model was created (RFC 3339).
    #[cfg_attr(feature = "openapi", schema(example = "2026-01-04T11:23:00Z"))]
    pub created_at: DateTime<Utc>,
    /// Timestamp when this model was last updated (RFC 3339).
    #[cfg_attr(feature = "openapi", schema(example = "2026-05-27T15:24:00Z"))]
    pub updated_at: DateTime<Utc>,
    /// Joined provider display name.
    #[cfg_attr(feature = "openapi", schema(example = "Anthropic"))]
    pub provider_name: String,
    /// Joined provider implementation type.
    #[cfg_attr(feature = "openapi", schema(example = "anthropic"))]
    pub provider_type: DriverId,
    /// Derived: model is configured and ready for use. Currently means the
    /// joined provider is active and has an API key set; over time this may
    /// also incorporate live reachability checks. Not persisted.
    #[cfg_attr(feature = "openapi", schema(example = true))]
    pub healthy: bool,
    /// Readonly profile with model capabilities (limits, pricing, modalities). Not persisted.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub profile: Option<ModelProfile>,
    /// Vendor/brand of the model, derived from the model registry. Drives UI
    /// branding (icons). `None` when the model id is not in the registry. Not persisted.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub model_vendor: Option<ModelVendor>,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_provider_type_serialization() {
        // Verify all provider types serialize correctly
        assert_eq!(
            serde_json::to_string(&DriverId::OpenAI).unwrap(),
            "\"openai\""
        );
        assert_eq!(
            serde_json::to_string(&DriverId::OpenRouter).unwrap(),
            "\"openrouter\""
        );
        assert_eq!(
            serde_json::to_string(&DriverId::OpenAICompletions).unwrap(),
            "\"openai_completions\""
        );
        assert_eq!(
            serde_json::to_string(&DriverId::AzureOpenAI).unwrap(),
            "\"azure_openai\""
        );
        assert_eq!(
            serde_json::to_string(&DriverId::Anthropic).unwrap(),
            "\"anthropic\""
        );
        assert_eq!(
            serde_json::to_string(&DriverId::Gemini).unwrap(),
            "\"gemini\""
        );
        assert_eq!(
            serde_json::to_string(&DriverId::LlmSim).unwrap(),
            "\"llmsim\""
        );
        assert_eq!(serde_json::to_string(&DriverId::Meta).unwrap(), "\"meta\"");
    }

    #[test]
    fn test_provider_type_deserialization() {
        // Verify all provider types deserialize correctly
        assert_eq!(
            serde_json::from_str::<DriverId>("\"openai\"").unwrap(),
            DriverId::OpenAI
        );
        assert_eq!(
            serde_json::from_str::<DriverId>("\"openrouter\"").unwrap(),
            DriverId::OpenRouter
        );
        assert_eq!(
            serde_json::from_str::<DriverId>("\"openai_completions\"").unwrap(),
            DriverId::OpenAICompletions
        );
        assert_eq!(
            serde_json::from_str::<DriverId>("\"azure_openai\"").unwrap(),
            DriverId::AzureOpenAI
        );
        assert_eq!(
            serde_json::from_str::<DriverId>("\"anthropic\"").unwrap(),
            DriverId::Anthropic
        );
        assert_eq!(
            serde_json::from_str::<DriverId>("\"gemini\"").unwrap(),
            DriverId::Gemini
        );
        assert_eq!(
            serde_json::from_str::<DriverId>("\"llmsim\"").unwrap(),
            DriverId::LlmSim
        );
        assert_eq!(
            serde_json::from_str::<DriverId>("\"meta\"").unwrap(),
            DriverId::Meta
        );
    }

    #[test]
    fn test_provider_type_from_str() {
        // Verify FromStr works correctly
        assert_eq!("openai".parse::<DriverId>().unwrap(), DriverId::OpenAI);
        assert_eq!(
            "openrouter".parse::<DriverId>().unwrap(),
            DriverId::OpenRouter
        );
        assert_eq!(
            "openai_completions".parse::<DriverId>().unwrap(),
            DriverId::OpenAICompletions
        );
        assert_eq!(
            "azure_openai".parse::<DriverId>().unwrap(),
            DriverId::AzureOpenAI
        );
        assert_eq!(
            "anthropic".parse::<DriverId>().unwrap(),
            DriverId::Anthropic
        );
        assert_eq!("gemini".parse::<DriverId>().unwrap(), DriverId::Gemini);
        assert_eq!("llmsim".parse::<DriverId>().unwrap(), DriverId::LlmSim);
        assert_eq!("meta".parse::<DriverId>().unwrap(), DriverId::Meta);
    }

    #[test]
    fn test_provider_type_display() {
        // Verify Display works correctly
        assert_eq!(DriverId::OpenAI.to_string(), "openai");
        assert_eq!(DriverId::OpenRouter.to_string(), "openrouter");
        assert_eq!(DriverId::AzureOpenAI.to_string(), "azure_openai");
        assert_eq!(
            DriverId::OpenAICompletions.to_string(),
            "openai_completions"
        );
        assert_eq!(DriverId::Anthropic.to_string(), "anthropic");
        assert_eq!(DriverId::Gemini.to_string(), "gemini");
        assert_eq!(DriverId::LlmSim.to_string(), "llmsim");
        assert_eq!(DriverId::Meta.to_string(), "meta");
    }
}
