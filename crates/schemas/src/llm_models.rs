// LLM Provider and Model entity types
//
// These types represent the database entities for LLM providers and models.

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
// ============================================

/// Cost information for the model (per million tokens)
#[derive(Debug, Clone, Serialize, Deserialize)]
#[cfg_attr(feature = "openapi", derive(ToSchema))]
pub struct LlmModelCost {
    pub input: f64,
    pub output: f64,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub cache_read: Option<f64>,
}

/// Token limits for the model
#[derive(Debug, Clone, Serialize, Deserialize)]
#[cfg_attr(feature = "openapi", derive(ToSchema))]
pub struct LlmModelLimits {
    pub context: i32,
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
    pub input: Vec<Modality>,
    pub output: Vec<Modality>,
}

/// Reasoning effort level
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

/// Named reasoning effort value
#[derive(Debug, Clone, Serialize, Deserialize)]
#[cfg_attr(feature = "openapi", derive(ToSchema))]
pub struct ReasoningEffortValue {
    pub value: ReasoningEffort,
    pub name: String,
}

/// Reasoning effort configuration
#[derive(Debug, Clone, Serialize, Deserialize)]
#[cfg_attr(feature = "openapi", derive(ToSchema))]
pub struct ReasoningEffortConfig {
    pub values: Vec<ReasoningEffortValue>,
    pub default: ReasoningEffort,
}

/// LLM Model Profile describing model capabilities
#[derive(Debug, Clone, Serialize, Deserialize)]
#[cfg_attr(feature = "openapi", derive(ToSchema))]
pub struct LlmModelProfile {
    pub name: String,
    pub family: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub release_date: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub last_updated: Option<String>,
    pub attachment: bool,
    pub reasoning: bool,
    pub temperature: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub knowledge: Option<String>,
    pub tool_call: bool,
    pub structured_output: bool,
    pub open_weights: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub cost: Option<LlmModelCost>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub limits: Option<LlmModelLimits>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub modalities: Option<LlmModelModalities>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub reasoning_effort: Option<ReasoningEffortConfig>,
}
