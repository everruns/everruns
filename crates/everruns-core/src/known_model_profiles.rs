// Known Model Profiles
//
// Hardcoded model profiles for common OpenAI and Anthropic models.
// These provide default capabilities (like reasoning effort) without requiring
// database configuration.
//
// Model data is based on https://models.dev/api.json
// which aggregates model information from various providers.
//
// When a model has a profile in the database, it takes precedence over these defaults.

use crate::model_profile_types::{ModelProfile, ReasoningEffortConfig, ReasoningLevel};

/// Get a known model profile by model ID.
///
/// Returns `Some(ModelProfile)` if the model has known capabilities,
/// or `None` if the model is not in the known list.
///
/// Model matching is case-insensitive and supports partial matching for versioned models.
pub fn get_known_model_profile(model_id: &str) -> Option<ModelProfile> {
    let model_lower = model_id.to_lowercase();

    // OpenAI reasoning models (o1, o3, o4 series)
    // These support reasoning_effort parameter with low/medium/high values
    if is_openai_reasoning_model(&model_lower) {
        return Some(openai_reasoning_profile());
    }

    // Anthropic models - currently no special profile needed
    // Extended thinking uses a different mechanism (thinking blocks)
    // that doesn't require a model profile configuration

    None
}

/// Check if this is an OpenAI reasoning model (o1, o3, o4 series)
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
}

/// Create the standard OpenAI reasoning model profile
fn openai_reasoning_profile() -> ModelProfile {
    ModelProfile {
        reasoning_effort: Some(ReasoningEffortConfig {
            supported: true,
            levels: vec![
                ReasoningLevel {
                    value: "low".to_string(),
                    label: "Low".to_string(),
                    description: Some("Faster responses, minimal reasoning".to_string()),
                },
                ReasoningLevel {
                    value: "medium".to_string(),
                    label: "Medium".to_string(),
                    description: Some("Balanced reasoning depth (default)".to_string()),
                },
                ReasoningLevel {
                    value: "high".to_string(),
                    label: "High".to_string(),
                    description: Some("Maximum reasoning, detailed analysis".to_string()),
                },
            ],
            default: Some("medium".to_string()),
        }),
    }
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
}
