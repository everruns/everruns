// Model Profile Types
//
// Configuration for model-specific capabilities like reasoning effort.
// These types are defined in core and re-exported by contracts.

use serde::{Deserialize, Serialize};
use utoipa::ToSchema;

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
