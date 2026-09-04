// Model-profile types (knowledge/foundations/providers.md), based on the
// models.dev structure. Profile data lives in `crate::profiles`.

use serde::{Deserialize, Serialize};

#[cfg(feature = "openapi")]
use utoipa::ToSchema;

/// A typed service a provider driver can offer (see knowledge/foundations/providers.md).
///
/// Declared in code by each driver, never stored in the database. Only `Chat`
/// has a driver trait today; the set is additive and new kinds gain factories
/// on `DriverDescriptor` when their first consumer lands.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ServiceKind {
    /// Chat completion (`ChatDriver`).
    Chat,
    /// Text embeddings (planned: knowledge-base hybrid retrieval).
    Embeddings,
    /// Realtime voice sessions (server-side adapter using provider credentials).
    Realtime,
    /// Image generation.
    Images,
    /// Search-result reranking.
    Rerank,
}

impl std::fmt::Display for ServiceKind {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let s = match self {
            ServiceKind::Chat => "chat",
            ServiceKind::Embeddings => "embeddings",
            ServiceKind::Realtime => "realtime",
            ServiceKind::Images => "images",
            ServiceKind::Rerank => "rerank",
        };
        f.write_str(s)
    }
}

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
/// The registry of profiles lives in `crate::profiles`; retired models are
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
}
