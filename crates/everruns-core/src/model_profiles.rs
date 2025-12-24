// Model Profiles
//
// Configuration for model-specific capabilities like reasoning effort,
// plus hardcoded profiles for common OpenAI and Anthropic models.
//
// Model data is based on https://models.dev/api.json
// which aggregates model information from various providers.
//
// When a model has a profile in the database, it takes precedence over these defaults.

use serde::{Deserialize, Serialize};
use utoipa::ToSchema;

// ============================================================================
// Types
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
    /// Create a model profile with OpenAI reasoning model defaults (o1, o3, o4, gpt-5, etc.)
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

// ============================================================================
// Known Model Profiles
// ============================================================================

/// Get a known model profile by model ID.
///
/// Returns `Some(ModelProfile)` if the model has known capabilities,
/// or `None` if the model is not in the known list.
///
/// Model matching is case-insensitive and supports partial matching for versioned models.
pub fn get_known_model_profile(model_id: &str) -> Option<ModelProfile> {
    let model_lower = model_id.to_lowercase();

    // OpenAI reasoning models (o1, o3, o4 series, gpt-5 series)
    // These support reasoning_effort parameter with low/medium/high values
    if is_openai_reasoning_model(&model_lower) {
        return Some(ModelProfile::openai_reasoning());
    }

    // Anthropic models - currently no special profile needed
    // Extended thinking uses a different mechanism (thinking blocks)
    // that doesn't require a model profile configuration

    None
}

/// Check if this is an OpenAI reasoning model (o1, o3, o4, gpt-5 series)
fn is_openai_reasoning_model(model_lower: &str) -> bool {
    // o1 series
    model_lower.starts_with("o1")
        || model_lower == "o1-preview"
        || model_lower == "o1-mini"
        // o3 series
        || model_lower.starts_with("o3")
        || model_lower == "o3-mini"
        // o4 series (preview)
        || model_lower.starts_with("o4")
        || model_lower == "o4-mini"
        // GPT-5 series - all variants support reasoning
        // Based on https://models.dev:
        // - gpt-5: 400k context, 128k output, reasoning: yes, $1.25/$10.00 per 1M tokens
        // - gpt-5.1: 400k context, 128k output, reasoning: yes, $1.25/$10.00 per 1M tokens
        // - gpt-5.2: 400k context, 128k output, reasoning: yes, $1.75/$14.00 per 1M tokens
        || model_lower.starts_with("gpt-5")
}

/// Get a merged model profile, preferring database values over known defaults.
///
/// If `db_profile` is provided and has values set, those take precedence.
/// Otherwise, falls back to known model profiles.
pub fn get_effective_model_profile(
    model_id: &str,
    db_profile: Option<&ModelProfile>,
) -> ModelProfile {
    // If database has a profile with reasoning_effort configured, use it
    if let Some(profile) = db_profile {
        if profile.reasoning_effort.is_some() {
            return profile.clone();
        }
    }

    // Fall back to known profiles
    get_known_model_profile(model_id).unwrap_or_default()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_openai_reasoning_models() {
        // o1 series
        assert!(get_known_model_profile("o1").is_some());
        assert!(get_known_model_profile("o1-preview").is_some());
        assert!(get_known_model_profile("o1-mini").is_some());
        assert!(get_known_model_profile("O1-Mini").is_some()); // case insensitive

        // o3 series
        assert!(get_known_model_profile("o3").is_some());
        assert!(get_known_model_profile("o3-mini").is_some());

        // o4 series
        assert!(get_known_model_profile("o4-mini").is_some());

        // Non-reasoning models
        assert!(get_known_model_profile("gpt-4o").is_none());
        assert!(get_known_model_profile("gpt-4-turbo").is_none());
        assert!(get_known_model_profile("claude-3-opus").is_none());
    }

    #[test]
    fn test_gpt5_reasoning_models() {
        // GPT-5 base
        assert!(get_known_model_profile("gpt-5").is_some());
        assert!(get_known_model_profile("GPT-5").is_some()); // case insensitive

        // GPT-5.1
        assert!(get_known_model_profile("gpt-5.1").is_some());
        assert!(get_known_model_profile("GPT-5.1").is_some());

        // GPT-5.2
        assert!(get_known_model_profile("gpt-5.2").is_some());
        assert!(get_known_model_profile("GPT-5.2").is_some());

        // Verify they have reasoning support
        let profile = get_known_model_profile("gpt-5").unwrap();
        assert!(profile.supports_reasoning_effort());

        let profile = get_known_model_profile("gpt-5.1").unwrap();
        assert!(profile.supports_reasoning_effort());

        let profile = get_known_model_profile("gpt-5.2").unwrap();
        assert!(profile.supports_reasoning_effort());
    }

    #[test]
    fn test_reasoning_profile_structure() {
        let profile = get_known_model_profile("o1").unwrap();
        let reasoning = profile.reasoning_effort.unwrap();

        assert!(reasoning.supported);
        assert_eq!(reasoning.levels.len(), 3);
        assert_eq!(reasoning.default, Some("medium".to_string()));

        // Check levels
        let values: Vec<&str> = reasoning.levels.iter().map(|l| l.value.as_str()).collect();
        assert_eq!(values, vec!["low", "medium", "high"]);
    }

    #[test]
    fn test_effective_profile_prefers_db() {
        let db_profile = ModelProfile {
            reasoning_effort: Some(ReasoningEffortConfig {
                supported: true,
                levels: vec![ReasoningLevel {
                    value: "custom".to_string(),
                    label: "Custom".to_string(),
                    description: None,
                }],
                default: Some("custom".to_string()),
            }),
        };

        let effective = get_effective_model_profile("o1", Some(&db_profile));
        assert_eq!(
            effective.reasoning_effort.unwrap().levels[0].value,
            "custom"
        );
    }

    #[test]
    fn test_effective_profile_falls_back_to_known() {
        let empty_db_profile = ModelProfile::default();

        let effective = get_effective_model_profile("o1", Some(&empty_db_profile));
        assert!(effective.reasoning_effort.is_some());
        assert_eq!(effective.reasoning_effort.unwrap().levels.len(), 3);
    }

    #[test]
    fn test_model_profile_openai_reasoning_constructor() {
        let profile = ModelProfile::openai_reasoning();
        assert!(profile.supports_reasoning_effort());

        let reasoning = profile.reasoning_effort.unwrap();
        assert!(reasoning.supported);
        assert_eq!(reasoning.levels.len(), 3);
        assert_eq!(reasoning.default, Some("medium".to_string()));
    }
}
