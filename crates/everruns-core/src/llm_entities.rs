// LLM Provider and Model entity types
//
// These types represent the database entities for LLM providers and models.
// Note: This is separate from llm.rs which defines the LlmProvider trait.

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use uuid::Uuid;

#[cfg(feature = "openapi")]
use utoipa::ToSchema;

/// LLM provider type
#[derive(Debug, Clone, Serialize, Deserialize)]
#[cfg_attr(feature = "openapi", derive(ToSchema))]
#[serde(rename_all = "snake_case")]
pub enum LlmProviderType {
    Openai,
    Anthropic,
    #[serde(rename = "azure_openai")]
    AzureOpenAI,
}

impl std::fmt::Display for LlmProviderType {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            LlmProviderType::Openai => write!(f, "openai"),
            LlmProviderType::Anthropic => write!(f, "anthropic"),
            LlmProviderType::AzureOpenAI => write!(f, "azure_openai"),
        }
    }
}

impl std::str::FromStr for LlmProviderType {
    type Err = String;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        match s {
            "openai" => Ok(LlmProviderType::Openai),
            "anthropic" => Ok(LlmProviderType::Anthropic),
            "azure_openai" => Ok(LlmProviderType::AzureOpenAI),
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

/// LLM Provider entity (API keys never exposed)
/// Note: This is the entity struct, separate from the LlmProvider trait in llm.rs
#[derive(Debug, Clone, Serialize, Deserialize)]
#[cfg_attr(feature = "openapi", derive(ToSchema))]
pub struct LlmProvider {
    pub id: Uuid,
    pub name: String,
    pub provider_type: LlmProviderType,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub base_url: Option<String>,
    /// Whether an API key is configured (key is never returned)
    pub api_key_set: bool,
    pub status: LlmProviderStatus,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
}

/// LLM Model entity
#[derive(Debug, Clone, Serialize, Deserialize)]
#[cfg_attr(feature = "openapi", derive(ToSchema))]
pub struct LlmModel {
    pub id: Uuid,
    pub provider_id: Uuid,
    pub model_id: String,
    pub display_name: String,
    pub capabilities: Vec<String>,
    pub is_default: bool,
    pub status: LlmModelStatus,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
}

/// LLM Model with provider info
#[derive(Debug, Clone, Serialize, Deserialize)]
#[cfg_attr(feature = "openapi", derive(ToSchema))]
pub struct LlmModelWithProvider {
    pub id: Uuid,
    pub provider_id: Uuid,
    pub model_id: String,
    pub display_name: String,
    pub capabilities: Vec<String>,
    pub is_default: bool,
    pub status: LlmModelStatus,
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
