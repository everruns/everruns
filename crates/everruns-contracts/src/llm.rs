// LLM Provider and Model public DTOs

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use utoipa::ToSchema;
use uuid::Uuid;

// Re-export model profile types from core
pub use everruns_core::model_profile_types::{ModelProfile, ReasoningEffortConfig, ReasoningLevel};

#[derive(Debug, Clone, Serialize, Deserialize, ToSchema)]
#[serde(rename_all = "snake_case")]
pub enum LlmProviderType {
    Openai,
    Anthropic,
    AzureOpenai,
}

impl std::fmt::Display for LlmProviderType {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            LlmProviderType::Openai => write!(f, "openai"),
            LlmProviderType::Anthropic => write!(f, "anthropic"),
            LlmProviderType::AzureOpenai => write!(f, "azure_openai"),
        }
    }
}

impl std::str::FromStr for LlmProviderType {
    type Err = String;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        match s {
            "openai" => Ok(LlmProviderType::Openai),
            "anthropic" => Ok(LlmProviderType::Anthropic),
            "azure_openai" => Ok(LlmProviderType::AzureOpenai),
            _ => Err(format!("Unknown provider type: {}", s)),
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, ToSchema)]
#[serde(rename_all = "snake_case")]
pub enum LlmProviderStatus {
    Active,
    Disabled,
}

#[derive(Debug, Clone, Serialize, Deserialize, ToSchema)]
#[serde(rename_all = "snake_case")]
pub enum LlmModelStatus {
    Active,
    Disabled,
}

/// LLM Provider (API keys never exposed)
#[derive(Debug, Clone, Serialize, Deserialize, ToSchema)]
pub struct LlmProvider {
    pub id: Uuid,
    pub name: String,
    pub provider_type: LlmProviderType,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub base_url: Option<String>,
    /// Whether an API key is configured (key is never returned)
    pub api_key_set: bool,
    pub is_default: bool,
    pub status: LlmProviderStatus,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
}

/// LLM Model
#[derive(Debug, Clone, Serialize, Deserialize, ToSchema)]
pub struct LlmModel {
    pub id: Uuid,
    pub provider_id: Uuid,
    pub model_id: String,
    pub display_name: String,
    pub capabilities: Vec<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub context_window: Option<i32>,
    /// Model-specific profile with capabilities like reasoning effort
    #[serde(default)]
    pub model_profile: ModelProfile,
    pub is_default: bool,
    pub status: LlmModelStatus,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
}

/// LLM Model with provider info
#[derive(Debug, Clone, Serialize, Deserialize, ToSchema)]
pub struct LlmModelWithProvider {
    pub id: Uuid,
    pub provider_id: Uuid,
    pub model_id: String,
    pub display_name: String,
    pub capabilities: Vec<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub context_window: Option<i32>,
    /// Model-specific profile with capabilities like reasoning effort
    #[serde(default)]
    pub model_profile: ModelProfile,
    pub is_default: bool,
    pub status: LlmModelStatus,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
    pub provider_name: String,
    pub provider_type: LlmProviderType,
}
