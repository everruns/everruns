// LLM Provider and Model entity types
//
// These types represent the database entities for LLM providers and models.
// Note: This is separate from llm.rs which defines the LlmProvider trait.

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};

use crate::typed_id::{ModelId, ProviderId};

#[cfg(feature = "openapi")]
use utoipa::ToSchema;

/// LLM provider type
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "openapi", derive(ToSchema))]
#[serde(rename_all = "snake_case")]
pub enum LlmProviderType {
    /// OpenAI using Open Responses API (<https://www.openresponses.org/>)
    Openai,
    /// Azure OpenAI using the Azure-hosted OpenAI v1 API
    #[serde(rename = "azure_openai")]
    AzureOpenai,
    /// OpenAI using Chat Completions API (for backward compatibility)
    #[serde(rename = "openai_completions")]
    OpenaiCompletions,
    Anthropic,
    /// Google Gemini API
    Gemini,
    /// LLM simulator for testing
    #[serde(rename = "llmsim")]
    LlmSim,
}

impl std::fmt::Display for LlmProviderType {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            LlmProviderType::Openai => write!(f, "openai"),
            LlmProviderType::AzureOpenai => write!(f, "azure_openai"),
            LlmProviderType::OpenaiCompletions => write!(f, "openai_completions"),
            LlmProviderType::Anthropic => write!(f, "anthropic"),
            LlmProviderType::Gemini => write!(f, "gemini"),
            LlmProviderType::LlmSim => write!(f, "llmsim"),
        }
    }
}

impl std::str::FromStr for LlmProviderType {
    type Err = String;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        match s {
            "openai" => Ok(LlmProviderType::Openai),
            "azure_openai" => Ok(LlmProviderType::AzureOpenai),
            "openai_completions" => Ok(LlmProviderType::OpenaiCompletions),
            "anthropic" => Ok(LlmProviderType::Anthropic),
            "gemini" => Ok(LlmProviderType::Gemini),
            "llmsim" => Ok(LlmProviderType::LlmSim),
            _ => Err(format!("Unknown provider type: {}", s)),
        }
    }
}

/// LLM provider status
#[derive(Debug, Clone, Serialize, Deserialize)]
#[cfg_attr(feature = "openapi", derive(ToSchema))]
#[serde(rename_all = "snake_case")]
pub enum LlmProviderStatus {
    Active,
    Disabled,
}

// LLM model "healthy" status is not persisted on the model row. It is
// derived at read time from the joined provider's state and exposed as a
// boolean on `LlmModelWithProvider`. The per-row `enabled` flag is the only
// persisted user-facing toggle, and it controls visibility in UI model
// pickers.

/// How the model was added to the system
#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq, Eq)]
#[cfg_attr(feature = "openapi", derive(ToSchema))]
#[serde(rename_all = "snake_case")]
pub enum LlmModelSource {
    /// User-created via API or UI
    #[default]
    Manual,
    /// Automatically discovered from provider's list_models API
    Discovered,
    /// From hardcoded seed data
    Predefined,
}

impl std::fmt::Display for LlmModelSource {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            LlmModelSource::Manual => write!(f, "manual"),
            LlmModelSource::Discovered => write!(f, "discovered"),
            LlmModelSource::Predefined => write!(f, "predefined"),
        }
    }
}

impl std::str::FromStr for LlmModelSource {
    type Err = String;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        match s {
            "manual" => Ok(LlmModelSource::Manual),
            "discovered" => Ok(LlmModelSource::Discovered),
            "predefined" => Ok(LlmModelSource::Predefined),
            _ => Err(format!("Unknown model source: {}", s)),
        }
    }
}

/// LLM Provider entity (API keys never exposed)
/// Note: This is the entity struct, separate from the LlmProvider trait in llm.rs
#[derive(Debug, Clone, Serialize, Deserialize)]
#[cfg_attr(feature = "openapi", derive(ToSchema))]
pub struct LlmProvider {
    /// Prefixed public identifier. See [ID Schema](https://docs.everruns.com/advanced/id-schema/).
    #[cfg_attr(feature = "openapi", schema(value_type = String, example = "provider_01933b5a00007000800000000000001"))]
    pub id: ProviderId,
    /// Human-readable provider name. Safe to render in user-facing messages.
    pub name: String,
    /// Provider implementation type (OpenAI, Anthropic, Gemini, etc.).
    pub provider_type: LlmProviderType,
    /// Custom base URL for self-hosted / proxied providers. `None` means use the provider's default endpoint.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub base_url: Option<String>,
    /// Whether an API key is configured. The key itself is never returned.
    pub api_key_set: bool,
    /// Current lifecycle status of this provider.
    pub status: LlmProviderStatus,
    /// Timestamp of the most recent successful model sync from the provider's API (RFC 3339).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub last_synced_at: Option<DateTime<Utc>>,
    /// Timestamp when this provider was created (RFC 3339).
    pub created_at: DateTime<Utc>,
    /// Timestamp when this provider was last updated (RFC 3339).
    pub updated_at: DateTime<Utc>,
}

/// LLM Model entity
#[derive(Debug, Clone, Serialize, Deserialize)]
#[cfg_attr(feature = "openapi", derive(ToSchema))]
pub struct LlmModel {
    /// Prefixed public identifier. See [ID Schema](https://docs.everruns.com/advanced/id-schema/).
    #[cfg_attr(feature = "openapi", schema(value_type = String, example = "model_01933b5a00007000800000000000001"))]
    pub id: ModelId,
    /// Owning provider's prefixed public identifier.
    #[cfg_attr(feature = "openapi", schema(value_type = String, example = "provider_01933b5a00007000800000000000001"))]
    pub provider_id: ProviderId,
    /// Provider-side model identifier as sent on the wire (e.g. `gpt-4o`, `claude-sonnet-4`).
    pub model_id: String,
    /// Human-readable display name. Safe to render in user-facing messages.
    pub display_name: String,
    /// Capability tags supported by this model (e.g. `chat`, `tools`, `vision`).
    pub capabilities: Vec<String>,
    /// Whether this model is starred in the UI for quick access.
    pub is_favorite: bool,
    /// Whether this model is selectable. Controls UI visibility AND server-side resolution: `LlmResolverService` requires `enabled = true`, and org default-model validation rejects disabled models. Disabled models stay visible in raw list endpoints (so admins can re-enable them) but cannot be used in active sessions or as a session/agent default.
    pub enabled: bool,
    /// How this model entry was added (manually, discovered, or seeded as predefined).
    pub source: LlmModelSource,
    /// Timestamp when this model was created (RFC 3339).
    pub created_at: DateTime<Utc>,
    /// Timestamp when this model was last updated (RFC 3339).
    pub updated_at: DateTime<Utc>,
}

/// LLM Model with provider info
#[derive(Debug, Clone, Serialize, Deserialize)]
#[cfg_attr(feature = "openapi", derive(ToSchema))]
pub struct LlmModelWithProvider {
    /// Prefixed public identifier. See [ID Schema](https://docs.everruns.com/advanced/id-schema/).
    #[cfg_attr(feature = "openapi", schema(value_type = String, example = "model_01933b5a00007000800000000000001"))]
    pub id: ModelId,
    /// Owning provider's prefixed public identifier.
    #[cfg_attr(feature = "openapi", schema(value_type = String, example = "provider_01933b5a00007000800000000000001"))]
    pub provider_id: ProviderId,
    /// Provider-side model identifier as sent on the wire (e.g. `gpt-4o`).
    pub model_id: String,
    /// Human-readable display name.
    pub display_name: String,
    /// Capability tags supported by this model.
    pub capabilities: Vec<String>,
    /// Whether this model is starred in the UI for quick access.
    pub is_favorite: bool,
    /// Whether this model is selectable. Controls UI visibility AND server-side resolution: `LlmResolverService` requires `enabled = true`, and org default-model validation rejects disabled models.
    pub enabled: bool,
    /// How this model entry was added (manually, discovered, or seeded as predefined).
    pub source: LlmModelSource,
    /// Timestamp when this model was created (RFC 3339).
    pub created_at: DateTime<Utc>,
    /// Timestamp when this model was last updated (RFC 3339).
    pub updated_at: DateTime<Utc>,
    /// Joined provider display name.
    pub provider_name: String,
    /// Joined provider implementation type.
    pub provider_type: LlmProviderType,
    /// Derived: model is configured and ready for use. Currently means the
    /// joined provider is active and has an API key set; over time this may
    /// also incorporate live reachability checks. Not persisted.
    pub healthy: bool,
    /// Readonly profile with model capabilities (limits, pricing, modalities). Not persisted.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub profile: Option<LlmModelProfile>,
}

// ============================================
// LLM Model Profile types
// Based on models.dev structure
// ============================================

/// Cost information for the model (per million tokens)
#[derive(Debug, Clone, Serialize, Deserialize)]
#[cfg_attr(feature = "openapi", derive(ToSchema))]
pub struct LlmModelCost {
    /// Input cost per million tokens (USD)
    pub input: f64,
    /// Output cost per million tokens (USD)
    pub output: f64,
    /// Cached read cost per million tokens (USD), if supported
    #[serde(skip_serializing_if = "Option::is_none")]
    pub cache_read: Option<f64>,
    /// Tiered pricing that applies above certain context thresholds.
    /// When present, the base cost fields apply up to the tier threshold,
    /// and each tier's costs apply for tokens beyond that threshold.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub cost_tiers: Vec<CostTier>,
}

/// A pricing tier that activates above a context token threshold.
/// For example, OpenAI charges higher rates for prompts exceeding 200K tokens.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[cfg_attr(feature = "openapi", derive(ToSchema))]
pub struct CostTier {
    /// Context token threshold above which this tier applies
    pub above_tokens: i32,
    /// Input cost per million tokens (USD) for this tier
    pub input: f64,
    /// Output cost per million tokens (USD) for this tier
    pub output: f64,
    /// Cached read cost per million tokens (USD) for this tier, if supported
    #[serde(skip_serializing_if = "Option::is_none")]
    pub cache_read: Option<f64>,
}

/// Token limits for the model
#[derive(Debug, Clone, Serialize, Deserialize)]
#[cfg_attr(feature = "openapi", derive(ToSchema))]
pub struct LlmModelLimits {
    /// Maximum context window size in tokens
    pub context: i32,
    /// Maximum input tokens (if different from context - output)
    #[serde(skip_serializing_if = "Option::is_none")]
    pub input: Option<i32>,
    /// Maximum output tokens
    pub output: i32,
    /// Maximum images or PDF pages per request
    #[serde(skip_serializing_if = "Option::is_none", default)]
    pub max_media: Option<i32>,
}

/// Modality type (text, image, audio, video)
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[cfg_attr(feature = "openapi", derive(ToSchema))]
#[serde(rename_all = "snake_case")]
pub enum Modality {
    Text,
    Image,
    Audio,
    Video,
    Pdf,
}

/// Model modalities for input and output
#[derive(Debug, Clone, Serialize, Deserialize)]
#[cfg_attr(feature = "openapi", derive(ToSchema))]
pub struct LlmModelModalities {
    /// Supported input modalities
    pub input: Vec<Modality>,
    /// Supported output modalities
    pub output: Vec<Modality>,
}

/// Reasoning effort level for models that support it
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[cfg_attr(feature = "openapi", derive(ToSchema))]
#[serde(rename_all = "snake_case")]
pub enum ReasoningEffort {
    None,
    Minimal,
    Low,
    Medium,
    High,
    Xhigh,
}

/// Named reasoning effort value for UI display
#[derive(Debug, Clone, Serialize, Deserialize)]
#[cfg_attr(feature = "openapi", derive(ToSchema))]
pub struct ReasoningEffortValue {
    /// The API value (e.g., "low", "medium")
    pub value: ReasoningEffort,
    /// Display name (e.g., "Low", "Medium")
    pub name: String,
}

/// Reasoning effort configuration for a model
#[derive(Debug, Clone, Serialize, Deserialize)]
#[cfg_attr(feature = "openapi", derive(ToSchema))]
pub struct ReasoningEffortConfig {
    /// Available reasoning effort values for this model
    pub values: Vec<ReasoningEffortValue>,
    /// Default reasoning effort for this model
    pub default: ReasoningEffort,
}

/// LLM Model Profile describing model capabilities
/// Based on models.dev structure (<https://models.dev/api.json>)
///
/// NOTE: Currently only includes profiles for:
/// - OpenAI: gpt-4o, gpt-4o-mini, o1, o1-mini, o1-pro, o3-mini
/// - Anthropic: claude-3-5-sonnet, claude-3-5-haiku, claude-3-opus, claude-3-sonnet, claude-3-haiku, claude-sonnet-4, claude-opus-4
///
/// Additional model profiles can be added as needed.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[cfg_attr(feature = "openapi", derive(ToSchema))]
pub struct LlmModelProfile {
    /// Display name of the model
    pub name: String,
    /// Model family (e.g., "gpt-4o", "claude-3-5-sonnet")
    pub family: String,
    /// Short human-readable description of the model's strengths and intended use
    #[serde(skip_serializing_if = "Option::is_none")]
    pub description: Option<String>,
    /// Release date (YYYY-MM-DD format)
    #[serde(skip_serializing_if = "Option::is_none")]
    pub release_date: Option<String>,
    /// Last updated date (YYYY-MM-DD format)
    #[serde(skip_serializing_if = "Option::is_none")]
    pub last_updated: Option<String>,
    /// Whether the model supports file/image attachments
    pub attachment: bool,
    /// Whether the model has reasoning/chain-of-thought capabilities
    pub reasoning: bool,
    /// Whether temperature control is supported
    pub temperature: bool,
    /// Knowledge cutoff date (YYYY-MM-DD format)
    #[serde(skip_serializing_if = "Option::is_none")]
    pub knowledge: Option<String>,
    /// Whether the model supports tool/function calling
    pub tool_call: bool,
    /// Whether the model supports structured output (JSON mode)
    pub structured_output: bool,
    /// Whether the model has open weights
    pub open_weights: bool,
    /// Cost per million tokens
    #[serde(skip_serializing_if = "Option::is_none")]
    pub cost: Option<LlmModelCost>,
    /// Token limits
    #[serde(skip_serializing_if = "Option::is_none")]
    pub limits: Option<LlmModelLimits>,
    /// Supported modalities
    #[serde(skip_serializing_if = "Option::is_none")]
    pub modalities: Option<LlmModelModalities>,
    /// Reasoning effort configuration (for reasoning models)
    #[serde(skip_serializing_if = "Option::is_none")]
    pub reasoning_effort: Option<ReasoningEffortConfig>,
    /// Whether the model supports tool_search (deferred tool loading).
    /// When true, the driver can use namespaces and defer_loading to reduce
    /// token usage for large tool sets. Currently supported by GPT-5.4 and newer.
    #[serde(default)]
    pub tool_search: bool,
    /// Whether the model supports native execution phases ("commentary" / "final_answer").
    /// When true, the driver sends the `phase` field on assistant messages in the wire format.
    /// Currently supported by GPT-5.4 and newer via OpenAI Responses API.
    #[serde(default)]
    pub supports_phases: bool,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_llm_provider_type_serialization() {
        // Verify all provider types serialize correctly
        assert_eq!(
            serde_json::to_string(&LlmProviderType::Openai).unwrap(),
            "\"openai\""
        );
        assert_eq!(
            serde_json::to_string(&LlmProviderType::OpenaiCompletions).unwrap(),
            "\"openai_completions\""
        );
        assert_eq!(
            serde_json::to_string(&LlmProviderType::AzureOpenai).unwrap(),
            "\"azure_openai\""
        );
        assert_eq!(
            serde_json::to_string(&LlmProviderType::Anthropic).unwrap(),
            "\"anthropic\""
        );
        assert_eq!(
            serde_json::to_string(&LlmProviderType::Gemini).unwrap(),
            "\"gemini\""
        );
        assert_eq!(
            serde_json::to_string(&LlmProviderType::LlmSim).unwrap(),
            "\"llmsim\""
        );
    }

    #[test]
    fn test_llm_provider_type_deserialization() {
        // Verify all provider types deserialize correctly
        assert!(matches!(
            serde_json::from_str::<LlmProviderType>("\"openai\"").unwrap(),
            LlmProviderType::Openai
        ));
        assert!(matches!(
            serde_json::from_str::<LlmProviderType>("\"openai_completions\"").unwrap(),
            LlmProviderType::OpenaiCompletions
        ));
        assert!(matches!(
            serde_json::from_str::<LlmProviderType>("\"azure_openai\"").unwrap(),
            LlmProviderType::AzureOpenai
        ));
        assert!(matches!(
            serde_json::from_str::<LlmProviderType>("\"anthropic\"").unwrap(),
            LlmProviderType::Anthropic
        ));
        assert!(matches!(
            serde_json::from_str::<LlmProviderType>("\"gemini\"").unwrap(),
            LlmProviderType::Gemini
        ));
        assert!(matches!(
            serde_json::from_str::<LlmProviderType>("\"llmsim\"").unwrap(),
            LlmProviderType::LlmSim
        ));
    }

    #[test]
    fn test_llm_provider_type_from_str() {
        // Verify FromStr works correctly
        assert!(matches!(
            "openai".parse::<LlmProviderType>().unwrap(),
            LlmProviderType::Openai
        ));
        assert!(matches!(
            "openai_completions".parse::<LlmProviderType>().unwrap(),
            LlmProviderType::OpenaiCompletions
        ));
        assert!(matches!(
            "azure_openai".parse::<LlmProviderType>().unwrap(),
            LlmProviderType::AzureOpenai
        ));
        assert!(matches!(
            "anthropic".parse::<LlmProviderType>().unwrap(),
            LlmProviderType::Anthropic
        ));
        assert!(matches!(
            "gemini".parse::<LlmProviderType>().unwrap(),
            LlmProviderType::Gemini
        ));
        assert!(matches!(
            "llmsim".parse::<LlmProviderType>().unwrap(),
            LlmProviderType::LlmSim
        ));
    }

    #[test]
    fn test_llm_model_limits_input_omitted_when_none() {
        let limits = LlmModelLimits {
            context: 200_000,
            input: None,
            output: 64_000,
            max_media: None,
        };
        let json = serde_json::to_value(&limits).unwrap();
        assert!(!json.as_object().unwrap().contains_key("input"));
    }

    #[test]
    fn test_llm_model_limits_input_included_when_some() {
        let limits = LlmModelLimits {
            context: 200_000,
            input: Some(150_000),
            output: 64_000,
            max_media: None,
        };
        let json = serde_json::to_value(&limits).unwrap();
        assert_eq!(json["input"], 150_000);
    }

    #[test]
    fn test_llm_model_limits_deserialize_without_input() {
        let json = r#"{"context": 200000, "output": 64000}"#;
        let limits: LlmModelLimits = serde_json::from_str(json).unwrap();
        assert_eq!(limits.context, 200_000);
        assert!(limits.input.is_none());
        assert_eq!(limits.output, 64_000);
    }

    #[test]
    fn test_llm_provider_type_display() {
        // Verify Display works correctly
        assert_eq!(LlmProviderType::Openai.to_string(), "openai");
        assert_eq!(LlmProviderType::AzureOpenai.to_string(), "azure_openai");
        assert_eq!(
            LlmProviderType::OpenaiCompletions.to_string(),
            "openai_completions"
        );
        assert_eq!(LlmProviderType::Anthropic.to_string(), "anthropic");
        assert_eq!(LlmProviderType::Gemini.to_string(), "gemini");
        assert_eq!(LlmProviderType::LlmSim.to_string(), "llmsim");
    }
}
