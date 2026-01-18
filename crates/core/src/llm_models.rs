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
#[derive(Debug, Clone, Serialize, Deserialize)]
#[cfg_attr(feature = "openapi", derive(ToSchema))]
#[serde(rename_all = "snake_case")]
pub enum LlmProviderType {
    /// OpenAI using Open Responses API (https://www.openresponses.org/)
    Openai,
    /// OpenAI using Chat Completions API (for backward compatibility)
    #[serde(rename = "openai_completions")]
    OpenaiCompletions,
    Anthropic,
    /// LLM simulator for testing
    #[serde(rename = "llmsim")]
    LlmSim,
}

impl std::fmt::Display for LlmProviderType {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            LlmProviderType::Openai => write!(f, "openai"),
            LlmProviderType::OpenaiCompletions => write!(f, "openai_completions"),
            LlmProviderType::Anthropic => write!(f, "anthropic"),
            LlmProviderType::LlmSim => write!(f, "llmsim"),
        }
    }
}

impl std::str::FromStr for LlmProviderType {
    type Err = String;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        match s {
            "openai" => Ok(LlmProviderType::Openai),
            "openai_completions" => Ok(LlmProviderType::OpenaiCompletions),
            "anthropic" => Ok(LlmProviderType::Anthropic),
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

/// LLM model status
#[derive(Debug, Clone, Serialize, Deserialize)]
#[cfg_attr(feature = "openapi", derive(ToSchema))]
#[serde(rename_all = "snake_case")]
pub enum LlmModelStatus {
    Active,
    Disabled,
}

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
    #[cfg_attr(feature = "openapi", schema(value_type = String, example = "prov_01933b5a00007000800000000000001"))]
    pub id: ProviderId,
    pub name: String,
    pub provider_type: LlmProviderType,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub base_url: Option<String>,
    /// Whether an API key is configured (key is never returned)
    pub api_key_set: bool,
    pub status: LlmProviderStatus,
    /// When models were last synced from provider API
    #[serde(skip_serializing_if = "Option::is_none")]
    pub last_synced_at: Option<DateTime<Utc>>,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
}

/// LLM Model entity
#[derive(Debug, Clone, Serialize, Deserialize)]
#[cfg_attr(feature = "openapi", derive(ToSchema))]
pub struct LlmModel {
    #[cfg_attr(feature = "openapi", schema(value_type = String, example = "mod_01933b5a00007000800000000000001"))]
    pub id: ModelId,
    #[cfg_attr(feature = "openapi", schema(value_type = String, example = "prov_01933b5a00007000800000000000001"))]
    pub provider_id: ProviderId,
    pub model_id: String,
    pub display_name: String,
    pub capabilities: Vec<String>,
    pub is_default: bool,
    pub is_favorite: bool,
    pub status: LlmModelStatus,
    /// How the model was added to the system
    pub source: LlmModelSource,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
}

/// LLM Model with provider info
#[derive(Debug, Clone, Serialize, Deserialize)]
#[cfg_attr(feature = "openapi", derive(ToSchema))]
pub struct LlmModelWithProvider {
    #[cfg_attr(feature = "openapi", schema(value_type = String, example = "mod_01933b5a00007000800000000000001"))]
    pub id: ModelId,
    #[cfg_attr(feature = "openapi", schema(value_type = String, example = "prov_01933b5a00007000800000000000001"))]
    pub provider_id: ProviderId,
    pub model_id: String,
    pub display_name: String,
    pub capabilities: Vec<String>,
    pub is_default: bool,
    pub is_favorite: bool,
    pub status: LlmModelStatus,
    /// How the model was added to the system
    pub source: LlmModelSource,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
    pub provider_name: String,
    pub provider_type: LlmProviderType,
    /// Readonly profile with model capabilities (not persisted to database)
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
}

/// Token limits for the model
#[derive(Debug, Clone, Serialize, Deserialize)]
#[cfg_attr(feature = "openapi", derive(ToSchema))]
pub struct LlmModelLimits {
    /// Maximum context window size in tokens
    pub context: i32,
    /// Maximum output tokens
    pub output: i32,
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
/// Based on models.dev structure (https://models.dev/api.json)
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
            serde_json::to_string(&LlmProviderType::Anthropic).unwrap(),
            "\"anthropic\""
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
            serde_json::from_str::<LlmProviderType>("\"anthropic\"").unwrap(),
            LlmProviderType::Anthropic
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
            "anthropic".parse::<LlmProviderType>().unwrap(),
            LlmProviderType::Anthropic
        ));
        assert!(matches!(
            "llmsim".parse::<LlmProviderType>().unwrap(),
            LlmProviderType::LlmSim
        ));
    }

    #[test]
    fn test_llm_provider_type_display() {
        // Verify Display works correctly
        assert_eq!(LlmProviderType::Openai.to_string(), "openai");
        assert_eq!(
            LlmProviderType::OpenaiCompletions.to_string(),
            "openai_completions"
        );
        assert_eq!(LlmProviderType::Anthropic.to_string(), "anthropic");
        assert_eq!(LlmProviderType::LlmSim.to_string(), "llmsim");
    }
}
