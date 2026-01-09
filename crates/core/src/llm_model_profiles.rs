// Hardcoded LLM Model Profiles
//
// This module provides model profiles based on models.dev structure.
// Profiles are matched by provider_type + model_id.
//
// NOTE: Currently only includes profiles for selected models.
// Additional model profiles can be added as needed by extending the match arms.
//
// Data source: https://models.dev/api.json

use crate::llm_models::{
    LlmModelCost, LlmModelLimits, LlmModelModalities, LlmModelProfile, LlmProviderType, Modality,
    ReasoningEffort, ReasoningEffortConfig, ReasoningEffortValue,
};

// Helper functions for creating reasoning effort configurations

fn effort(value: ReasoningEffort, name: &str) -> ReasoningEffortValue {
    ReasoningEffortValue {
        value,
        name: name.into(),
    }
}

/// Standard reasoning efforts for pre-gpt-5.1 models (o1, o1-mini, o3-mini)
/// Default: medium, supports: low, medium, high
fn reasoning_effort_standard() -> ReasoningEffortConfig {
    ReasoningEffortConfig {
        values: vec![
            effort(ReasoningEffort::Low, "Low"),
            effort(ReasoningEffort::Medium, "Medium"),
            effort(ReasoningEffort::High, "High"),
        ],
        default: ReasoningEffort::Medium,
    }
}

/// Reasoning effort for o1-pro (only high)
fn reasoning_effort_high_only() -> ReasoningEffortConfig {
    ReasoningEffortConfig {
        values: vec![effort(ReasoningEffort::High, "High")],
        default: ReasoningEffort::High,
    }
}

/// Reasoning effort for pre-gpt-5.1 models (gpt-5, gpt-5-mini, gpt-5-nano, gpt-5-codex)
/// Default: medium, supports: low, medium, high (no none)
fn reasoning_effort_gpt5_pre51() -> ReasoningEffortConfig {
    ReasoningEffortConfig {
        values: vec![
            effort(ReasoningEffort::Low, "Low"),
            effort(ReasoningEffort::Medium, "Medium"),
            effort(ReasoningEffort::High, "High"),
        ],
        default: ReasoningEffort::Medium,
    }
}

/// Reasoning effort for gpt-5.1 models
/// Default: none, supports: none, low, medium, high
fn reasoning_effort_gpt51() -> ReasoningEffortConfig {
    ReasoningEffortConfig {
        values: vec![
            effort(ReasoningEffort::None, "None"),
            effort(ReasoningEffort::Low, "Low"),
            effort(ReasoningEffort::Medium, "Medium"),
            effort(ReasoningEffort::High, "High"),
        ],
        default: ReasoningEffort::None,
    }
}

/// Reasoning effort for models after gpt-5.1-codex-max (gpt-5.2, gpt-5.2-pro, gpt-5.2-codex)
/// Default: none, supports: none, low, medium, high, xhigh
fn reasoning_effort_gpt52() -> ReasoningEffortConfig {
    ReasoningEffortConfig {
        values: vec![
            effort(ReasoningEffort::None, "None"),
            effort(ReasoningEffort::Low, "Low"),
            effort(ReasoningEffort::Medium, "Medium"),
            effort(ReasoningEffort::High, "High"),
            effort(ReasoningEffort::Xhigh, "Extra High"),
        ],
        default: ReasoningEffort::None,
    }
}

/// Reasoning effort for gpt-5.2-pro
/// Default: medium, supports: medium, high, xhigh
fn reasoning_effort_gpt52_pro() -> ReasoningEffortConfig {
    ReasoningEffortConfig {
        values: vec![
            effort(ReasoningEffort::Medium, "Medium"),
            effort(ReasoningEffort::High, "High"),
            effort(ReasoningEffort::Xhigh, "Extra High"),
        ],
        default: ReasoningEffort::Medium,
    }
}

/// Get a model profile by matching provider_type and model_id
/// Returns None if no matching profile is found
pub fn get_model_profile(
    provider_type: &LlmProviderType,
    model_id: &str,
) -> Option<LlmModelProfile> {
    match provider_type {
        LlmProviderType::Openai => get_openai_profile(model_id),
        LlmProviderType::Anthropic => get_anthropic_profile(model_id),
        LlmProviderType::AzureOpenAI => get_openai_profile(model_id), // Azure uses same model IDs
        LlmProviderType::LlmSim => None, // No profile for simulated LLM
    }
}

fn get_openai_profile(model_id: &str) -> Option<LlmModelProfile> {
    // Normalize model ID by extracting base name
    let base_id = normalize_model_id(model_id);

    match base_id {
        "gpt-4o" => Some(LlmModelProfile {
            name: "GPT-4o".into(),
            family: "gpt-4o".into(),
            release_date: Some("2024-05-13".into()),
            last_updated: Some("2024-11-20".into()),
            attachment: true,
            reasoning: false,
            temperature: true,
            knowledge: Some("2023-10-01".into()),
            tool_call: true,
            structured_output: true,
            open_weights: false,
            cost: Some(LlmModelCost {
                input: 2.50,
                output: 10.00,
                cache_read: Some(1.25),
            }),
            limits: Some(LlmModelLimits {
                context: 128_000,
                output: 16_384,
            }),
            modalities: Some(LlmModelModalities {
                input: vec![Modality::Text, Modality::Image, Modality::Audio],
                output: vec![Modality::Text, Modality::Audio],
            }),
            reasoning_effort: None,
        }),

        "gpt-4o-mini" => Some(LlmModelProfile {
            name: "GPT-4o mini".into(),
            family: "gpt-4o-mini".into(),
            release_date: Some("2024-07-18".into()),
            last_updated: Some("2024-07-18".into()),
            attachment: true,
            reasoning: false,
            temperature: true,
            knowledge: Some("2023-10-01".into()),
            tool_call: true,
            structured_output: true,
            open_weights: false,
            cost: Some(LlmModelCost {
                input: 0.15,
                output: 0.60,
                cache_read: Some(0.075),
            }),
            limits: Some(LlmModelLimits {
                context: 128_000,
                output: 16_384,
            }),
            modalities: Some(LlmModelModalities {
                input: vec![Modality::Text, Modality::Image],
                output: vec![Modality::Text],
            }),
            reasoning_effort: None,
        }),

        "o1" => Some(LlmModelProfile {
            name: "o1".into(),
            family: "o1".into(),
            release_date: Some("2024-12-17".into()),
            last_updated: Some("2024-12-17".into()),
            attachment: true,
            reasoning: true,
            temperature: true,
            knowledge: Some("2023-10-01".into()),
            tool_call: true,
            structured_output: true,
            open_weights: false,
            cost: Some(LlmModelCost {
                input: 15.00,
                output: 60.00,
                cache_read: Some(7.50),
            }),
            limits: Some(LlmModelLimits {
                context: 200_000,
                output: 100_000,
            }),
            modalities: Some(LlmModelModalities {
                input: vec![Modality::Text, Modality::Image],
                output: vec![Modality::Text],
            }),
            reasoning_effort: Some(reasoning_effort_standard()),
        }),

        "o1-mini" => Some(LlmModelProfile {
            name: "o1-mini".into(),
            family: "o1-mini".into(),
            release_date: Some("2024-09-12".into()),
            last_updated: Some("2024-09-12".into()),
            attachment: false,
            reasoning: true,
            temperature: true,
            knowledge: Some("2023-10-01".into()),
            tool_call: false,
            structured_output: false,
            open_weights: false,
            cost: Some(LlmModelCost {
                input: 3.00,
                output: 12.00,
                cache_read: Some(1.50),
            }),
            limits: Some(LlmModelLimits {
                context: 128_000,
                output: 65_536,
            }),
            modalities: Some(LlmModelModalities {
                input: vec![Modality::Text],
                output: vec![Modality::Text],
            }),
            reasoning_effort: Some(reasoning_effort_standard()),
        }),

        "o1-pro" => Some(LlmModelProfile {
            name: "o1-pro".into(),
            family: "o1-pro".into(),
            release_date: Some("2025-03-19".into()),
            last_updated: Some("2025-03-19".into()),
            attachment: true,
            reasoning: true,
            temperature: true,
            knowledge: Some("2023-10-01".into()),
            tool_call: true,
            structured_output: true,
            open_weights: false,
            cost: Some(LlmModelCost {
                input: 150.00,
                output: 600.00,
                cache_read: None,
            }),
            limits: Some(LlmModelLimits {
                context: 200_000,
                output: 100_000,
            }),
            modalities: Some(LlmModelModalities {
                input: vec![Modality::Text, Modality::Image],
                output: vec![Modality::Text],
            }),
            reasoning_effort: Some(reasoning_effort_high_only()),
        }),

        "o3-mini" => Some(LlmModelProfile {
            name: "o3-mini".into(),
            family: "o3-mini".into(),
            release_date: Some("2025-01-31".into()),
            last_updated: Some("2025-01-31".into()),
            attachment: false,
            reasoning: true,
            temperature: true,
            knowledge: Some("2023-10-01".into()),
            tool_call: true,
            structured_output: true,
            open_weights: false,
            cost: Some(LlmModelCost {
                input: 1.10,
                output: 4.40,
                cache_read: Some(0.55),
            }),
            limits: Some(LlmModelLimits {
                context: 200_000,
                output: 100_000,
            }),
            modalities: Some(LlmModelModalities {
                input: vec![Modality::Text],
                output: vec![Modality::Text],
            }),
            reasoning_effort: Some(reasoning_effort_standard()),
        }),

        "o3" => Some(LlmModelProfile {
            name: "o3".into(),
            family: "o3".into(),
            release_date: Some("2025-04-16".into()),
            last_updated: Some("2025-04-16".into()),
            attachment: true,
            reasoning: true,
            temperature: true,
            knowledge: Some("2024-06-01".into()),
            tool_call: true,
            structured_output: true,
            open_weights: false,
            cost: Some(LlmModelCost {
                input: 2.00,
                output: 8.00,
                cache_read: Some(1.00),
            }),
            limits: Some(LlmModelLimits {
                context: 200_000,
                output: 100_000,
            }),
            modalities: Some(LlmModelModalities {
                input: vec![Modality::Text, Modality::Image],
                output: vec![Modality::Text],
            }),
            reasoning_effort: Some(reasoning_effort_standard()),
        }),

        "o3-pro" => Some(LlmModelProfile {
            name: "o3 Pro".into(),
            family: "o3-pro".into(),
            release_date: Some("2025-06-10".into()),
            last_updated: Some("2025-06-10".into()),
            attachment: true,
            reasoning: true,
            temperature: true,
            knowledge: Some("2024-06-01".into()),
            tool_call: true,
            structured_output: true,
            open_weights: false,
            cost: Some(LlmModelCost {
                input: 20.00,
                output: 80.00,
                cache_read: None,
            }),
            limits: Some(LlmModelLimits {
                context: 200_000,
                output: 100_000,
            }),
            modalities: Some(LlmModelModalities {
                input: vec![Modality::Text, Modality::Image],
                output: vec![Modality::Text],
            }),
            reasoning_effort: Some(reasoning_effort_high_only()),
        }),

        "o4-mini" => Some(LlmModelProfile {
            name: "o4 mini".into(),
            family: "o4-mini".into(),
            release_date: Some("2025-04-16".into()),
            last_updated: Some("2025-04-16".into()),
            attachment: true,
            reasoning: true,
            temperature: true,
            knowledge: Some("2024-06-01".into()),
            tool_call: true,
            structured_output: true,
            open_weights: false,
            cost: Some(LlmModelCost {
                input: 1.10,
                output: 4.40,
                cache_read: Some(0.55),
            }),
            limits: Some(LlmModelLimits {
                context: 200_000,
                output: 100_000,
            }),
            modalities: Some(LlmModelModalities {
                input: vec![Modality::Text, Modality::Image],
                output: vec![Modality::Text],
            }),
            reasoning_effort: Some(reasoning_effort_standard()),
        }),

        // GPT-4.1 family models
        "gpt-4.1" => Some(LlmModelProfile {
            name: "GPT-4.1".into(),
            family: "gpt-4.1".into(),
            release_date: Some("2025-04-14".into()),
            last_updated: Some("2025-04-14".into()),
            attachment: true,
            reasoning: false,
            temperature: true,
            knowledge: Some("2024-06-01".into()),
            tool_call: true,
            structured_output: true,
            open_weights: false,
            cost: Some(LlmModelCost {
                input: 2.00,
                output: 8.00,
                cache_read: Some(1.00),
            }),
            limits: Some(LlmModelLimits {
                context: 128_000,
                output: 16_384,
            }),
            modalities: Some(LlmModelModalities {
                input: vec![Modality::Text, Modality::Image],
                output: vec![Modality::Text],
            }),
            reasoning_effort: None,
        }),

        "gpt-4.1-mini" => Some(LlmModelProfile {
            name: "GPT-4.1 mini".into(),
            family: "gpt-4.1-mini".into(),
            release_date: Some("2025-04-14".into()),
            last_updated: Some("2025-04-14".into()),
            attachment: true,
            reasoning: false,
            temperature: true,
            knowledge: Some("2024-06-01".into()),
            tool_call: true,
            structured_output: true,
            open_weights: false,
            cost: Some(LlmModelCost {
                input: 0.40,
                output: 1.60,
                cache_read: Some(0.20),
            }),
            limits: Some(LlmModelLimits {
                context: 128_000,
                output: 16_384,
            }),
            modalities: Some(LlmModelModalities {
                input: vec![Modality::Text, Modality::Image],
                output: vec![Modality::Text],
            }),
            reasoning_effort: None,
        }),

        "gpt-4.1-nano" => Some(LlmModelProfile {
            name: "GPT-4.1 nano".into(),
            family: "gpt-4.1-nano".into(),
            release_date: Some("2025-04-14".into()),
            last_updated: Some("2025-04-14".into()),
            attachment: true,
            reasoning: false,
            temperature: true,
            knowledge: Some("2024-06-01".into()),
            tool_call: true,
            structured_output: true,
            open_weights: false,
            cost: Some(LlmModelCost {
                input: 0.10,
                output: 0.40,
                cache_read: Some(0.05),
            }),
            limits: Some(LlmModelLimits {
                context: 128_000,
                output: 16_384,
            }),
            modalities: Some(LlmModelModalities {
                input: vec![Modality::Text, Modality::Image],
                output: vec![Modality::Text],
            }),
            reasoning_effort: None,
        }),

        // GPT-5 family models
        // Pre-5.1 models: default medium, supports low/medium/high (no none)
        "gpt-5" => Some(LlmModelProfile {
            name: "GPT-5".into(),
            family: "gpt-5".into(),
            release_date: Some("2025-08-07".into()),
            last_updated: Some("2025-08-07".into()),
            attachment: true,
            reasoning: true,
            temperature: true,
            knowledge: Some("2024-10-01".into()),
            tool_call: true,
            structured_output: true,
            open_weights: false,
            cost: Some(LlmModelCost {
                input: 1.25,
                output: 10.00,
                cache_read: Some(0.125),
            }),
            limits: Some(LlmModelLimits {
                context: 128_000,
                output: 128_000,
            }),
            modalities: Some(LlmModelModalities {
                input: vec![Modality::Text, Modality::Image],
                output: vec![Modality::Text],
            }),
            reasoning_effort: Some(reasoning_effort_gpt5_pre51()),
        }),

        "gpt-5-mini" => Some(LlmModelProfile {
            name: "GPT-5 mini".into(),
            family: "gpt-5-mini".into(),
            release_date: Some("2025-08-13".into()),
            last_updated: Some("2025-08-13".into()),
            attachment: true,
            reasoning: true,
            temperature: true,
            knowledge: Some("2024-06-01".into()),
            tool_call: true,
            structured_output: true,
            open_weights: false,
            cost: Some(LlmModelCost {
                input: 0.25,
                output: 2.00,
                cache_read: Some(0.025),
            }),
            limits: Some(LlmModelLimits {
                context: 128_000,
                output: 64_000,
            }),
            modalities: Some(LlmModelModalities {
                input: vec![Modality::Text, Modality::Image],
                output: vec![Modality::Text],
            }),
            reasoning_effort: Some(reasoning_effort_gpt5_pre51()),
        }),

        "gpt-5-nano" => Some(LlmModelProfile {
            name: "GPT-5 nano".into(),
            family: "gpt-5-nano".into(),
            release_date: Some("2025-08-13".into()),
            last_updated: Some("2025-08-13".into()),
            attachment: true,
            reasoning: true,
            temperature: true,
            knowledge: Some("2024-05-31".into()),
            tool_call: true,
            structured_output: true,
            open_weights: false,
            cost: Some(LlmModelCost {
                input: 0.05,
                output: 0.40,
                cache_read: Some(0.005),
            }),
            limits: Some(LlmModelLimits {
                context: 128_000,
                output: 64_000,
            }),
            modalities: Some(LlmModelModalities {
                input: vec![Modality::Text, Modality::Image],
                output: vec![Modality::Text],
            }),
            reasoning_effort: Some(reasoning_effort_gpt5_pre51()),
        }),

        "gpt-5-pro" => Some(LlmModelProfile {
            name: "GPT-5 Pro".into(),
            family: "gpt-5-pro".into(),
            release_date: Some("2025-08-07".into()),
            last_updated: Some("2025-08-07".into()),
            attachment: true,
            reasoning: true,
            temperature: true,
            knowledge: Some("2024-10-01".into()),
            tool_call: true,
            structured_output: true,
            open_weights: false,
            cost: Some(LlmModelCost {
                input: 15.00,
                output: 60.00,
                cache_read: None,
            }),
            limits: Some(LlmModelLimits {
                context: 128_000,
                output: 128_000,
            }),
            modalities: Some(LlmModelModalities {
                input: vec![Modality::Text, Modality::Image],
                output: vec![Modality::Text],
            }),
            reasoning_effort: Some(reasoning_effort_high_only()),
        }),

        "gpt-5-codex" => Some(LlmModelProfile {
            name: "GPT-5 Codex".into(),
            family: "gpt-5-codex".into(),
            release_date: Some("2025-08-07".into()),
            last_updated: Some("2025-08-07".into()),
            attachment: true,
            reasoning: true,
            temperature: true,
            knowledge: Some("2024-09-30".into()),
            tool_call: true,
            structured_output: true,
            open_weights: false,
            cost: Some(LlmModelCost {
                input: 1.25,
                output: 10.00,
                cache_read: Some(0.125),
            }),
            limits: Some(LlmModelLimits {
                context: 128_000,
                output: 128_000,
            }),
            modalities: Some(LlmModelModalities {
                input: vec![Modality::Text, Modality::Image],
                output: vec![Modality::Text],
            }),
            reasoning_effort: Some(reasoning_effort_gpt5_pre51()),
        }),

        // GPT-5.1 models: default none, supports none/low/medium/high
        "gpt-5.1" => Some(LlmModelProfile {
            name: "GPT-5.1".into(),
            family: "gpt-5.1".into(),
            release_date: Some("2025-11-13".into()),
            last_updated: Some("2025-11-13".into()),
            attachment: true,
            reasoning: true,
            temperature: true,
            knowledge: Some("2024-09-30".into()),
            tool_call: true,
            structured_output: true,
            open_weights: false,
            cost: Some(LlmModelCost {
                input: 1.50,
                output: 12.00,
                cache_read: Some(0.15),
            }),
            limits: Some(LlmModelLimits {
                context: 128_000,
                output: 128_000,
            }),
            modalities: Some(LlmModelModalities {
                input: vec![Modality::Text, Modality::Image],
                output: vec![Modality::Text],
            }),
            reasoning_effort: Some(reasoning_effort_gpt51()),
        }),

        "gpt-5.1-codex" => Some(LlmModelProfile {
            name: "GPT-5.1 Codex".into(),
            family: "gpt-5.1-codex".into(),
            release_date: Some("2025-11-13".into()),
            last_updated: Some("2025-11-13".into()),
            attachment: true,
            reasoning: true,
            temperature: true,
            knowledge: Some("2024-09-30".into()),
            tool_call: true,
            structured_output: true,
            open_weights: false,
            cost: Some(LlmModelCost {
                input: 1.50,
                output: 12.00,
                cache_read: Some(0.15),
            }),
            limits: Some(LlmModelLimits {
                context: 128_000,
                output: 128_000,
            }),
            modalities: Some(LlmModelModalities {
                input: vec![Modality::Text, Modality::Image],
                output: vec![Modality::Text],
            }),
            reasoning_effort: Some(reasoning_effort_gpt51()),
        }),

        "gpt-5.1-codex-mini" => Some(LlmModelProfile {
            name: "GPT-5.1 Codex mini".into(),
            family: "gpt-5.1-codex-mini".into(),
            release_date: Some("2025-11-13".into()),
            last_updated: Some("2025-11-13".into()),
            attachment: true,
            reasoning: true,
            temperature: true,
            knowledge: Some("2024-09-30".into()),
            tool_call: true,
            structured_output: true,
            open_weights: false,
            cost: Some(LlmModelCost {
                input: 0.30,
                output: 2.40,
                cache_read: Some(0.03),
            }),
            limits: Some(LlmModelLimits {
                context: 128_000,
                output: 100_000,
            }),
            modalities: Some(LlmModelModalities {
                input: vec![Modality::Text, Modality::Image],
                output: vec![Modality::Text],
            }),
            reasoning_effort: Some(reasoning_effort_gpt51()),
        }),

        // GPT-5.1-codex-max and after: supports xhigh
        "gpt-5.1-codex-max" => Some(LlmModelProfile {
            name: "GPT-5.1 Codex max".into(),
            family: "gpt-5.1-codex-max".into(),
            release_date: Some("2025-11-13".into()),
            last_updated: Some("2025-11-13".into()),
            attachment: true,
            reasoning: true,
            temperature: true,
            knowledge: Some("2024-09-30".into()),
            tool_call: true,
            structured_output: true,
            open_weights: false,
            cost: Some(LlmModelCost {
                input: 3.00,
                output: 24.00,
                cache_read: Some(0.30),
            }),
            limits: Some(LlmModelLimits {
                context: 128_000,
                output: 128_000,
            }),
            modalities: Some(LlmModelModalities {
                input: vec![Modality::Text, Modality::Image],
                output: vec![Modality::Text],
            }),
            reasoning_effort: Some(reasoning_effort_gpt52()),
        }),

        // GPT-5.2 models: supports xhigh
        "gpt-5.2" => Some(LlmModelProfile {
            name: "GPT-5.2".into(),
            family: "gpt-5.2".into(),
            release_date: Some("2025-12-11".into()),
            last_updated: Some("2025-12-11".into()),
            attachment: true,
            reasoning: true,
            temperature: false,
            knowledge: Some("2025-08-31".into()),
            tool_call: true,
            structured_output: true,
            open_weights: false,
            cost: Some(LlmModelCost {
                input: 1.75,
                output: 14.00,
                cache_read: Some(0.175),
            }),
            limits: Some(LlmModelLimits {
                context: 128_000,
                output: 64_000,
            }),
            modalities: Some(LlmModelModalities {
                input: vec![Modality::Text, Modality::Image],
                output: vec![Modality::Text],
            }),
            reasoning_effort: Some(reasoning_effort_gpt52()),
        }),

        "gpt-5.2-pro" => Some(LlmModelProfile {
            name: "GPT-5.2 Pro".into(),
            family: "gpt-5.2-pro".into(),
            release_date: Some("2025-12-11".into()),
            last_updated: Some("2025-12-11".into()),
            attachment: true,
            reasoning: true,
            temperature: false,
            knowledge: Some("2025-08-31".into()),
            tool_call: true,
            structured_output: true,
            open_weights: false,
            cost: Some(LlmModelCost {
                input: 17.50,
                output: 70.00,
                cache_read: None,
            }),
            limits: Some(LlmModelLimits {
                context: 128_000,
                output: 128_000,
            }),
            modalities: Some(LlmModelModalities {
                input: vec![Modality::Text, Modality::Image],
                output: vec![Modality::Text],
            }),
            reasoning_effort: Some(reasoning_effort_gpt52_pro()),
        }),

        "gpt-5.2-codex" => Some(LlmModelProfile {
            name: "GPT-5.2 Codex".into(),
            family: "gpt-5.2-codex".into(),
            release_date: Some("2025-12-11".into()),
            last_updated: Some("2025-12-11".into()),
            attachment: true,
            reasoning: true,
            temperature: false,
            knowledge: Some("2025-08-31".into()),
            tool_call: true,
            structured_output: true,
            open_weights: false,
            cost: Some(LlmModelCost {
                input: 1.75,
                output: 14.00,
                cache_read: Some(0.175),
            }),
            limits: Some(LlmModelLimits {
                context: 128_000,
                output: 128_000,
            }),
            modalities: Some(LlmModelModalities {
                input: vec![Modality::Text, Modality::Image],
                output: vec![Modality::Text],
            }),
            reasoning_effort: Some(reasoning_effort_gpt52()),
        }),

        // GPT-5 chat-latest models (point to latest chat-optimized versions)
        "gpt-5-chat-latest" => Some(LlmModelProfile {
            name: "GPT-5 Chat".into(),
            family: "gpt-5".into(),
            release_date: Some("2025-08-07".into()),
            last_updated: Some("2025-08-07".into()),
            attachment: true,
            reasoning: true,
            temperature: true,
            knowledge: Some("2024-10-01".into()),
            tool_call: true,
            structured_output: true,
            open_weights: false,
            cost: Some(LlmModelCost {
                input: 1.25,
                output: 10.00,
                cache_read: Some(0.125),
            }),
            limits: Some(LlmModelLimits {
                context: 128_000,
                output: 128_000,
            }),
            modalities: Some(LlmModelModalities {
                input: vec![Modality::Text, Modality::Image],
                output: vec![Modality::Text],
            }),
            reasoning_effort: Some(reasoning_effort_gpt5_pre51()),
        }),

        "gpt-5.1-chat-latest" => Some(LlmModelProfile {
            name: "GPT-5.1 Chat".into(),
            family: "gpt-5.1".into(),
            release_date: Some("2025-11-13".into()),
            last_updated: Some("2025-11-13".into()),
            attachment: true,
            reasoning: true,
            temperature: true,
            knowledge: Some("2024-09-30".into()),
            tool_call: true,
            structured_output: true,
            open_weights: false,
            cost: Some(LlmModelCost {
                input: 1.50,
                output: 12.00,
                cache_read: Some(0.15),
            }),
            limits: Some(LlmModelLimits {
                context: 128_000,
                output: 128_000,
            }),
            modalities: Some(LlmModelModalities {
                input: vec![Modality::Text, Modality::Image],
                output: vec![Modality::Text],
            }),
            reasoning_effort: Some(reasoning_effort_gpt51()),
        }),

        "gpt-5.2-chat-latest" => Some(LlmModelProfile {
            name: "GPT-5.2 Chat".into(),
            family: "gpt-5.2".into(),
            release_date: Some("2025-12-11".into()),
            last_updated: Some("2025-12-11".into()),
            attachment: true,
            reasoning: true,
            temperature: false,
            knowledge: Some("2025-08-31".into()),
            tool_call: true,
            structured_output: true,
            open_weights: false,
            cost: Some(LlmModelCost {
                input: 1.75,
                output: 14.00,
                cache_read: Some(0.175),
            }),
            limits: Some(LlmModelLimits {
                context: 128_000,
                output: 64_000,
            }),
            modalities: Some(LlmModelModalities {
                input: vec![Modality::Text, Modality::Image],
                output: vec![Modality::Text],
            }),
            reasoning_effort: Some(reasoning_effort_gpt52()),
        }),

        // Deep research models
        "o3-deep-research" => Some(LlmModelProfile {
            name: "o3 Deep Research".into(),
            family: "o3".into(),
            release_date: Some("2025-04-16".into()),
            last_updated: Some("2025-04-16".into()),
            attachment: true,
            reasoning: true,
            temperature: true,
            knowledge: Some("2024-06-01".into()),
            tool_call: true,
            structured_output: true,
            open_weights: false,
            cost: Some(LlmModelCost {
                input: 2.00,
                output: 8.00,
                cache_read: Some(1.00),
            }),
            limits: Some(LlmModelLimits {
                context: 200_000,
                output: 100_000,
            }),
            modalities: Some(LlmModelModalities {
                input: vec![Modality::Text, Modality::Image],
                output: vec![Modality::Text],
            }),
            reasoning_effort: Some(reasoning_effort_standard()),
        }),

        "o4-mini-deep-research" => Some(LlmModelProfile {
            name: "o4 mini Deep Research".into(),
            family: "o4-mini".into(),
            release_date: Some("2025-04-16".into()),
            last_updated: Some("2025-04-16".into()),
            attachment: true,
            reasoning: true,
            temperature: true,
            knowledge: Some("2024-06-01".into()),
            tool_call: true,
            structured_output: true,
            open_weights: false,
            cost: Some(LlmModelCost {
                input: 1.10,
                output: 4.40,
                cache_read: Some(0.55),
            }),
            limits: Some(LlmModelLimits {
                context: 200_000,
                output: 100_000,
            }),
            modalities: Some(LlmModelModalities {
                input: vec![Modality::Text, Modality::Image],
                output: vec![Modality::Text],
            }),
            reasoning_effort: Some(reasoning_effort_standard()),
        }),

        "o1-preview" => Some(LlmModelProfile {
            name: "o1 Preview".into(),
            family: "o1".into(),
            release_date: Some("2024-09-12".into()),
            last_updated: Some("2024-09-12".into()),
            attachment: false,
            reasoning: true,
            temperature: true,
            knowledge: Some("2023-10-01".into()),
            tool_call: false,
            structured_output: false,
            open_weights: false,
            cost: Some(LlmModelCost {
                input: 15.00,
                output: 60.00,
                cache_read: Some(7.50),
            }),
            limits: Some(LlmModelLimits {
                context: 128_000,
                output: 32_768,
            }),
            modalities: Some(LlmModelModalities {
                input: vec![Modality::Text],
                output: vec![Modality::Text],
            }),
            reasoning_effort: Some(reasoning_effort_standard()),
        }),

        _ => None,
    }
}

fn get_anthropic_profile(model_id: &str) -> Option<LlmModelProfile> {
    // Normalize model ID by extracting base name
    let base_id = normalize_anthropic_model_id(model_id);

    match base_id {
        // Claude 4.5 series (newest)
        "claude-opus-4-5" => Some(LlmModelProfile {
            name: "Claude Opus 4.5".into(),
            family: "claude-opus-4-5".into(),
            release_date: Some("2025-11-24".into()),
            last_updated: Some("2025-11-24".into()),
            attachment: true,
            reasoning: true,
            temperature: true,
            knowledge: Some("2025-04-01".into()),
            tool_call: true,
            structured_output: true,
            open_weights: false,
            cost: Some(LlmModelCost {
                input: 5.00,
                output: 25.00,
                cache_read: Some(0.50),
            }),
            limits: Some(LlmModelLimits {
                context: 200_000,
                output: 64_000,
            }),
            modalities: Some(LlmModelModalities {
                input: vec![Modality::Text, Modality::Image],
                output: vec![Modality::Text],
            }),
            reasoning_effort: None, // Anthropic uses extended thinking
        }),

        "claude-sonnet-4-5" => Some(LlmModelProfile {
            name: "Claude Sonnet 4.5".into(),
            family: "claude-sonnet-4-5".into(),
            release_date: Some("2025-09-29".into()),
            last_updated: Some("2025-09-29".into()),
            attachment: true,
            reasoning: true,
            temperature: true,
            knowledge: Some("2025-04-01".into()),
            tool_call: true,
            structured_output: true,
            open_weights: false,
            cost: Some(LlmModelCost {
                input: 3.00,
                output: 15.00,
                cache_read: Some(0.30),
            }),
            limits: Some(LlmModelLimits {
                context: 200_000,
                output: 64_000,
            }),
            modalities: Some(LlmModelModalities {
                input: vec![Modality::Text, Modality::Image],
                output: vec![Modality::Text],
            }),
            reasoning_effort: None,
        }),

        "claude-haiku-4-5" => Some(LlmModelProfile {
            name: "Claude Haiku 4.5".into(),
            family: "claude-haiku-4-5".into(),
            release_date: Some("2025-10-15".into()),
            last_updated: Some("2025-10-15".into()),
            attachment: true,
            reasoning: true,
            temperature: true,
            knowledge: Some("2025-04-01".into()),
            tool_call: true,
            structured_output: true,
            open_weights: false,
            cost: Some(LlmModelCost {
                input: 1.00,
                output: 5.00,
                cache_read: Some(0.10),
            }),
            limits: Some(LlmModelLimits {
                context: 200_000,
                output: 16_000,
            }),
            modalities: Some(LlmModelModalities {
                input: vec![Modality::Text, Modality::Image],
                output: vec![Modality::Text],
            }),
            reasoning_effort: None,
        }),

        // Claude 4 series
        "claude-sonnet-4" => Some(LlmModelProfile {
            name: "Claude Sonnet 4".into(),
            family: "claude-sonnet-4".into(),
            release_date: Some("2025-05-14".into()),
            last_updated: Some("2025-05-14".into()),
            attachment: true,
            reasoning: false,
            temperature: true,
            knowledge: Some("2025-03-01".into()),
            tool_call: true,
            structured_output: true,
            open_weights: false,
            cost: Some(LlmModelCost {
                input: 3.00,
                output: 15.00,
                cache_read: Some(0.30),
            }),
            limits: Some(LlmModelLimits {
                context: 200_000,
                output: 16_000,
            }),
            modalities: Some(LlmModelModalities {
                input: vec![Modality::Text, Modality::Image],
                output: vec![Modality::Text],
            }),
            reasoning_effort: None,
        }),

        "claude-opus-4" => Some(LlmModelProfile {
            name: "Claude Opus 4".into(),
            family: "claude-opus-4".into(),
            release_date: Some("2025-05-14".into()),
            last_updated: Some("2025-05-14".into()),
            attachment: true,
            reasoning: true,
            temperature: true,
            knowledge: Some("2025-03-01".into()),
            tool_call: true,
            structured_output: true,
            open_weights: false,
            cost: Some(LlmModelCost {
                input: 15.00,
                output: 75.00,
                cache_read: Some(1.50),
            }),
            limits: Some(LlmModelLimits {
                context: 200_000,
                output: 32_000,
            }),
            modalities: Some(LlmModelModalities {
                input: vec![Modality::Text, Modality::Image],
                output: vec![Modality::Text],
            }),
            reasoning_effort: None, // Anthropic uses extended thinking, not reasoning effort
        }),

        // Claude 3.7 series
        "claude-3-7-sonnet" => Some(LlmModelProfile {
            name: "Claude 3.7 Sonnet".into(),
            family: "claude-3-7-sonnet".into(),
            release_date: Some("2025-02-19".into()),
            last_updated: Some("2025-02-19".into()),
            attachment: true,
            reasoning: true, // Extended thinking mode
            temperature: true,
            knowledge: Some("2024-11-01".into()),
            tool_call: true,
            structured_output: true,
            open_weights: false,
            cost: Some(LlmModelCost {
                input: 3.00,
                output: 15.00,
                cache_read: Some(0.30),
            }),
            limits: Some(LlmModelLimits {
                context: 200_000,
                output: 64_000, // Extended output with thinking
            }),
            modalities: Some(LlmModelModalities {
                input: vec![Modality::Text, Modality::Image],
                output: vec![Modality::Text],
            }),
            reasoning_effort: None,
        }),

        // Claude 3.5 series
        "claude-3-5-sonnet" => Some(LlmModelProfile {
            name: "Claude 3.5 Sonnet".into(),
            family: "claude-3-5-sonnet".into(),
            release_date: Some("2024-06-20".into()),
            last_updated: Some("2024-10-22".into()),
            attachment: true,
            reasoning: false,
            temperature: true,
            knowledge: Some("2024-04-01".into()),
            tool_call: true,
            structured_output: true,
            open_weights: false,
            cost: Some(LlmModelCost {
                input: 3.00,
                output: 15.00,
                cache_read: Some(0.30),
            }),
            limits: Some(LlmModelLimits {
                context: 200_000,
                output: 8_192,
            }),
            modalities: Some(LlmModelModalities {
                input: vec![Modality::Text, Modality::Image],
                output: vec![Modality::Text],
            }),
            reasoning_effort: None,
        }),

        "claude-3-5-haiku" => Some(LlmModelProfile {
            name: "Claude 3.5 Haiku".into(),
            family: "claude-3-5-haiku".into(),
            release_date: Some("2024-10-22".into()),
            last_updated: Some("2024-10-22".into()),
            attachment: true,
            reasoning: false,
            temperature: true,
            knowledge: Some("2024-07-01".into()),
            tool_call: true,
            structured_output: true,
            open_weights: false,
            cost: Some(LlmModelCost {
                input: 1.00,
                output: 5.00,
                cache_read: Some(0.10),
            }),
            limits: Some(LlmModelLimits {
                context: 200_000,
                output: 8_192,
            }),
            modalities: Some(LlmModelModalities {
                input: vec![Modality::Text, Modality::Image],
                output: vec![Modality::Text],
            }),
            reasoning_effort: None,
        }),

        "claude-3-opus" => Some(LlmModelProfile {
            name: "Claude 3 Opus".into(),
            family: "claude-3-opus".into(),
            release_date: Some("2024-02-29".into()),
            last_updated: Some("2024-02-29".into()),
            attachment: true,
            reasoning: false,
            temperature: true,
            knowledge: Some("2023-08-01".into()),
            tool_call: true,
            structured_output: true,
            open_weights: false,
            cost: Some(LlmModelCost {
                input: 15.00,
                output: 75.00,
                cache_read: Some(1.50),
            }),
            limits: Some(LlmModelLimits {
                context: 200_000,
                output: 4_096,
            }),
            modalities: Some(LlmModelModalities {
                input: vec![Modality::Text, Modality::Image],
                output: vec![Modality::Text],
            }),
            reasoning_effort: None,
        }),

        "claude-3-sonnet" => Some(LlmModelProfile {
            name: "Claude 3 Sonnet".into(),
            family: "claude-3-sonnet".into(),
            release_date: Some("2024-02-29".into()),
            last_updated: Some("2024-02-29".into()),
            attachment: true,
            reasoning: false,
            temperature: true,
            knowledge: Some("2023-08-01".into()),
            tool_call: true,
            structured_output: true,
            open_weights: false,
            cost: Some(LlmModelCost {
                input: 3.00,
                output: 15.00,
                cache_read: Some(0.30),
            }),
            limits: Some(LlmModelLimits {
                context: 200_000,
                output: 4_096,
            }),
            modalities: Some(LlmModelModalities {
                input: vec![Modality::Text, Modality::Image],
                output: vec![Modality::Text],
            }),
            reasoning_effort: None,
        }),

        "claude-3-haiku" => Some(LlmModelProfile {
            name: "Claude 3 Haiku".into(),
            family: "claude-3-haiku".into(),
            release_date: Some("2024-03-07".into()),
            last_updated: Some("2024-03-07".into()),
            attachment: true,
            reasoning: false,
            temperature: true,
            knowledge: Some("2023-08-01".into()),
            tool_call: true,
            structured_output: true,
            open_weights: false,
            cost: Some(LlmModelCost {
                input: 0.25,
                output: 1.25,
                cache_read: Some(0.03),
            }),
            limits: Some(LlmModelLimits {
                context: 200_000,
                output: 4_096,
            }),
            modalities: Some(LlmModelModalities {
                input: vec![Modality::Text, Modality::Image],
                output: vec![Modality::Text],
            }),
            reasoning_effort: None,
        }),

        _ => None,
    }
}

/// Normalize OpenAI model ID to base name
/// e.g., "gpt-4o-2024-11-20" -> "gpt-4o"
fn normalize_model_id(model_id: &str) -> &str {
    // Known base model patterns (order matters - more specific first)
    let patterns = [
        // GPT-5.2 models
        "gpt-5.2-chat-latest",
        "gpt-5.2-codex",
        "gpt-5.2-pro",
        "gpt-5.2",
        // GPT-5.1 models
        "gpt-5.1-chat-latest",
        "gpt-5.1-codex-max",
        "gpt-5.1-codex-mini",
        "gpt-5.1-codex",
        "gpt-5.1",
        // GPT-5 models
        "gpt-5-chat-latest",
        "gpt-5-codex",
        "gpt-5-nano",
        "gpt-5-mini",
        "gpt-5-pro",
        "gpt-5",
        // GPT-4.1 models
        "gpt-4.1-nano",
        "gpt-4.1-mini",
        "gpt-4.1",
        // GPT-4 models
        "gpt-4o-mini",
        "gpt-4o",
        // Reasoning models (o-series)
        "o4-mini-deep-research",
        "o4-mini",
        "o3-deep-research",
        "o3-pro",
        "o3-mini",
        "o3",
        "o1-preview",
        "o1-mini",
        "o1-pro",
        "o1",
    ];

    for pattern in patterns {
        if model_id == pattern || model_id.starts_with(&format!("{}-", pattern)) {
            return pattern;
        }
    }

    model_id
}

/// Normalize Anthropic model ID to base name
/// e.g., "claude-3-5-sonnet-20241022" -> "claude-3-5-sonnet"
fn normalize_anthropic_model_id(model_id: &str) -> &str {
    // Known base model patterns (order matters - more specific first)
    let patterns = [
        // Claude 4.5 series
        "claude-opus-4-5",
        "claude-sonnet-4-5",
        "claude-haiku-4-5",
        // Claude 4 series
        "claude-sonnet-4",
        "claude-opus-4",
        // Claude 3.7 series
        "claude-3-7-sonnet",
        // Claude 3.5 series
        "claude-3-5-sonnet",
        "claude-3-5-haiku",
        // Claude 3 series
        "claude-3-opus",
        "claude-3-sonnet",
        "claude-3-haiku",
    ];

    for pattern in patterns {
        if model_id == pattern
            || model_id.starts_with(&format!("{}-", pattern))
            || model_id == format!("{}-latest", pattern)
        {
            return pattern;
        }
    }

    model_id
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_get_profile_openai_gpt4o() {
        let profile = get_model_profile(&LlmProviderType::Openai, "gpt-4o");
        assert!(profile.is_some());
        let profile = profile.unwrap();
        assert_eq!(profile.name, "GPT-4o");
        assert_eq!(profile.family, "gpt-4o");
        assert!(profile.tool_call);
        assert!(profile.structured_output);
    }

    #[test]
    fn test_get_profile_openai_gpt4o_versioned() {
        let profile = get_model_profile(&LlmProviderType::Openai, "gpt-4o-2024-11-20");
        assert!(profile.is_some());
        let profile = profile.unwrap();
        assert_eq!(profile.name, "GPT-4o");
    }

    #[test]
    fn test_get_profile_anthropic_claude35_sonnet() {
        let profile = get_model_profile(&LlmProviderType::Anthropic, "claude-3-5-sonnet-20241022");
        assert!(profile.is_some());
        let profile = profile.unwrap();
        assert_eq!(profile.name, "Claude 3.5 Sonnet");
        assert!(profile.tool_call);
    }

    #[test]
    fn test_get_profile_anthropic_claude_sonnet4() {
        let profile = get_model_profile(&LlmProviderType::Anthropic, "claude-sonnet-4-20250514");
        assert!(profile.is_some());
        let profile = profile.unwrap();
        assert_eq!(profile.name, "Claude Sonnet 4");
    }

    #[test]
    fn test_get_profile_unknown_model() {
        let profile = get_model_profile(&LlmProviderType::Openai, "unknown-model");
        assert!(profile.is_none());
    }

    #[test]
    fn test_get_profile_wrong_provider() {
        // Try to get an OpenAI model with Anthropic provider
        let profile = get_model_profile(&LlmProviderType::Anthropic, "gpt-4o");
        assert!(profile.is_none());
    }

    #[test]
    fn test_profile_has_cost_and_limits() {
        let profile = get_model_profile(&LlmProviderType::Openai, "gpt-4o").unwrap();
        assert!(profile.cost.is_some());
        assert!(profile.limits.is_some());

        let cost = profile.cost.unwrap();
        assert!(cost.input > 0.0);
        assert!(cost.output > 0.0);

        let limits = profile.limits.unwrap();
        assert!(limits.context > 0);
        assert!(limits.output > 0);
    }

    #[test]
    fn test_o1_has_reasoning() {
        let profile = get_model_profile(&LlmProviderType::Openai, "o1").unwrap();
        assert!(profile.reasoning);
    }

    #[test]
    fn test_claude_opus_4_has_reasoning() {
        let profile = get_model_profile(&LlmProviderType::Anthropic, "claude-opus-4").unwrap();
        assert!(profile.reasoning);
    }

    #[test]
    fn test_normalize_openai_model_id() {
        assert_eq!(normalize_model_id("gpt-4o"), "gpt-4o");
        assert_eq!(normalize_model_id("gpt-4o-2024-11-20"), "gpt-4o");
        assert_eq!(normalize_model_id("gpt-4o-mini"), "gpt-4o-mini");
        assert_eq!(normalize_model_id("o1-2024-12-17"), "o1");
        assert_eq!(normalize_model_id("o1-mini"), "o1-mini");
    }

    #[test]
    fn test_normalize_anthropic_model_id() {
        assert_eq!(
            normalize_anthropic_model_id("claude-3-5-sonnet"),
            "claude-3-5-sonnet"
        );
        assert_eq!(
            normalize_anthropic_model_id("claude-3-5-sonnet-20241022"),
            "claude-3-5-sonnet"
        );
        assert_eq!(
            normalize_anthropic_model_id("claude-3-5-sonnet-latest"),
            "claude-3-5-sonnet"
        );
        assert_eq!(
            normalize_anthropic_model_id("claude-sonnet-4-20250514"),
            "claude-sonnet-4"
        );
    }

    #[test]
    fn test_azure_uses_openai_profiles() {
        let profile = get_model_profile(&LlmProviderType::AzureOpenAI, "gpt-4o");
        assert!(profile.is_some());
        assert_eq!(profile.unwrap().name, "GPT-4o");
    }

    // GPT-5 model tests

    #[test]
    fn test_gpt5_profile() {
        let profile = get_model_profile(&LlmProviderType::Openai, "gpt-5").unwrap();
        assert_eq!(profile.name, "GPT-5");
        assert_eq!(profile.family, "gpt-5");
        assert!(profile.reasoning);
        assert!(profile.tool_call);

        // Pre-5.1 reasoning effort: default medium, supports low/medium/high
        let effort = profile.reasoning_effort.unwrap();
        assert_eq!(effort.default, ReasoningEffort::Medium);
        assert_eq!(effort.values.len(), 3);
        assert!(!effort
            .values
            .iter()
            .any(|v| v.value == ReasoningEffort::None));
    }

    #[test]
    fn test_gpt5_mini_profile() {
        let profile = get_model_profile(&LlmProviderType::Openai, "gpt-5-mini").unwrap();
        assert_eq!(profile.name, "GPT-5 mini");
        assert!(profile.reasoning);

        let effort = profile.reasoning_effort.unwrap();
        assert_eq!(effort.default, ReasoningEffort::Medium);
    }

    #[test]
    fn test_gpt5_pro_profile() {
        let profile = get_model_profile(&LlmProviderType::Openai, "gpt-5-pro").unwrap();
        assert_eq!(profile.name, "GPT-5 Pro");
        assert!(profile.reasoning);

        // gpt-5-pro: only supports high
        let effort = profile.reasoning_effort.unwrap();
        assert_eq!(effort.default, ReasoningEffort::High);
        assert_eq!(effort.values.len(), 1);
        assert_eq!(effort.values[0].value, ReasoningEffort::High);
    }

    #[test]
    fn test_gpt51_profile() {
        let profile = get_model_profile(&LlmProviderType::Openai, "gpt-5.1").unwrap();
        assert_eq!(profile.name, "GPT-5.1");
        assert!(profile.reasoning);
        assert!(profile.tool_call);

        // gpt-5.1: default none, supports none/low/medium/high
        let effort = profile.reasoning_effort.unwrap();
        assert_eq!(effort.default, ReasoningEffort::None);
        assert_eq!(effort.values.len(), 4);
        assert!(effort
            .values
            .iter()
            .any(|v| v.value == ReasoningEffort::None));
        assert!(!effort
            .values
            .iter()
            .any(|v| v.value == ReasoningEffort::Xhigh));
    }

    #[test]
    fn test_gpt51_codex_max_profile() {
        let profile = get_model_profile(&LlmProviderType::Openai, "gpt-5.1-codex-max").unwrap();
        assert_eq!(profile.name, "GPT-5.1 Codex max");
        assert!(profile.reasoning);

        // After gpt-5.1-codex-max: supports xhigh
        let effort = profile.reasoning_effort.unwrap();
        assert!(effort
            .values
            .iter()
            .any(|v| v.value == ReasoningEffort::Xhigh));
    }

    #[test]
    fn test_gpt52_profile() {
        let profile = get_model_profile(&LlmProviderType::Openai, "gpt-5.2").unwrap();
        assert_eq!(profile.name, "GPT-5.2");
        assert!(profile.reasoning);

        // gpt-5.2: default none, supports none/low/medium/high/xhigh
        let effort = profile.reasoning_effort.unwrap();
        assert_eq!(effort.default, ReasoningEffort::None);
        assert_eq!(effort.values.len(), 5);
        assert!(effort
            .values
            .iter()
            .any(|v| v.value == ReasoningEffort::Xhigh));
    }

    #[test]
    fn test_gpt52_pro_profile() {
        let profile = get_model_profile(&LlmProviderType::Openai, "gpt-5.2-pro").unwrap();
        assert_eq!(profile.name, "GPT-5.2 Pro");
        assert!(profile.reasoning);

        // gpt-5.2-pro: default medium, supports medium/high/xhigh
        let effort = profile.reasoning_effort.unwrap();
        assert_eq!(effort.default, ReasoningEffort::Medium);
        assert_eq!(effort.values.len(), 3);
        assert!(effort
            .values
            .iter()
            .any(|v| v.value == ReasoningEffort::Xhigh));
        assert!(!effort
            .values
            .iter()
            .any(|v| v.value == ReasoningEffort::None));
    }

    #[test]
    fn test_normalize_gpt5_model_ids() {
        assert_eq!(normalize_model_id("gpt-5"), "gpt-5");
        assert_eq!(normalize_model_id("gpt-5-2025-08-07"), "gpt-5");
        assert_eq!(normalize_model_id("gpt-5-mini"), "gpt-5-mini");
        assert_eq!(normalize_model_id("gpt-5-nano"), "gpt-5-nano");
        assert_eq!(normalize_model_id("gpt-5-pro"), "gpt-5-pro");
        assert_eq!(normalize_model_id("gpt-5-codex"), "gpt-5-codex");
        assert_eq!(normalize_model_id("gpt-5.1"), "gpt-5.1");
        assert_eq!(normalize_model_id("gpt-5.1-codex"), "gpt-5.1-codex");
        assert_eq!(
            normalize_model_id("gpt-5.1-codex-mini"),
            "gpt-5.1-codex-mini"
        );
        assert_eq!(normalize_model_id("gpt-5.1-codex-max"), "gpt-5.1-codex-max");
        assert_eq!(normalize_model_id("gpt-5.2"), "gpt-5.2");
        assert_eq!(normalize_model_id("gpt-5.2-pro"), "gpt-5.2-pro");
        assert_eq!(normalize_model_id("gpt-5.2-codex"), "gpt-5.2-codex");
    }

    // GPT-4.1 model tests

    #[test]
    fn test_gpt41_profile() {
        let profile = get_model_profile(&LlmProviderType::Openai, "gpt-4.1").unwrap();
        assert_eq!(profile.name, "GPT-4.1");
        assert_eq!(profile.family, "gpt-4.1");
        assert!(!profile.reasoning);
        assert!(profile.tool_call);
    }

    #[test]
    fn test_gpt41_mini_profile() {
        let profile = get_model_profile(&LlmProviderType::Openai, "gpt-4.1-mini").unwrap();
        assert_eq!(profile.name, "GPT-4.1 mini");
        assert!(!profile.reasoning);
    }

    #[test]
    fn test_gpt41_nano_profile() {
        let profile = get_model_profile(&LlmProviderType::Openai, "gpt-4.1-nano").unwrap();
        assert_eq!(profile.name, "GPT-4.1 nano");
        assert!(!profile.reasoning);
    }

    // o3/o4 reasoning model tests

    #[test]
    fn test_o3_profile() {
        let profile = get_model_profile(&LlmProviderType::Openai, "o3").unwrap();
        assert_eq!(profile.name, "o3");
        assert!(profile.reasoning);
        assert!(profile.tool_call);
        let effort = profile.reasoning_effort.unwrap();
        assert_eq!(effort.default, ReasoningEffort::Medium);
    }

    #[test]
    fn test_o3_pro_profile() {
        let profile = get_model_profile(&LlmProviderType::Openai, "o3-pro").unwrap();
        assert_eq!(profile.name, "o3 Pro");
        assert!(profile.reasoning);
        let effort = profile.reasoning_effort.unwrap();
        assert_eq!(effort.default, ReasoningEffort::High);
    }

    #[test]
    fn test_o4_mini_profile() {
        let profile = get_model_profile(&LlmProviderType::Openai, "o4-mini").unwrap();
        assert_eq!(profile.name, "o4 mini");
        assert!(profile.reasoning);
        let effort = profile.reasoning_effort.unwrap();
        assert_eq!(effort.default, ReasoningEffort::Medium);
    }

    // Claude 4.5 model tests

    #[test]
    fn test_claude_opus_45_profile() {
        let profile =
            get_model_profile(&LlmProviderType::Anthropic, "claude-opus-4-5-20251101").unwrap();
        assert_eq!(profile.name, "Claude Opus 4.5");
        assert!(profile.reasoning);
        assert!(profile.tool_call);
    }

    #[test]
    fn test_claude_sonnet_45_profile() {
        let profile =
            get_model_profile(&LlmProviderType::Anthropic, "claude-sonnet-4-5-20250929").unwrap();
        assert_eq!(profile.name, "Claude Sonnet 4.5");
        assert!(profile.reasoning);
    }

    #[test]
    fn test_claude_haiku_45_profile() {
        let profile =
            get_model_profile(&LlmProviderType::Anthropic, "claude-haiku-4-5-20251001").unwrap();
        assert_eq!(profile.name, "Claude Haiku 4.5");
        assert!(profile.reasoning);
    }

    #[test]
    fn test_claude_37_sonnet_profile() {
        let profile =
            get_model_profile(&LlmProviderType::Anthropic, "claude-3-7-sonnet-20250219").unwrap();
        assert_eq!(profile.name, "Claude 3.7 Sonnet");
        assert!(profile.reasoning);
        assert!(profile.tool_call);
    }

    // Normalize tests for new models

    #[test]
    fn test_normalize_gpt41_model_ids() {
        assert_eq!(normalize_model_id("gpt-4.1"), "gpt-4.1");
        assert_eq!(normalize_model_id("gpt-4.1-2025-04-14"), "gpt-4.1");
        assert_eq!(normalize_model_id("gpt-4.1-mini"), "gpt-4.1-mini");
        assert_eq!(normalize_model_id("gpt-4.1-nano"), "gpt-4.1-nano");
    }

    #[test]
    fn test_normalize_o_series_model_ids() {
        assert_eq!(normalize_model_id("o3"), "o3");
        assert_eq!(normalize_model_id("o3-2025-04-16"), "o3");
        assert_eq!(normalize_model_id("o3-pro"), "o3-pro");
        assert_eq!(normalize_model_id("o4-mini"), "o4-mini");
    }

    #[test]
    fn test_normalize_claude_45_model_ids() {
        assert_eq!(
            normalize_anthropic_model_id("claude-opus-4-5-20251101"),
            "claude-opus-4-5"
        );
        assert_eq!(
            normalize_anthropic_model_id("claude-sonnet-4-5-20250929"),
            "claude-sonnet-4-5"
        );
        assert_eq!(
            normalize_anthropic_model_id("claude-haiku-4-5-20251001"),
            "claude-haiku-4-5"
        );
    }

    #[test]
    fn test_normalize_claude_37_model_ids() {
        assert_eq!(
            normalize_anthropic_model_id("claude-3-7-sonnet"),
            "claude-3-7-sonnet"
        );
        assert_eq!(
            normalize_anthropic_model_id("claude-3-7-sonnet-20250219"),
            "claude-3-7-sonnet"
        );
    }
}
