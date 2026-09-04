// Model entity and model-profile types (knowledge/foundations/providers.md)
//
// A Model is a specific model via a specific provider (provider FK + wire
// model id). ModelProfile is the model's identity and metadata; profile data
// lives in `crate::model_profiles`.

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};

pub use crate::provider::{DriverId, ProviderStatus};
use crate::typed_id::{ModelId, ProviderId};

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

// ============================================
// LLM Model Profile types
// Based on models.dev structure
// ============================================

/// Cost information for the model (per million tokens)
#[derive(Debug, Clone, Serialize, Deserialize)]
#[cfg_attr(feature = "openapi", derive(ToSchema))]
pub struct ModelCost {
    /// Input cost per million tokens (USD)
    pub input: f64,
    /// Output cost per million tokens (USD)
    pub output: f64,
    /// Cached read cost per million tokens (USD), if supported
    #[serde(skip_serializing_if = "Option::is_none")]
    pub cache_read: Option<f64>,
    /// Tiered pricing that applies when prompt tokens exceed context thresholds.
    /// When present, the highest matching tier replaces the base rates for the
    /// whole request.
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
pub struct ModelLimits {
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
pub struct ModelModalities {
    /// Supported input modalities
    pub input: Vec<Modality>,
    /// Supported output modalities
    pub output: Vec<Modality>,
}

/// Reasoning effort level for models that support it
#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[cfg_attr(feature = "openapi", derive(ToSchema))]
#[serde(rename_all = "snake_case")]
pub enum ReasoningEffort {
    /// Explicitly no reasoning. Distinct from an unset effort: the caller asked
    /// for the model's non-reasoning behavior, so drivers omit the reasoning
    /// request fields entirely rather than sending a provider default.
    None,
    Minimal,
    Low,
    Medium,
    High,
    Xhigh,
    /// Highest tier, above `Xhigh`. First offered by GPT-6 Astra
    /// (`reasoning.effort` accepts `max` there in addition to `xhigh`).
    Max,
}

impl ReasoningEffort {
    /// Wire value. Matches the serde representation and what providers accept.
    pub fn as_str(&self) -> &'static str {
        match self {
            Self::None => "none",
            Self::Minimal => "minimal",
            Self::Low => "low",
            Self::Medium => "medium",
            Self::High => "high",
            Self::Xhigh => "xhigh",
            Self::Max => "max",
        }
    }

    /// Parse a wire value. Returns `None` for anything unrecognized so callers
    /// can decide whether to reject or ignore.
    pub fn parse(value: &str) -> Option<Self> {
        match value.to_ascii_lowercase().as_str() {
            "none" => Some(Self::None),
            "minimal" => Some(Self::Minimal),
            "low" => Some(Self::Low),
            "medium" => Some(Self::Medium),
            "high" => Some(Self::High),
            "xhigh" => Some(Self::Xhigh),
            "max" => Some(Self::Max),
            _ => None,
        }
    }

    /// Whether this effort asks the model to reason at all.
    pub fn requests_reasoning(&self) -> bool {
        !matches!(self, Self::None)
    }
}

impl std::fmt::Display for ReasoningEffort {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(self.as_str())
    }
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

/// Speed level for models that expose a latency/price service tier.
/// Wire values map 1:1 to the OpenAI `service_tier` request parameter:
/// `flex` (slower, cheaper), `default` (standard), `priority` (faster,
/// premium). `auto` is deliberately not offered — omitting the field
/// preserves the provider's default routing.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[cfg_attr(feature = "openapi", derive(ToSchema))]
#[serde(rename_all = "snake_case")]
pub enum Speed {
    Flex,
    Default,
    Priority,
}

/// Named speed value for UI display
#[derive(Debug, Clone, Serialize, Deserialize)]
#[cfg_attr(feature = "openapi", derive(ToSchema))]
pub struct SpeedValue {
    /// The API value (e.g., "flex", "priority")
    pub value: Speed,
    /// Display name (e.g., "Flex", "Fast")
    pub name: String,
}

/// Speed configuration for a model
#[derive(Debug, Clone, Serialize, Deserialize)]
#[cfg_attr(feature = "openapi", derive(ToSchema))]
pub struct SpeedConfig {
    /// Available speed values for this model
    pub values: Vec<SpeedValue>,
    /// Default speed for this model
    pub default: Speed,
}

/// Verbosity level for models that support output-length control.
/// Wire values map 1:1 to the OpenAI `verbosity` request parameter:
/// `low` (terse), `medium` (balanced, provider default), `high`
/// (comprehensive). Independent of `ReasoningEffort`, which tunes the
/// amount of reasoning rather than the length of the final answer.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[cfg_attr(feature = "openapi", derive(ToSchema))]
#[serde(rename_all = "snake_case")]
pub enum Verbosity {
    Low,
    Medium,
    High,
}

/// Named verbosity value for UI display
#[derive(Debug, Clone, Serialize, Deserialize)]
#[cfg_attr(feature = "openapi", derive(ToSchema))]
pub struct VerbosityValue {
    /// The API value (e.g., "low", "high")
    pub value: Verbosity,
    /// Display name (e.g., "Low", "High")
    pub name: String,
}

/// Verbosity configuration for a model
#[derive(Debug, Clone, Serialize, Deserialize)]
#[cfg_attr(feature = "openapi", derive(ToSchema))]
pub struct VerbosityConfig {
    /// Available verbosity values for this model
    pub values: Vec<VerbosityValue>,
    /// Default verbosity for this model
    pub default: Verbosity,
}

/// Vendor / brand that authored a model. Independent of the provider type
/// that serves it (the same model may be offered by several providers or
/// gateways). Primarily drives UI iconography.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "openapi", derive(ToSchema))]
#[serde(rename_all = "lowercase")]
pub enum ModelVendor {
    OpenAi,
    Anthropic,
    Google,
    Nvidia,
    Qwen,
    Microsoft,
    Meta,
    MiniMax,
    Moonshot,
    XAi,
    LlmSim,
}

impl ModelVendor {
    /// Stable lowercase slug, the first segment of a model-profile key
    /// (`"{vendor}/{canonical_id}"`, see knowledge/foundations/providers.md). Matches the
    /// serde `lowercase` representation.
    pub fn slug(&self) -> &'static str {
        match self {
            ModelVendor::OpenAi => "openai",
            ModelVendor::Anthropic => "anthropic",
            ModelVendor::Google => "google",
            ModelVendor::Nvidia => "nvidia",
            ModelVendor::Qwen => "qwen",
            ModelVendor::Microsoft => "microsoft",
            ModelVendor::Meta => "meta",
            ModelVendor::MiniMax => "minimax",
            ModelVendor::Moonshot => "moonshot",
            ModelVendor::XAi => "xai",
            ModelVendor::LlmSim => "llmsim",
        }
    }
}

/// LLM Model Profile describing model capabilities
/// Based on models.dev structure (<https://models.dev/api.json>)
///
/// The registry of profiles lives in `model_profiles.rs`; retired models are
/// dropped from it as vendors sunset them.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[cfg_attr(feature = "openapi", derive(ToSchema))]
pub struct ModelProfile {
    /// Display name of the model
    pub name: String,
    /// Model family (e.g., "gpt-5.6-sol", "claude-sonnet-5")
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
    pub cost: Option<ModelCost>,
    /// Token limits
    #[serde(skip_serializing_if = "Option::is_none")]
    pub limits: Option<ModelLimits>,
    /// Supported modalities
    #[serde(skip_serializing_if = "Option::is_none")]
    pub modalities: Option<ModelModalities>,
    /// Reasoning effort configuration (for reasoning models)
    #[serde(skip_serializing_if = "Option::is_none")]
    pub reasoning_effort: Option<ReasoningEffortConfig>,
    /// Speed (service tier) configuration, for models served with
    /// selectable latency/price tiers (OpenAI `service_tier`).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub speed: Option<SpeedConfig>,
    /// Verbosity configuration, for models that expose output-length
    /// control (OpenAI `verbosity`).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub verbosity: Option<VerbosityConfig>,
    /// Whether the model supports tool_search (deferred tool loading).
    /// When true, the driver can use namespaces and defer_loading to reduce
    /// token usage for large tool sets. Currently supported by GPT-5.4 and newer.
    #[serde(default)]
    pub tool_search: bool,
    /// Provider-advertised request parameters supported by this model.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub supported_parameters: Vec<String>,
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
    fn test_model_limits_input_omitted_when_none() {
        let limits = ModelLimits {
            context: 200_000,
            input: None,
            output: 64_000,
            max_media: None,
        };
        let json = serde_json::to_value(&limits).unwrap();
        assert!(!json.as_object().unwrap().contains_key("input"));
    }

    #[test]
    fn test_model_limits_input_included_when_some() {
        let limits = ModelLimits {
            context: 200_000,
            input: Some(150_000),
            output: 64_000,
            max_media: None,
        };
        let json = serde_json::to_value(&limits).unwrap();
        assert_eq!(json["input"], 150_000);
    }

    #[test]
    fn test_model_limits_deserialize_without_input() {
        let json = r#"{"context": 200000, "output": 64000}"#;
        let limits: ModelLimits = serde_json::from_str(json).unwrap();
        assert_eq!(limits.context, 200_000);
        assert!(limits.input.is_none());
        assert_eq!(limits.output, 64_000);
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
