// LLM Provider and Model public DTOs

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use utoipa::ToSchema;
use uuid::Uuid;

// ============================================================================
// Model Profile Types - Configuration for model-specific capabilities
// ============================================================================

/// Reasoning effort level configuration
#[derive(Debug, Clone, Serialize, Deserialize, ToSchema)]
pub struct ReasoningLevel {
    /// The value to pass to the API (e.g., "low", "medium", "high")
    pub value: String,
    /// Human-readable label for the UI
    pub label: String,
    /// Description of what this level does
    #[serde(skip_serializing_if = "Option::is_none")]
    pub description: Option<String>,
}

/// Reasoning effort configuration for a model
#[derive(Debug, Clone, Default, Serialize, Deserialize, ToSchema)]
pub struct ReasoningEffortConfig {
    /// Whether reasoning effort is supported by this model
    #[serde(default)]
    pub supported: bool,
    /// Available reasoning effort levels
    #[serde(default)]
    pub levels: Vec<ReasoningLevel>,
    /// Default reasoning effort level
    #[serde(skip_serializing_if = "Option::is_none")]
    pub default: Option<String>,
}

/// Model profile containing model-specific capabilities and settings
#[derive(Debug, Clone, Default, Serialize, Deserialize, ToSchema)]
pub struct ModelProfile {
    /// Reasoning effort configuration
    #[serde(skip_serializing_if = "Option::is_none")]
    pub reasoning_effort: Option<ReasoningEffortConfig>,
}

impl ModelProfile {
    /// Create a model profile with OpenAI reasoning model defaults (o1, o3, etc.)
    pub fn openai_reasoning() -> Self {
        Self {
            reasoning_effort: Some(ReasoningEffortConfig {
                supported: true,
                levels: vec![
                    ReasoningLevel {
                        value: "low".to_string(),
                        label: "Low".to_string(),
                        description: Some(
                            "Faster responses, minimal reasoning, fewer tokens".to_string(),
                        ),
                    },
                    ReasoningLevel {
                        value: "medium".to_string(),
                        label: "Medium".to_string(),
                        description: Some("Balanced depth and efficiency (default)".to_string()),
                    },
                    ReasoningLevel {
                        value: "high".to_string(),
                        label: "High".to_string(),
                        description: Some(
                            "Deeper reasoning, more detailed explanations".to_string(),
                        ),
                    },
                ],
                default: Some("medium".to_string()),
            }),
        }
    }

    /// Check if reasoning effort is supported
    pub fn supports_reasoning_effort(&self) -> bool {
        self.reasoning_effort
            .as_ref()
            .map(|r| r.supported)
            .unwrap_or(false)
    }
}

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
