// Hardcoded LLM Model Profiles
//
// This module provides model profiles based on models.dev structure.
// Profiles are matched by provider_type + model_id.
//
// IMPORTANT: Never guess or extrapolate profile data (pricing, limits, capabilities).
// Always source from https://github.com/sst/models.dev/tree/dev/providers
// and cross-reference with official provider documentation. If a model is not
// yet listed on models.dev, wait until the data is available before adding it.
//
// NOTE: Currently only includes profiles for selected models.
// Additional model profiles can be added as needed by extending the match arms.
//
// Data source: https://github.com/sst/models.dev/tree/dev/providers
// Cross-referenced with official Anthropic and OpenAI documentation

use crate::llm_models::{
    CostTier, LlmModelCost, LlmModelLimits, LlmModelModalities, LlmModelProfile, LlmProviderType,
    Modality, ReasoningEffort, ReasoningEffortConfig, ReasoningEffortValue,
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

/// Reasoning effort for gpt-5.5
/// Default: medium, supports: none, low, medium, high, xhigh
fn reasoning_effort_gpt55() -> ReasoningEffortConfig {
    ReasoningEffortConfig {
        values: vec![
            effort(ReasoningEffort::None, "None"),
            effort(ReasoningEffort::Low, "Low"),
            effort(ReasoningEffort::Medium, "Medium"),
            effort(ReasoningEffort::High, "High"),
            effort(ReasoningEffort::Xhigh, "Extra High"),
        ],
        default: ReasoningEffort::Medium,
    }
}

/// Reasoning effort for OpenAI Realtime voice sessions.
fn reasoning_effort_realtime() -> ReasoningEffortConfig {
    ReasoningEffortConfig {
        values: vec![
            effort(ReasoningEffort::Minimal, "Minimal"),
            effort(ReasoningEffort::Low, "Low"),
            effort(ReasoningEffort::Medium, "Medium"),
            effort(ReasoningEffort::High, "High"),
            effort(ReasoningEffort::Xhigh, "Extra High"),
        ],
        default: ReasoningEffort::Low,
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

/// Extended thinking config for Anthropic Claude models
/// Maps to thinking budget_tokens: low=1024, medium=4096, high=16384, xhigh=32768
fn reasoning_effort_anthropic_extended_thinking() -> ReasoningEffortConfig {
    ReasoningEffortConfig {
        values: vec![
            effort(ReasoningEffort::Low, "Low (1K tokens)"),
            effort(ReasoningEffort::Medium, "Medium (4K tokens)"),
            effort(ReasoningEffort::High, "High (16K tokens)"),
            effort(ReasoningEffort::Xhigh, "Extra High (32K tokens)"),
        ],
        default: ReasoningEffort::Medium,
    }
}

/// Adaptive thinking config for recent Claude reasoning models
/// (Opus 4.7, Opus 4.6, Sonnet 4.6)
/// Uses thinking.type="adaptive" with effort parameter instead of budget_tokens
/// Default: high, supports: low, medium, high, max (mapped to xhigh)
fn reasoning_effort_anthropic_adaptive_thinking() -> ReasoningEffortConfig {
    ReasoningEffortConfig {
        values: vec![
            effort(ReasoningEffort::Low, "Low"),
            effort(ReasoningEffort::Medium, "Medium"),
            effort(ReasoningEffort::High, "High"),
            effort(ReasoningEffort::Xhigh, "Max"),
        ],
        default: ReasoningEffort::High,
    }
}

/// Get a model profile by matching provider_type and model_id
/// Returns None if no matching profile is found
pub fn get_model_profile(
    provider_type: &LlmProviderType,
    model_id: &str,
) -> Option<LlmModelProfile> {
    match provider_type {
        LlmProviderType::Openai
        | LlmProviderType::AzureOpenai
        | LlmProviderType::OpenaiCompletions => get_openai_profile(model_id),
        LlmProviderType::Anthropic => get_anthropic_profile(model_id),
        LlmProviderType::Gemini => get_gemini_profile(model_id),
        LlmProviderType::LlmSim => get_llmsim_profile(model_id),
    }
}

fn get_openai_profile(model_id: &str) -> Option<LlmModelProfile> {
    // Normalize model ID by extracting base name
    let base_id = normalize_model_id(model_id);

    match base_id {
        "gpt-realtime-2" => Some(LlmModelProfile {
            name: "GPT Realtime 2".into(),
            family: "gpt-realtime".into(),
            description: Some("OpenAI Realtime model for low-latency voice sessions".into()),
            release_date: None,
            last_updated: None,
            attachment: false,
            reasoning: true,
            temperature: false,
            knowledge: None,
            tool_call: true,
            structured_output: false,
            open_weights: false,
            cost: None,
            limits: None,
            modalities: Some(LlmModelModalities {
                input: vec![Modality::Text, Modality::Audio],
                output: vec![Modality::Text, Modality::Audio],
            }),
            reasoning_effort: Some(reasoning_effort_realtime()),
            tool_search: false,
            supports_phases: true,
        }),

        "gpt-4o" => Some(LlmModelProfile {
            name: "GPT-4o".into(),
            family: "gpt-4o".into(),
            description: None,
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
                cost_tiers: vec![],
            }),
            limits: Some(LlmModelLimits {
                context: 128_000,
                input: None,
                output: 16_384,
                max_media: None,
            }),
            modalities: Some(LlmModelModalities {
                input: vec![Modality::Text, Modality::Image, Modality::Audio],
                output: vec![Modality::Text, Modality::Audio],
            }),
            reasoning_effort: None,
            tool_search: false,
            supports_phases: false,
        }),

        "gpt-4o-mini" => Some(LlmModelProfile {
            name: "GPT-4o mini".into(),
            family: "gpt-4o-mini".into(),
            description: None,
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
                cost_tiers: vec![],
            }),
            limits: Some(LlmModelLimits {
                context: 128_000,
                input: None,
                output: 16_384,
                max_media: None,
            }),
            modalities: Some(LlmModelModalities {
                input: vec![Modality::Text, Modality::Image],
                output: vec![Modality::Text],
            }),
            reasoning_effort: None,
            tool_search: false,
            supports_phases: false,
        }),

        "o1" => Some(LlmModelProfile {
            name: "o1".into(),
            family: "o1".into(),
            description: None,
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
                cost_tiers: vec![],
            }),
            limits: Some(LlmModelLimits {
                context: 200_000,
                input: None,
                output: 100_000,
                max_media: None,
            }),
            modalities: Some(LlmModelModalities {
                input: vec![Modality::Text, Modality::Image],
                output: vec![Modality::Text],
            }),
            reasoning_effort: Some(reasoning_effort_standard()),
            tool_search: false,
            supports_phases: false,
        }),

        "o1-mini" => Some(LlmModelProfile {
            name: "o1-mini".into(),
            family: "o1-mini".into(),
            description: None,
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
                cost_tiers: vec![],
            }),
            limits: Some(LlmModelLimits {
                context: 128_000,
                input: None,
                output: 65_536,
                max_media: None,
            }),
            modalities: Some(LlmModelModalities {
                input: vec![Modality::Text],
                output: vec![Modality::Text],
            }),
            reasoning_effort: Some(reasoning_effort_standard()),
            tool_search: false,
            supports_phases: false,
        }),

        "o1-pro" => Some(LlmModelProfile {
            name: "o1-pro".into(),
            family: "o1-pro".into(),
            description: None,
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
                cost_tiers: vec![],
            }),
            limits: Some(LlmModelLimits {
                context: 200_000,
                input: None,
                output: 100_000,
                max_media: None,
            }),
            modalities: Some(LlmModelModalities {
                input: vec![Modality::Text, Modality::Image],
                output: vec![Modality::Text],
            }),
            reasoning_effort: Some(reasoning_effort_high_only()),
            tool_search: false,
            supports_phases: false,
        }),

        "o3-mini" => Some(LlmModelProfile {
            name: "o3-mini".into(),
            family: "o3-mini".into(),
            description: None,
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
                cost_tiers: vec![],
            }),
            limits: Some(LlmModelLimits {
                context: 200_000,
                input: None,
                output: 100_000,
                max_media: None,
            }),
            modalities: Some(LlmModelModalities {
                input: vec![Modality::Text],
                output: vec![Modality::Text],
            }),
            reasoning_effort: Some(reasoning_effort_standard()),
            tool_search: false,
            supports_phases: false,
        }),

        "o3" => Some(LlmModelProfile {
            name: "o3".into(),
            family: "o3".into(),
            description: None,
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
                cost_tiers: vec![],
            }),
            limits: Some(LlmModelLimits {
                context: 200_000,
                input: None,
                output: 100_000,
                max_media: None,
            }),
            modalities: Some(LlmModelModalities {
                input: vec![Modality::Text, Modality::Image],
                output: vec![Modality::Text],
            }),
            reasoning_effort: Some(reasoning_effort_standard()),
            tool_search: false,
            supports_phases: false,
        }),

        "o3-pro" => Some(LlmModelProfile {
            name: "o3 Pro".into(),
            family: "o3-pro".into(),
            description: None,
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
                cost_tiers: vec![],
            }),
            limits: Some(LlmModelLimits {
                context: 200_000,
                input: None,
                output: 100_000,
                max_media: None,
            }),
            modalities: Some(LlmModelModalities {
                input: vec![Modality::Text, Modality::Image],
                output: vec![Modality::Text],
            }),
            reasoning_effort: Some(reasoning_effort_high_only()),
            tool_search: false,
            supports_phases: false,
        }),

        "o4-mini" => Some(LlmModelProfile {
            name: "o4 mini".into(),
            family: "o4-mini".into(),
            description: None,
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
                cost_tiers: vec![],
            }),
            limits: Some(LlmModelLimits {
                context: 200_000,
                input: None,
                output: 100_000,
                max_media: None,
            }),
            modalities: Some(LlmModelModalities {
                input: vec![Modality::Text, Modality::Image],
                output: vec![Modality::Text],
            }),
            reasoning_effort: Some(reasoning_effort_standard()),
            tool_search: false,
            supports_phases: false,
        }),

        // GPT-4.1 family models
        "gpt-4.1" => Some(LlmModelProfile {
            name: "GPT-4.1".into(),
            family: "gpt-4.1".into(),
            description: None,
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
                cost_tiers: vec![],
            }),
            limits: Some(LlmModelLimits {
                context: 128_000,
                input: None,
                output: 16_384,
                max_media: None,
            }),
            modalities: Some(LlmModelModalities {
                input: vec![Modality::Text, Modality::Image],
                output: vec![Modality::Text],
            }),
            reasoning_effort: None,
            tool_search: false,
            supports_phases: false,
        }),

        "gpt-4.1-mini" => Some(LlmModelProfile {
            name: "GPT-4.1 mini".into(),
            family: "gpt-4.1-mini".into(),
            description: None,
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
                cost_tiers: vec![],
            }),
            limits: Some(LlmModelLimits {
                context: 128_000,
                input: None,
                output: 16_384,
                max_media: None,
            }),
            modalities: Some(LlmModelModalities {
                input: vec![Modality::Text, Modality::Image],
                output: vec![Modality::Text],
            }),
            reasoning_effort: None,
            tool_search: false,
            supports_phases: false,
        }),

        "gpt-4.1-nano" => Some(LlmModelProfile {
            name: "GPT-4.1 nano".into(),
            family: "gpt-4.1-nano".into(),
            description: None,
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
                cost_tiers: vec![],
            }),
            limits: Some(LlmModelLimits {
                context: 128_000,
                input: None,
                output: 16_384,
                max_media: None,
            }),
            modalities: Some(LlmModelModalities {
                input: vec![Modality::Text, Modality::Image],
                output: vec![Modality::Text],
            }),
            reasoning_effort: None,
            tool_search: false,
            supports_phases: false,
        }),

        // GPT-5 family models
        // Pre-5.1 models: default medium, supports low/medium/high (no none)
        "gpt-5" => Some(LlmModelProfile {
            name: "GPT-5".into(),
            family: "gpt-5".into(),
            description: None,
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
                cost_tiers: vec![],
            }),
            limits: Some(LlmModelLimits {
                context: 128_000,
                input: None,
                output: 128_000,
                max_media: None,
            }),
            modalities: Some(LlmModelModalities {
                input: vec![Modality::Text, Modality::Image],
                output: vec![Modality::Text],
            }),
            reasoning_effort: Some(reasoning_effort_gpt5_pre51()),
            tool_search: false,
            supports_phases: false,
        }),

        "gpt-5-mini" => Some(LlmModelProfile {
            name: "GPT-5 mini".into(),
            family: "gpt-5-mini".into(),
            description: None,
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
                cost_tiers: vec![],
            }),
            limits: Some(LlmModelLimits {
                context: 128_000,
                input: None,
                output: 64_000,
                max_media: None,
            }),
            modalities: Some(LlmModelModalities {
                input: vec![Modality::Text, Modality::Image],
                output: vec![Modality::Text],
            }),
            reasoning_effort: Some(reasoning_effort_gpt5_pre51()),
            tool_search: false,
            supports_phases: false,
        }),

        "gpt-5-nano" => Some(LlmModelProfile {
            name: "GPT-5 nano".into(),
            family: "gpt-5-nano".into(),
            description: None,
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
                cost_tiers: vec![],
            }),
            limits: Some(LlmModelLimits {
                context: 128_000,
                input: None,
                output: 64_000,
                max_media: None,
            }),
            modalities: Some(LlmModelModalities {
                input: vec![Modality::Text, Modality::Image],
                output: vec![Modality::Text],
            }),
            reasoning_effort: Some(reasoning_effort_gpt5_pre51()),
            tool_search: false,
            supports_phases: false,
        }),

        "gpt-5-pro" => Some(LlmModelProfile {
            name: "GPT-5 Pro".into(),
            family: "gpt-5-pro".into(),
            description: None,
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
                cost_tiers: vec![],
            }),
            limits: Some(LlmModelLimits {
                context: 128_000,
                input: None,
                output: 128_000,
                max_media: None,
            }),
            modalities: Some(LlmModelModalities {
                input: vec![Modality::Text, Modality::Image],
                output: vec![Modality::Text],
            }),
            reasoning_effort: Some(reasoning_effort_high_only()),
            tool_search: false,
            supports_phases: false,
        }),

        "gpt-5-codex" => Some(LlmModelProfile {
            name: "GPT-5 Codex".into(),
            family: "gpt-5-codex".into(),
            description: None,
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
                cost_tiers: vec![],
            }),
            limits: Some(LlmModelLimits {
                context: 128_000,
                input: None,
                output: 128_000,
                max_media: None,
            }),
            modalities: Some(LlmModelModalities {
                input: vec![Modality::Text, Modality::Image],
                output: vec![Modality::Text],
            }),
            reasoning_effort: Some(reasoning_effort_gpt5_pre51()),
            tool_search: false,
            supports_phases: false,
        }),

        // GPT-5.1 models: default none, supports none/low/medium/high
        "gpt-5.1" => Some(LlmModelProfile {
            name: "GPT-5.1".into(),
            family: "gpt-5.1".into(),
            description: None,
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
                cost_tiers: vec![],
            }),
            limits: Some(LlmModelLimits {
                context: 128_000,
                input: None,
                output: 128_000,
                max_media: None,
            }),
            modalities: Some(LlmModelModalities {
                input: vec![Modality::Text, Modality::Image],
                output: vec![Modality::Text],
            }),
            reasoning_effort: Some(reasoning_effort_gpt51()),
            tool_search: false,
            supports_phases: false,
        }),

        "gpt-5.1-codex" => Some(LlmModelProfile {
            name: "GPT-5.1 Codex".into(),
            family: "gpt-5.1-codex".into(),
            description: None,
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
                cost_tiers: vec![],
            }),
            limits: Some(LlmModelLimits {
                context: 128_000,
                input: None,
                output: 128_000,
                max_media: None,
            }),
            modalities: Some(LlmModelModalities {
                input: vec![Modality::Text, Modality::Image],
                output: vec![Modality::Text],
            }),
            reasoning_effort: Some(reasoning_effort_gpt51()),
            tool_search: false,
            supports_phases: false,
        }),

        "gpt-5.1-codex-mini" => Some(LlmModelProfile {
            name: "GPT-5.1 Codex mini".into(),
            family: "gpt-5.1-codex-mini".into(),
            description: None,
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
                cost_tiers: vec![],
            }),
            limits: Some(LlmModelLimits {
                context: 128_000,
                input: None,
                output: 100_000,
                max_media: None,
            }),
            modalities: Some(LlmModelModalities {
                input: vec![Modality::Text, Modality::Image],
                output: vec![Modality::Text],
            }),
            reasoning_effort: Some(reasoning_effort_gpt51()),
            tool_search: false,
            supports_phases: false,
        }),

        // GPT-5.1-codex-max and after: supports xhigh
        "gpt-5.1-codex-max" => Some(LlmModelProfile {
            name: "GPT-5.1 Codex max".into(),
            family: "gpt-5.1-codex-max".into(),
            description: None,
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
                cost_tiers: vec![],
            }),
            limits: Some(LlmModelLimits {
                context: 128_000,
                input: None,
                output: 128_000,
                max_media: None,
            }),
            modalities: Some(LlmModelModalities {
                input: vec![Modality::Text, Modality::Image],
                output: vec![Modality::Text],
            }),
            reasoning_effort: Some(reasoning_effort_gpt52()),
            tool_search: false,
            supports_phases: false,
        }),

        // GPT-5.2 models: supports xhigh, 400K context
        "gpt-5.2" => Some(LlmModelProfile {
            name: "GPT-5.2".into(),
            family: "gpt-5.2".into(),
            description: None,
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
                cost_tiers: vec![],
            }),
            limits: Some(LlmModelLimits {
                context: 400_000,
                input: None,
                output: 128_000,
                max_media: None,
            }),
            modalities: Some(LlmModelModalities {
                input: vec![Modality::Text, Modality::Image],
                output: vec![Modality::Text],
            }),
            reasoning_effort: Some(reasoning_effort_gpt52()),
            tool_search: false,
            supports_phases: false,
        }),

        "gpt-5.2-pro" => Some(LlmModelProfile {
            name: "GPT-5.2 Pro".into(),
            family: "gpt-5.2-pro".into(),
            description: None,
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
                input: 21.00,
                output: 168.00,
                cache_read: None,
                cost_tiers: vec![],
            }),
            limits: Some(LlmModelLimits {
                context: 400_000,
                input: None,
                output: 128_000,
                max_media: None,
            }),
            modalities: Some(LlmModelModalities {
                input: vec![Modality::Text, Modality::Image],
                output: vec![Modality::Text],
            }),
            reasoning_effort: Some(reasoning_effort_gpt52_pro()),
            tool_search: false,
            supports_phases: false,
        }),

        "gpt-5.2-codex" => Some(LlmModelProfile {
            name: "GPT-5.2 Codex".into(),
            family: "gpt-5.2-codex".into(),
            description: None,
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
                cost_tiers: vec![],
            }),
            limits: Some(LlmModelLimits {
                context: 400_000,
                input: None,
                output: 128_000,
                max_media: None,
            }),
            modalities: Some(LlmModelModalities {
                input: vec![Modality::Text, Modality::Image],
                output: vec![Modality::Text],
            }),
            reasoning_effort: Some(reasoning_effort_gpt52()),
            tool_search: false,
            supports_phases: false,
        }),

        // GPT-5.3 Codex: same pricing as 5.2, 25% faster inference
        "gpt-5.3-codex" => Some(LlmModelProfile {
            name: "GPT-5.3 Codex".into(),
            family: "gpt-5.3-codex".into(),
            description: None,
            release_date: Some("2026-02-05".into()),
            last_updated: Some("2026-02-05".into()),
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
                cost_tiers: vec![],
            }),
            limits: Some(LlmModelLimits {
                context: 400_000,
                input: None,
                output: 128_000,
                max_media: None,
            }),
            modalities: Some(LlmModelModalities {
                input: vec![Modality::Text, Modality::Image],
                output: vec![Modality::Text],
            }),
            reasoning_effort: Some(reasoning_effort_gpt52()),
            tool_search: false,
            supports_phases: false,
        }),

        // GPT-5.5 family: latest flagship reasoning models. Released 2026-04-23.
        // Flat pricing (no 200K context tiers, unlike 5.4).
        // Source: developers.openai.com/api/docs/models/gpt-5.5 and .../gpt-5.5-pro
        // (models.dev did not yet list these variants at the time of addition;
        // refresh once the models.dev entry appears).
        "gpt-5.5" => Some(LlmModelProfile {
            name: "GPT-5.5".into(),
            family: "gpt-5.5".into(),
            description: Some("Latest flagship reasoning model. Best for complex multi-step tasks, code generation, and deep analysis.".into()),
            release_date: Some("2026-04-23".into()),
            last_updated: Some("2026-04-23".into()),
            attachment: true,
            reasoning: true,
            temperature: false,
            knowledge: Some("2025-12-01".into()),
            tool_call: true,
            structured_output: true,
            open_weights: false,
            cost: Some(LlmModelCost {
                input: 5.00,
                output: 30.00,
                cache_read: Some(0.50),
                cost_tiers: vec![],
            }),
            limits: Some(LlmModelLimits {
                context: 1_050_000,
                input: None,
                output: 128_000,
                max_media: None,
            }),
            modalities: Some(LlmModelModalities {
                input: vec![Modality::Text, Modality::Image],
                output: vec![Modality::Text],
            }),
            reasoning_effort: Some(reasoning_effort_gpt55()),
            // tool_search gated off pending live verification of the Responses-API
            // deferred-loading round-trip on gpt-5.5. The round-trip reproducibly
            // fails with a server_error during the reasoning phase (EVE-521), making
            // the agent loop unusable on the default model. gpt-5.4 keeps tool_search
            // (it has live integration coverage); re-enable here once the gpt-5.5
            // round-trip is verified end-to-end.
            tool_search: false,
            supports_phases: true,
        }),

        "gpt-5.5-pro" => Some(LlmModelProfile {
            name: "GPT-5.5 Pro".into(),
            family: "gpt-5.5-pro".into(),
            description: Some("Extended-thinking variant for the hardest problems. Trades speed for deeper reasoning on math, science, and complex code.".into()),
            release_date: Some("2026-04-23".into()),
            last_updated: Some("2026-04-23".into()),
            attachment: true,
            reasoning: true,
            temperature: false,
            knowledge: Some("2025-12-01".into()),
            tool_call: true,
            structured_output: true,
            open_weights: false,
            cost: Some(LlmModelCost {
                input: 30.00,
                output: 180.00,
                cache_read: None,
                cost_tiers: vec![],
            }),
            limits: Some(LlmModelLimits {
                context: 1_050_000,
                input: None,
                output: 128_000,
                max_media: None,
            }),
            modalities: Some(LlmModelModalities {
                input: vec![Modality::Text, Modality::Image],
                output: vec![Modality::Text],
            }),
            reasoning_effort: Some(reasoning_effort_gpt52_pro()),
            // Gated off with gpt-5.5: the deferred-loading round-trip fails during the
            // reasoning phase on the gpt-5.5 family (EVE-521). Re-enable once verified.
            tool_search: false,
            supports_phases: true,
        }),

        // GPT-5.4 family: reasoning models with 1.05M context, tool_search, native phases.
        // Released 2026-03-05 (5.4, 5.4-pro), 2026-03-17 (5.4-mini, 5.4-nano).
        // 5.4 and 5.4-pro have tiered pricing above 200K context tokens.
        "gpt-5.4" => Some(LlmModelProfile {
            name: "GPT-5.4".into(),
            family: "gpt-5.4".into(),
            description: Some("Flagship reasoning model with 1M+ context. Best for complex multi-step tasks, code generation, and deep analysis.".into()),
            release_date: Some("2026-03-05".into()),
            last_updated: Some("2026-03-05".into()),
            attachment: true,
            reasoning: true,
            temperature: false,
            knowledge: Some("2025-08-31".into()),
            tool_call: true,
            structured_output: true,
            open_weights: false,
            cost: Some(LlmModelCost {
                input: 2.50,
                output: 15.00,
                cache_read: Some(0.25),
                cost_tiers: vec![CostTier {
                    above_tokens: 200_000,
                    input: 5.00,
                    output: 22.50,
                    cache_read: Some(0.50),
                }],
            }),
            limits: Some(LlmModelLimits {
                context: 1_050_000,
                input: None,
                output: 128_000,
                max_media: None,
            }),
            modalities: Some(LlmModelModalities {
                input: vec![Modality::Text, Modality::Image, Modality::Pdf],
                output: vec![Modality::Text],
            }),
            reasoning_effort: Some(reasoning_effort_gpt52()),
            tool_search: true,
            supports_phases: true,
        }),

        "gpt-5.4-mini" => Some(LlmModelProfile {
            name: "GPT-5.4 mini".into(),
            family: "gpt-5.4-mini".into(),
            description: Some("Fast, cost-effective reasoning model. Balances strong performance with low latency for everyday tasks.".into()),
            release_date: Some("2026-03-17".into()),
            last_updated: Some("2026-03-17".into()),
            attachment: true,
            reasoning: true,
            temperature: false,
            knowledge: Some("2025-08-31".into()),
            tool_call: true,
            structured_output: true,
            open_weights: false,
            cost: Some(LlmModelCost {
                input: 0.75,
                output: 4.50,
                cache_read: Some(0.075),
                cost_tiers: vec![],
            }),
            limits: Some(LlmModelLimits {
                context: 400_000,
                input: None,
                output: 128_000,
                max_media: None,
            }),
            modalities: Some(LlmModelModalities {
                input: vec![Modality::Text, Modality::Image],
                output: vec![Modality::Text],
            }),
            reasoning_effort: Some(reasoning_effort_gpt52()),
            tool_search: true,
            supports_phases: true,
        }),

        "gpt-5.4-nano" => Some(LlmModelProfile {
            name: "GPT-5.4 nano".into(),
            family: "gpt-5.4-nano".into(),
            description: Some("Smallest and cheapest GPT-5.4 variant. Ideal for high-volume, latency-sensitive workloads.".into()),
            release_date: Some("2026-03-17".into()),
            last_updated: Some("2026-03-17".into()),
            attachment: true,
            reasoning: true,
            temperature: false,
            knowledge: Some("2025-08-31".into()),
            tool_call: true,
            structured_output: true,
            open_weights: false,
            cost: Some(LlmModelCost {
                input: 0.20,
                output: 1.25,
                cache_read: Some(0.02),
                cost_tiers: vec![],
            }),
            limits: Some(LlmModelLimits {
                context: 400_000,
                input: None,
                output: 128_000,
                max_media: None,
            }),
            modalities: Some(LlmModelModalities {
                input: vec![Modality::Text, Modality::Image],
                output: vec![Modality::Text],
            }),
            reasoning_effort: Some(reasoning_effort_gpt52()),
            tool_search: true,
            supports_phases: true,
        }),

        "gpt-5.4-pro" => Some(LlmModelProfile {
            name: "GPT-5.4 Pro".into(),
            family: "gpt-5.4-pro".into(),
            description: Some("Extended-thinking variant for the hardest problems. Trades speed for deeper reasoning on math, science, and complex code.".into()),
            release_date: Some("2026-03-05".into()),
            last_updated: Some("2026-03-05".into()),
            attachment: true,
            reasoning: true,
            temperature: false,
            knowledge: Some("2025-08-31".into()),
            tool_call: true,
            structured_output: false,
            open_weights: false,
            cost: Some(LlmModelCost {
                input: 30.00,
                output: 180.00,
                cache_read: None,
                cost_tiers: vec![CostTier {
                    above_tokens: 200_000,
                    input: 60.00,
                    output: 270.00,
                    cache_read: None,
                }],
            }),
            limits: Some(LlmModelLimits {
                context: 1_050_000,
                input: None,
                output: 128_000,
                max_media: None,
            }),
            modalities: Some(LlmModelModalities {
                input: vec![Modality::Text, Modality::Image],
                output: vec![Modality::Text],
            }),
            reasoning_effort: Some(reasoning_effort_gpt52_pro()),
            tool_search: true,
            supports_phases: true,
        }),

        // GPT-5 chat-latest models (point to latest chat-optimized versions)
        "gpt-5-chat-latest" => Some(LlmModelProfile {
            name: "GPT-5 Chat".into(),
            family: "gpt-5".into(),
            description: None,
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
                cost_tiers: vec![],
            }),
            limits: Some(LlmModelLimits {
                context: 128_000,
                input: None,
                output: 128_000,
                max_media: None,
            }),
            modalities: Some(LlmModelModalities {
                input: vec![Modality::Text, Modality::Image],
                output: vec![Modality::Text],
            }),
            reasoning_effort: Some(reasoning_effort_gpt5_pre51()),
            tool_search: false,
            supports_phases: false,
        }),

        "gpt-5.1-chat-latest" => Some(LlmModelProfile {
            name: "GPT-5.1 Chat".into(),
            family: "gpt-5.1".into(),
            description: None,
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
                cost_tiers: vec![],
            }),
            limits: Some(LlmModelLimits {
                context: 128_000,
                input: None,
                output: 128_000,
                max_media: None,
            }),
            modalities: Some(LlmModelModalities {
                input: vec![Modality::Text, Modality::Image],
                output: vec![Modality::Text],
            }),
            reasoning_effort: Some(reasoning_effort_gpt51()),
            tool_search: false,
            supports_phases: false,
        }),

        "gpt-5.2-chat-latest" => Some(LlmModelProfile {
            name: "GPT-5.2 Chat".into(),
            family: "gpt-5.2".into(),
            description: None,
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
                cost_tiers: vec![],
            }),
            limits: Some(LlmModelLimits {
                context: 128_000,
                input: None,
                output: 16_384,
                max_media: None,
            }),
            modalities: Some(LlmModelModalities {
                input: vec![Modality::Text, Modality::Image],
                output: vec![Modality::Text],
            }),
            reasoning_effort: Some(reasoning_effort_gpt52()),
            tool_search: false,
            supports_phases: false,
        }),

        // Deep research models
        "o3-deep-research" => Some(LlmModelProfile {
            name: "o3 Deep Research".into(),
            family: "o3".into(),
            description: None,
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
                cost_tiers: vec![],
            }),
            limits: Some(LlmModelLimits {
                context: 200_000,
                input: None,
                output: 100_000,
                max_media: None,
            }),
            modalities: Some(LlmModelModalities {
                input: vec![Modality::Text, Modality::Image],
                output: vec![Modality::Text],
            }),
            reasoning_effort: Some(reasoning_effort_standard()),
            tool_search: false,
            supports_phases: false,
        }),

        "o4-mini-deep-research" => Some(LlmModelProfile {
            name: "o4 mini Deep Research".into(),
            family: "o4-mini".into(),
            description: None,
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
                cost_tiers: vec![],
            }),
            limits: Some(LlmModelLimits {
                context: 200_000,
                input: None,
                output: 100_000,
                max_media: None,
            }),
            modalities: Some(LlmModelModalities {
                input: vec![Modality::Text, Modality::Image],
                output: vec![Modality::Text],
            }),
            reasoning_effort: Some(reasoning_effort_standard()),
            tool_search: false,
            supports_phases: false,
        }),

        "o1-preview" => Some(LlmModelProfile {
            name: "o1 Preview".into(),
            family: "o1".into(),
            description: None,
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
                cost_tiers: vec![],
            }),
            limits: Some(LlmModelLimits {
                context: 128_000,
                input: None,
                output: 32_768,
                max_media: None,
            }),
            modalities: Some(LlmModelModalities {
                input: vec![Modality::Text],
                output: vec![Modality::Text],
            }),
            reasoning_effort: Some(reasoning_effort_standard()),
            tool_search: false,
            supports_phases: false,
        }),

        // Fall through to third-party models served over the OpenAI-compatible
        // Chat Completions API (NVIDIA NIM, Alibaba, MiniMax, Moonshot, xAI, etc.).
        _ => get_openai_compatible_profile(base_id),
    }
}

/// Profiles for non-OpenAI models exposed through the OpenAI-compatible
/// Chat Completions API (`openai_completions` provider type). Sourced from
/// models.dev unless noted otherwise.
///
/// Each arm accepts both the bare provider id and common vendor-prefixed
/// aliases (OpenRouter style) so the profile resolves regardless of how the
/// user registered the model id.
///
/// `structured_output` is set to `false` where the upstream models.dev entry
/// does not assert it: absence of the field is not a claim of support, so we
/// do not advertise a capability we cannot confirm.
fn get_openai_compatible_profile(model_id: &str) -> Option<LlmModelProfile> {
    match model_id {
        // NVIDIA Nemotron 3 Super — flagship Nemotron reasoning model.
        // Source: models.dev (nvidia provider).
        "nemotron-3-super-120b-a12b" | "nvidia/nemotron-3-super-120b-a12b" => {
            Some(LlmModelProfile {
                name: "Nemotron 3 Super".into(),
                family: "nemotron-3-super".into(),
                description: None,
                release_date: Some("2026-03-11".into()),
                last_updated: Some("2026-03-11".into()),
                attachment: false,
                reasoning: true,
                temperature: true,
                knowledge: Some("2024-04-01".into()),
                tool_call: true,
                structured_output: false,
                open_weights: true,
                cost: Some(LlmModelCost {
                    input: 0.20,
                    output: 0.80,
                    cache_read: None,
                    cost_tiers: vec![],
                }),
                limits: Some(LlmModelLimits {
                    context: 262_144,
                    input: None,
                    output: 262_144,
                    max_media: None,
                }),
                modalities: Some(LlmModelModalities {
                    input: vec![Modality::Text],
                    output: vec![Modality::Text],
                }),
                reasoning_effort: None,
                tool_search: false,
                supports_phases: false,
            })
        }

        // Alibaba Qwen3.7 Max — flagship Qwen model.
        // Source: models.dev (alibaba provider). Knowledge cutoff not published.
        "qwen3.7-max" | "qwen/qwen3.7-max" => Some(LlmModelProfile {
            name: "Qwen3.7 Max".into(),
            family: "qwen3.7-max".into(),
            description: None,
            release_date: Some("2026-05-21".into()),
            last_updated: Some("2026-05-21".into()),
            attachment: false,
            reasoning: true,
            temperature: true,
            knowledge: None,
            tool_call: true,
            structured_output: false,
            open_weights: false,
            cost: Some(LlmModelCost {
                input: 2.50,
                output: 7.50,
                cache_read: Some(0.50),
                cost_tiers: vec![],
            }),
            limits: Some(LlmModelLimits {
                context: 1_000_000,
                input: None,
                output: 65_536,
                max_media: None,
            }),
            modalities: Some(LlmModelModalities {
                input: vec![Modality::Text],
                output: vec![Modality::Text],
            }),
            reasoning_effort: None,
            tool_search: false,
            supports_phases: false,
        }),

        // Microsoft MAI-1-preview — Microsoft's first end-to-end in-house
        // foundation model. Not on models.dev; sourced from Microsoft's official
        // announcement (microsoft.ai/news/two-new-in-house-models). It is a
        // text-only instruction model (not a reasoning model); context window,
        // pricing, and knowledge cutoff were never publicly disclosed, so cost
        // and limits are left unset rather than guessed.
        "MAI-1-preview" | "mai-1-preview" | "microsoft/MAI-1-preview" => Some(LlmModelProfile {
            name: "MAI-1-preview".into(),
            family: "mai-1-preview".into(),
            description: None,
            release_date: Some("2025-08-28".into()),
            last_updated: None,
            attachment: false,
            reasoning: false,
            temperature: true,
            knowledge: None,
            tool_call: false,
            structured_output: false,
            open_weights: false,
            cost: None,
            limits: None,
            modalities: Some(LlmModelModalities {
                input: vec![Modality::Text],
                output: vec![Modality::Text],
            }),
            reasoning_effort: None,
            tool_search: false,
            supports_phases: false,
        }),

        // MiniMax-M3 — flagship MiniMax model. Source: models.dev (minimax
        // provider). Reasoning is a toggle upstream (no graded effort).
        "MiniMax-M3" | "minimax/MiniMax-M3" => Some(LlmModelProfile {
            name: "MiniMax-M3".into(),
            family: "minimax-m3".into(),
            description: None,
            release_date: Some("2026-06-01".into()),
            last_updated: Some("2026-06-01".into()),
            attachment: true,
            reasoning: true,
            temperature: true,
            knowledge: None,
            tool_call: true,
            structured_output: false,
            open_weights: true,
            cost: Some(LlmModelCost {
                input: 0.60,
                output: 2.40,
                cache_read: Some(0.12),
                cost_tiers: vec![],
            }),
            limits: Some(LlmModelLimits {
                context: 512_000,
                input: None,
                output: 128_000,
                max_media: None,
            }),
            modalities: Some(LlmModelModalities {
                input: vec![Modality::Text, Modality::Image, Modality::Video],
                output: vec![Modality::Text],
            }),
            reasoning_effort: None,
            tool_search: false,
            supports_phases: false,
        }),

        // Moonshot Kimi K2 Thinking — flagship Kimi reasoning model.
        // Source: models.dev (moonshotai provider).
        "kimi-k2-thinking" | "moonshotai/kimi-k2-thinking" => Some(LlmModelProfile {
            name: "Kimi K2 Thinking".into(),
            family: "kimi-k2-thinking".into(),
            description: None,
            release_date: Some("2025-11-06".into()),
            last_updated: Some("2025-11-06".into()),
            attachment: false,
            reasoning: true,
            temperature: true,
            knowledge: Some("2024-08-01".into()),
            tool_call: true,
            structured_output: false,
            open_weights: true,
            cost: Some(LlmModelCost {
                input: 0.60,
                output: 2.50,
                cache_read: Some(0.15),
                cost_tiers: vec![],
            }),
            limits: Some(LlmModelLimits {
                context: 262_144,
                input: None,
                output: 262_144,
                max_media: None,
            }),
            modalities: Some(LlmModelModalities {
                input: vec![Modality::Text],
                output: vec![Modality::Text],
            }),
            reasoning_effort: None,
            tool_search: false,
            supports_phases: false,
        }),

        // xAI Grok 4.3 — flagship Grok model. Source: models.dev (xai provider).
        // Pricing has a >200K-token tier. Knowledge cutoff not published.
        "grok-4.3" | "x-ai/grok-4.3" | "xai/grok-4.3" => Some(LlmModelProfile {
            name: "Grok 4.3".into(),
            family: "grok-4.3".into(),
            description: None,
            release_date: Some("2026-04-17".into()),
            last_updated: Some("2026-04-17".into()),
            attachment: true,
            reasoning: true,
            temperature: true,
            knowledge: None,
            tool_call: true,
            structured_output: false,
            open_weights: false,
            cost: Some(LlmModelCost {
                input: 1.25,
                output: 2.50,
                cache_read: Some(0.20),
                cost_tiers: vec![CostTier {
                    above_tokens: 200_000,
                    input: 2.50,
                    output: 5.00,
                    cache_read: Some(0.40),
                }],
            }),
            limits: Some(LlmModelLimits {
                context: 1_000_000,
                input: None,
                output: 30_000,
                max_media: None,
            }),
            modalities: Some(LlmModelModalities {
                input: vec![Modality::Text, Modality::Image, Modality::Pdf],
                output: vec![Modality::Text],
            }),
            reasoning_effort: None,
            tool_search: false,
            supports_phases: false,
        }),

        _ => None,
    }
}

fn get_anthropic_profile(model_id: &str) -> Option<LlmModelProfile> {
    // Normalize model ID by extracting base name
    let base_id = normalize_anthropic_model_id(model_id);

    match base_id {
        // Claude 4.8 series (newest)
        // Source: Anthropic model card (claude-api skill `shared/models.md`) and
        // docs.claude.com — Opus 4.8 is not yet in models.dev. Same API surface as
        // Opus 4.7: adaptive thinking only, sampling parameters removed (temperature
        // returns 400, hence `temperature: false`). Release/knowledge dates are not
        // published in the model card; the Models API exposes them at runtime.
        "claude-opus-4-8" => Some(LlmModelProfile {
            name: "Claude Opus 4.8".into(),
            family: "claude-opus-4-8".into(),
            description: None,
            release_date: None,
            last_updated: None,
            attachment: true,
            reasoning: true,
            temperature: false,
            knowledge: None,
            tool_call: true,
            structured_output: true,
            open_weights: false,
            cost: Some(LlmModelCost {
                input: 5.00,
                output: 25.00,
                cache_read: Some(0.50),
                cost_tiers: vec![],
            }),
            limits: Some(LlmModelLimits {
                context: 1_000_000,
                input: None,
                output: 128_000,
                max_media: None,
            }),
            modalities: Some(LlmModelModalities {
                input: vec![Modality::Text, Modality::Image, Modality::Pdf],
                output: vec![Modality::Text],
            }),
            reasoning_effort: Some(reasoning_effort_anthropic_adaptive_thinking()),
            tool_search: false,
            supports_phases: false,
        }),

        // Claude 4.7 series
        "claude-opus-4-7" => Some(LlmModelProfile {
            name: "Claude Opus 4.7".into(),
            family: "claude-opus-4-7".into(),
            description: None,
            release_date: Some("2026-04-16".into()),
            last_updated: Some("2026-04-16".into()),
            attachment: true,
            reasoning: true,
            temperature: true,
            knowledge: Some("2026-01-01".into()),
            tool_call: true,
            structured_output: true,
            open_weights: false,
            cost: Some(LlmModelCost {
                input: 5.00,
                output: 25.00,
                cache_read: Some(0.50),
                cost_tiers: vec![],
            }),
            limits: Some(LlmModelLimits {
                context: 1_000_000,
                input: None,
                output: 128_000,
                max_media: None,
            }),
            modalities: Some(LlmModelModalities {
                input: vec![Modality::Text, Modality::Image, Modality::Pdf],
                output: vec![Modality::Text],
            }),
            reasoning_effort: Some(reasoning_effort_anthropic_adaptive_thinking()),
            tool_search: false,
            supports_phases: false,
        }),

        // Claude 4.6 series
        "claude-opus-4-6" => Some(LlmModelProfile {
            name: "Claude Opus 4.6".into(),
            family: "claude-opus-4-6".into(),
            description: None,
            release_date: Some("2026-02-05".into()),
            last_updated: Some("2026-02-05".into()),
            attachment: true,
            reasoning: true,
            temperature: true,
            knowledge: Some("2025-05-01".into()),
            tool_call: true,
            structured_output: true,
            open_weights: false,
            cost: Some(LlmModelCost {
                input: 5.00,
                output: 25.00,
                cache_read: Some(0.50),
                cost_tiers: vec![],
            }),
            limits: Some(LlmModelLimits {
                context: 1_000_000,
                input: None,
                output: 128_000,
                max_media: Some(600),
            }),
            modalities: Some(LlmModelModalities {
                input: vec![Modality::Text, Modality::Image],
                output: vec![Modality::Text],
            }),
            reasoning_effort: Some(reasoning_effort_anthropic_adaptive_thinking()),
            tool_search: false,
            supports_phases: false,
        }),

        "claude-sonnet-4-6" => Some(LlmModelProfile {
            name: "Claude Sonnet 4.6".into(),
            family: "claude-sonnet-4-6".into(),
            description: None,
            release_date: Some("2026-02-17".into()),
            last_updated: Some("2026-02-17".into()),
            attachment: true,
            reasoning: true,
            temperature: true,
            knowledge: Some("2025-08-01".into()),
            tool_call: true,
            structured_output: true,
            open_weights: false,
            cost: Some(LlmModelCost {
                input: 3.00,
                output: 15.00,
                cache_read: Some(0.30),
                cost_tiers: vec![],
            }),
            limits: Some(LlmModelLimits {
                context: 200_000,
                input: None,
                output: 64_000,
                max_media: None,
            }),
            modalities: Some(LlmModelModalities {
                input: vec![Modality::Text, Modality::Image],
                output: vec![Modality::Text],
            }),
            reasoning_effort: Some(reasoning_effort_anthropic_adaptive_thinking()),
            tool_search: false,
            supports_phases: false,
        }),

        // Claude 4.5 series
        "claude-opus-4-5" => Some(LlmModelProfile {
            name: "Claude Opus 4.5".into(),
            family: "claude-opus-4-5".into(),
            description: None,
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
                cost_tiers: vec![],
            }),
            limits: Some(LlmModelLimits {
                context: 200_000,
                input: None,
                output: 64_000,
                max_media: None,
            }),
            modalities: Some(LlmModelModalities {
                input: vec![Modality::Text, Modality::Image],
                output: vec![Modality::Text],
            }),
            reasoning_effort: Some(reasoning_effort_anthropic_extended_thinking()),
            tool_search: false,
            supports_phases: false,
        }),

        "claude-sonnet-4-5" => Some(LlmModelProfile {
            name: "Claude Sonnet 4.5".into(),
            family: "claude-sonnet-4-5".into(),
            description: None,
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
                cost_tiers: vec![],
            }),
            limits: Some(LlmModelLimits {
                context: 200_000,
                input: None,
                output: 64_000,
                max_media: None,
            }),
            modalities: Some(LlmModelModalities {
                input: vec![Modality::Text, Modality::Image],
                output: vec![Modality::Text],
            }),
            reasoning_effort: Some(reasoning_effort_anthropic_extended_thinking()),
            tool_search: false,
            supports_phases: false,
        }),

        "claude-haiku-4-5" => Some(LlmModelProfile {
            name: "Claude Haiku 4.5".into(),
            family: "claude-haiku-4-5".into(),
            description: None,
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
                cost_tiers: vec![],
            }),
            limits: Some(LlmModelLimits {
                context: 200_000,
                input: None,
                output: 16_000,
                max_media: None,
            }),
            modalities: Some(LlmModelModalities {
                input: vec![Modality::Text, Modality::Image],
                output: vec![Modality::Text],
            }),
            reasoning_effort: Some(reasoning_effort_anthropic_extended_thinking()),
            tool_search: false,
            supports_phases: false,
        }),

        // Claude 4.1 series
        "claude-opus-4-1" => Some(LlmModelProfile {
            name: "Claude Opus 4.1".into(),
            family: "claude-opus-4-1".into(),
            description: None,
            release_date: Some("2025-08-05".into()),
            last_updated: Some("2025-08-05".into()),
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
                cost_tiers: vec![],
            }),
            limits: Some(LlmModelLimits {
                context: 200_000,
                input: None,
                output: 32_000,
                max_media: None,
            }),
            modalities: Some(LlmModelModalities {
                input: vec![Modality::Text, Modality::Image],
                output: vec![Modality::Text],
            }),
            reasoning_effort: Some(reasoning_effort_anthropic_extended_thinking()),
            tool_search: false,
            supports_phases: false,
        }),

        // Claude 4 series
        "claude-sonnet-4" => Some(LlmModelProfile {
            name: "Claude Sonnet 4".into(),
            family: "claude-sonnet-4".into(),
            description: None,
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
                input: 3.00,
                output: 15.00,
                cache_read: Some(0.30),
                cost_tiers: vec![],
            }),
            limits: Some(LlmModelLimits {
                context: 200_000,
                input: None,
                output: 64_000,
                max_media: None,
            }),
            modalities: Some(LlmModelModalities {
                input: vec![Modality::Text, Modality::Image],
                output: vec![Modality::Text],
            }),
            reasoning_effort: Some(reasoning_effort_anthropic_extended_thinking()),
            tool_search: false,
            supports_phases: false,
        }),

        "claude-opus-4" => Some(LlmModelProfile {
            name: "Claude Opus 4".into(),
            family: "claude-opus-4".into(),
            description: None,
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
                cost_tiers: vec![],
            }),
            limits: Some(LlmModelLimits {
                context: 200_000,
                input: None,
                output: 32_000,
                max_media: None,
            }),
            modalities: Some(LlmModelModalities {
                input: vec![Modality::Text, Modality::Image],
                output: vec![Modality::Text],
            }),
            reasoning_effort: Some(reasoning_effort_anthropic_extended_thinking()),
            tool_search: false,
            supports_phases: false,
        }),

        // Claude 3.7 series
        "claude-3-7-sonnet" => Some(LlmModelProfile {
            name: "Claude 3.7 Sonnet".into(),
            family: "claude-3-7-sonnet".into(),
            description: None,
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
                cost_tiers: vec![],
            }),
            limits: Some(LlmModelLimits {
                context: 200_000,
                input: None,
                output: 64_000, // Extended output with thinking
                max_media: None,
            }),
            modalities: Some(LlmModelModalities {
                input: vec![Modality::Text, Modality::Image],
                output: vec![Modality::Text],
            }),
            reasoning_effort: Some(reasoning_effort_anthropic_extended_thinking()),
            tool_search: false,
            supports_phases: false,
        }),

        // Claude 3.5 series
        "claude-3-5-sonnet" => Some(LlmModelProfile {
            name: "Claude 3.5 Sonnet".into(),
            family: "claude-3-5-sonnet".into(),
            description: None,
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
                cost_tiers: vec![],
            }),
            limits: Some(LlmModelLimits {
                context: 200_000,
                input: None,
                output: 8_192,
                max_media: None,
            }),
            modalities: Some(LlmModelModalities {
                input: vec![Modality::Text, Modality::Image],
                output: vec![Modality::Text],
            }),
            reasoning_effort: None,
            tool_search: false,
            supports_phases: false,
        }),

        "claude-3-5-haiku" => Some(LlmModelProfile {
            name: "Claude 3.5 Haiku".into(),
            family: "claude-3-5-haiku".into(),
            description: None,
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
                cost_tiers: vec![],
            }),
            limits: Some(LlmModelLimits {
                context: 200_000,
                input: None,
                output: 8_192,
                max_media: None,
            }),
            modalities: Some(LlmModelModalities {
                input: vec![Modality::Text, Modality::Image],
                output: vec![Modality::Text],
            }),
            reasoning_effort: None,
            tool_search: false,
            supports_phases: false,
        }),

        "claude-3-opus" => Some(LlmModelProfile {
            name: "Claude 3 Opus".into(),
            family: "claude-3-opus".into(),
            description: None,
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
                cost_tiers: vec![],
            }),
            limits: Some(LlmModelLimits {
                context: 200_000,
                input: None,
                output: 4_096,
                max_media: None,
            }),
            modalities: Some(LlmModelModalities {
                input: vec![Modality::Text, Modality::Image],
                output: vec![Modality::Text],
            }),
            reasoning_effort: None,
            tool_search: false,
            supports_phases: false,
        }),

        "claude-3-sonnet" => Some(LlmModelProfile {
            name: "Claude 3 Sonnet".into(),
            family: "claude-3-sonnet".into(),
            description: None,
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
                cost_tiers: vec![],
            }),
            limits: Some(LlmModelLimits {
                context: 200_000,
                input: None,
                output: 4_096,
                max_media: None,
            }),
            modalities: Some(LlmModelModalities {
                input: vec![Modality::Text, Modality::Image],
                output: vec![Modality::Text],
            }),
            reasoning_effort: None,
            tool_search: false,
            supports_phases: false,
        }),

        "claude-3-haiku" => Some(LlmModelProfile {
            name: "Claude 3 Haiku".into(),
            family: "claude-3-haiku".into(),
            description: None,
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
                cost_tiers: vec![],
            }),
            limits: Some(LlmModelLimits {
                context: 200_000,
                input: None,
                output: 4_096,
                max_media: None,
            }),
            modalities: Some(LlmModelModalities {
                input: vec![Modality::Text, Modality::Image],
                output: vec![Modality::Text],
            }),
            reasoning_effort: None,
            tool_search: false,
            supports_phases: false,
        }),

        _ => None,
    }
}

/// Normalize Gemini model ID by extracting the base model name
/// e.g., "gemini-2.5-pro-preview-05-06" -> "gemini-2.5-pro"
fn normalize_gemini_model_id(model_id: &str) -> &str {
    let patterns = [
        "gemini-3.1-pro-preview",
        "gemini-2.5-flash",
        "gemini-2.5-pro",
        "gemini-2.0-flash",
        "gemini-1.5-pro",
        "gemini-1.5-flash",
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

/// Get Gemini model profile
fn get_gemini_profile(model_id: &str) -> Option<LlmModelProfile> {
    let base_id = normalize_gemini_model_id(model_id);

    match base_id {
        // Gemini 3.x series (newest). Source: models.dev (google provider).
        // `gemini-3-pro-preview` is deprecated upstream; 3.1 Pro Preview is the
        // current flagship Pro. Pricing has a >200K-token tier. Reasoning effort
        // (low/medium/high) is offered upstream but, consistent with the other
        // Gemini profiles here, effort selection is left unset.
        "gemini-3.1-pro-preview" => Some(LlmModelProfile {
            name: "Gemini 3.1 Pro Preview".into(),
            family: "gemini-3.1-pro-preview".into(),
            description: None,
            release_date: Some("2026-02-19".into()),
            last_updated: Some("2026-02-19".into()),
            attachment: true,
            reasoning: true,
            temperature: true,
            knowledge: Some("2025-01-01".into()),
            tool_call: true,
            structured_output: true,
            open_weights: false,
            cost: Some(LlmModelCost {
                input: 2.00,
                output: 12.00,
                cache_read: Some(0.20),
                cost_tiers: vec![CostTier {
                    above_tokens: 200_000,
                    input: 4.00,
                    output: 18.00,
                    cache_read: Some(0.40),
                }],
            }),
            limits: Some(LlmModelLimits {
                context: 1_048_576,
                input: None,
                output: 65_536,
                max_media: None,
            }),
            modalities: Some(LlmModelModalities {
                input: vec![
                    Modality::Text,
                    Modality::Image,
                    Modality::Audio,
                    Modality::Video,
                    Modality::Pdf,
                ],
                output: vec![Modality::Text],
            }),
            reasoning_effort: None,
            tool_search: false,
            supports_phases: false,
        }),

        "gemini-2.5-pro" => Some(LlmModelProfile {
            name: "Gemini 2.5 Pro".into(),
            family: "gemini-2.5-pro".into(),
            description: None,
            release_date: Some("2025-03-25".into()),
            last_updated: Some("2025-06-05".into()),
            attachment: true,
            reasoning: true,
            temperature: true,
            knowledge: Some("2025-03-01".into()),
            tool_call: true,
            structured_output: true,
            open_weights: false,
            cost: Some(LlmModelCost {
                input: 1.25,
                output: 10.00,
                cache_read: Some(0.31),
                cost_tiers: vec![],
            }),
            limits: Some(LlmModelLimits {
                context: 1_048_576,
                input: None,
                output: 65_536,
                max_media: None,
            }),
            modalities: Some(LlmModelModalities {
                input: vec![
                    Modality::Text,
                    Modality::Image,
                    Modality::Audio,
                    Modality::Video,
                ],
                output: vec![Modality::Text],
            }),
            reasoning_effort: None,
            tool_search: false,
            supports_phases: false,
        }),

        "gemini-2.5-flash" => Some(LlmModelProfile {
            name: "Gemini 2.5 Flash".into(),
            family: "gemini-2.5-flash".into(),
            description: None,
            release_date: Some("2025-04-17".into()),
            last_updated: Some("2025-06-12".into()),
            attachment: true,
            reasoning: true,
            temperature: true,
            knowledge: Some("2025-03-01".into()),
            tool_call: true,
            structured_output: true,
            open_weights: false,
            cost: Some(LlmModelCost {
                input: 0.15,
                output: 0.60,
                cache_read: Some(0.0375),
                cost_tiers: vec![],
            }),
            limits: Some(LlmModelLimits {
                context: 1_048_576,
                input: None,
                output: 65_536,
                max_media: None,
            }),
            modalities: Some(LlmModelModalities {
                input: vec![
                    Modality::Text,
                    Modality::Image,
                    Modality::Audio,
                    Modality::Video,
                ],
                output: vec![Modality::Text],
            }),
            reasoning_effort: None,
            tool_search: false,
            supports_phases: false,
        }),

        "gemini-2.0-flash" => Some(LlmModelProfile {
            name: "Gemini 2.0 Flash".into(),
            family: "gemini-2.0-flash".into(),
            description: None,
            release_date: Some("2025-02-05".into()),
            last_updated: Some("2025-02-05".into()),
            attachment: true,
            reasoning: false,
            temperature: true,
            knowledge: Some("2024-08-01".into()),
            tool_call: true,
            structured_output: true,
            open_weights: false,
            cost: Some(LlmModelCost {
                input: 0.10,
                output: 0.40,
                cache_read: Some(0.025),
                cost_tiers: vec![],
            }),
            limits: Some(LlmModelLimits {
                context: 1_048_576,
                input: None,
                output: 8_192,
                max_media: None,
            }),
            modalities: Some(LlmModelModalities {
                input: vec![
                    Modality::Text,
                    Modality::Image,
                    Modality::Audio,
                    Modality::Video,
                ],
                output: vec![Modality::Text, Modality::Image],
            }),
            reasoning_effort: None,
            tool_search: false,
            supports_phases: false,
        }),

        "gemini-1.5-pro" => Some(LlmModelProfile {
            name: "Gemini 1.5 Pro".into(),
            family: "gemini-1.5-pro".into(),
            description: None,
            release_date: Some("2024-02-15".into()),
            last_updated: Some("2024-09-24".into()),
            attachment: true,
            reasoning: false,
            temperature: true,
            knowledge: Some("2024-04-01".into()),
            tool_call: true,
            structured_output: true,
            open_weights: false,
            cost: Some(LlmModelCost {
                input: 1.25,
                output: 5.00,
                cache_read: Some(0.31),
                cost_tiers: vec![],
            }),
            limits: Some(LlmModelLimits {
                context: 2_097_152,
                input: None,
                output: 8_192,
                max_media: None,
            }),
            modalities: Some(LlmModelModalities {
                input: vec![
                    Modality::Text,
                    Modality::Image,
                    Modality::Audio,
                    Modality::Video,
                ],
                output: vec![Modality::Text],
            }),
            reasoning_effort: None,
            tool_search: false,
            supports_phases: false,
        }),

        "gemini-1.5-flash" => Some(LlmModelProfile {
            name: "Gemini 1.5 Flash".into(),
            family: "gemini-1.5-flash".into(),
            description: None,
            release_date: Some("2024-05-24".into()),
            last_updated: Some("2024-09-24".into()),
            attachment: true,
            reasoning: false,
            temperature: true,
            knowledge: Some("2024-04-01".into()),
            tool_call: true,
            structured_output: true,
            open_weights: false,
            cost: Some(LlmModelCost {
                input: 0.075,
                output: 0.30,
                cache_read: Some(0.01875),
                cost_tiers: vec![],
            }),
            limits: Some(LlmModelLimits {
                context: 1_048_576,
                input: None,
                output: 8_192,
                max_media: None,
            }),
            modalities: Some(LlmModelModalities {
                input: vec![
                    Modality::Text,
                    Modality::Image,
                    Modality::Audio,
                    Modality::Video,
                ],
                output: vec![Modality::Text],
            }),
            reasoning_effort: None,
            tool_search: false,
            supports_phases: false,
        }),

        _ => None,
    }
}

/// Get LlmSim model profile (simulated LLM for testing)
/// Profile is modeled close to GPT-5.2 for realistic testing
fn get_llmsim_profile(model_id: &str) -> Option<LlmModelProfile> {
    match model_id {
        "llmsim-default" | "llmsim" => Some(LlmModelProfile {
            name: "LlmSim Default".into(),
            family: "llmsim".into(),
            description: None,
            release_date: Some("2025-01-01".into()),
            last_updated: Some("2025-01-01".into()),
            attachment: true,
            reasoning: true,
            temperature: false, // Like gpt-5.2
            knowledge: Some("2025-08-31".into()),
            tool_call: true,
            structured_output: true,
            open_weights: false,
            cost: Some(LlmModelCost {
                input: 0.00, // Free for testing
                output: 0.00,
                cache_read: Some(0.00),
                cost_tiers: vec![],
            }),
            limits: Some(LlmModelLimits {
                context: 128_000,
                input: None,
                output: 64_000,
                max_media: None,
            }),
            modalities: Some(LlmModelModalities {
                input: vec![Modality::Text, Modality::Image],
                output: vec![Modality::Text],
            }),
            reasoning_effort: Some(reasoning_effort_gpt52()), // Same as GPT-5.2
            tool_search: false,
            supports_phases: false,
        }),
        _ => None,
    }
}

/// Normalize OpenAI model ID to base name
/// e.g., "gpt-4o-2024-11-20" -> "gpt-4o"
fn normalize_model_id(model_id: &str) -> &str {
    // Known base model patterns (order matters - more specific first)
    let patterns = [
        // Realtime models
        "gpt-realtime-2",
        // GPT-5.5 models
        "gpt-5.5-pro",
        "gpt-5.5",
        // GPT-5.4 models
        "gpt-5.4-pro",
        "gpt-5.4-nano",
        "gpt-5.4-mini",
        "gpt-5.4",
        // GPT-5.3 models
        "gpt-5.3-codex",
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
        // Claude 4.8
        "claude-opus-4-8",
        // Claude 4.7 / 4.6
        "claude-opus-4-7",
        "claude-opus-4-6",
        "claude-sonnet-4-6",
        // Claude 4.5 series
        "claude-opus-4-5",
        "claude-sonnet-4-5",
        "claude-haiku-4-5",
        // Claude 4.1 series
        "claude-opus-4-1",
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
    fn test_openai_completions_uses_openai_profiles() {
        let profile = get_model_profile(&LlmProviderType::OpenaiCompletions, "gpt-4o");
        assert!(profile.is_some());
        assert_eq!(profile.unwrap().name, "GPT-4o");
    }

    #[test]
    fn test_azure_openai_uses_openai_profiles() {
        let profile = get_model_profile(&LlmProviderType::AzureOpenai, "gpt-4o");
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
        assert!(
            !effort
                .values
                .iter()
                .any(|v| v.value == ReasoningEffort::None)
        );
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
        assert!(
            effort
                .values
                .iter()
                .any(|v| v.value == ReasoningEffort::None)
        );
        assert!(
            !effort
                .values
                .iter()
                .any(|v| v.value == ReasoningEffort::Xhigh)
        );
    }

    #[test]
    fn test_gpt51_codex_max_profile() {
        let profile = get_model_profile(&LlmProviderType::Openai, "gpt-5.1-codex-max").unwrap();
        assert_eq!(profile.name, "GPT-5.1 Codex max");
        assert!(profile.reasoning);

        // After gpt-5.1-codex-max: supports xhigh
        let effort = profile.reasoning_effort.unwrap();
        assert!(
            effort
                .values
                .iter()
                .any(|v| v.value == ReasoningEffort::Xhigh)
        );
    }

    #[test]
    fn test_gpt52_profile() {
        let profile = get_model_profile(&LlmProviderType::Openai, "gpt-5.2").unwrap();
        assert_eq!(profile.name, "GPT-5.2");
        assert!(profile.reasoning);

        let limits = profile.limits.unwrap();
        assert_eq!(limits.context, 400_000);
        assert_eq!(limits.output, 128_000);

        let cost = profile.cost.unwrap();
        assert!((cost.input - 1.75).abs() < f64::EPSILON);
        assert!((cost.output - 14.00).abs() < f64::EPSILON);

        // gpt-5.2: default none, supports none/low/medium/high/xhigh
        let effort = profile.reasoning_effort.unwrap();
        assert_eq!(effort.default, ReasoningEffort::None);
        assert_eq!(effort.values.len(), 5);
    }

    #[test]
    fn test_gpt52_pro_profile() {
        let profile = get_model_profile(&LlmProviderType::Openai, "gpt-5.2-pro").unwrap();
        assert_eq!(profile.name, "GPT-5.2 Pro");
        assert!(profile.reasoning);

        let limits = profile.limits.unwrap();
        assert_eq!(limits.context, 400_000);
        assert_eq!(limits.output, 128_000);

        let cost = profile.cost.unwrap();
        assert!((cost.input - 21.00).abs() < f64::EPSILON);
        assert!((cost.output - 168.00).abs() < f64::EPSILON);

        // gpt-5.2-pro: default medium, supports medium/high/xhigh
        let effort = profile.reasoning_effort.unwrap();
        assert_eq!(effort.default, ReasoningEffort::Medium);
        assert_eq!(effort.values.len(), 3);
    }

    #[test]
    fn test_gpt53_codex_profile() {
        let profile = get_model_profile(&LlmProviderType::Openai, "gpt-5.3-codex").unwrap();
        assert_eq!(profile.name, "GPT-5.3 Codex");
        assert!(profile.reasoning);
        assert!(profile.tool_call);

        let limits = profile.limits.unwrap();
        assert_eq!(limits.context, 400_000);
        assert_eq!(limits.output, 128_000);

        let cost = profile.cost.unwrap();
        assert!((cost.input - 1.75).abs() < f64::EPSILON);
        assert!((cost.output - 14.00).abs() < f64::EPSILON);
    }

    #[test]
    fn test_gpt55_profile() {
        let profile = get_model_profile(&LlmProviderType::Openai, "gpt-5.5").unwrap();
        assert_eq!(profile.name, "GPT-5.5");
        assert_eq!(profile.family, "gpt-5.5");
        assert!(profile.reasoning);
        assert!(profile.tool_call);
        assert!(profile.structured_output);
        // tool_search is gated off on gpt-5.5 pending live round-trip verification (EVE-521).
        assert!(!profile.tool_search);
        assert!(profile.supports_phases);

        let limits = profile.limits.unwrap();
        assert_eq!(limits.context, 1_050_000);
        assert_eq!(limits.output, 128_000);

        let cost = profile.cost.unwrap();
        assert!((cost.input - 5.00).abs() < f64::EPSILON);
        assert!((cost.output - 30.00).abs() < f64::EPSILON);
        assert!((cost.cache_read.unwrap() - 0.50).abs() < f64::EPSILON);
        assert!(cost.cost_tiers.is_empty()); // Flat pricing, no 200K tier

        // gpt-5.5: default medium, supports none/low/medium/high/xhigh
        let effort = profile.reasoning_effort.unwrap();
        assert_eq!(effort.default, ReasoningEffort::Medium);
        assert_eq!(effort.values.len(), 5);
    }

    #[test]
    fn test_gpt_realtime_2_profile() {
        let profile = get_model_profile(&LlmProviderType::Openai, "gpt-realtime-2").unwrap();
        assert_eq!(profile.name, "GPT Realtime 2");
        assert_eq!(profile.family, "gpt-realtime");
        assert!(profile.reasoning);
        assert!(profile.tool_call);
        assert!(profile.supports_phases);

        let modalities = profile.modalities.unwrap();
        assert!(modalities.input.contains(&Modality::Audio));
        assert!(modalities.output.contains(&Modality::Audio));

        let effort = profile.reasoning_effort.unwrap();
        assert_eq!(effort.default, ReasoningEffort::Low);
        assert!(
            effort
                .values
                .iter()
                .any(|v| v.value == ReasoningEffort::Minimal)
        );
        assert!(
            effort
                .values
                .iter()
                .any(|v| v.value == ReasoningEffort::Xhigh)
        );
    }

    #[test]
    fn test_gpt55_pro_profile() {
        let profile = get_model_profile(&LlmProviderType::Openai, "gpt-5.5-pro").unwrap();
        assert_eq!(profile.name, "GPT-5.5 Pro");
        assert_eq!(profile.family, "gpt-5.5-pro");
        assert!(profile.reasoning);
        assert!(profile.tool_call);
        assert!(profile.structured_output);
        // tool_search gated off with the rest of the gpt-5.5 family (EVE-521).
        assert!(!profile.tool_search);
        assert!(profile.supports_phases);

        let cost = profile.cost.unwrap();
        assert!((cost.input - 30.00).abs() < f64::EPSILON);
        assert!((cost.output - 180.00).abs() < f64::EPSILON);
        assert!(cost.cache_read.is_none());
        assert!(cost.cost_tiers.is_empty());

        // gpt-5.5-pro: default medium, supports medium/high/xhigh
        let effort = profile.reasoning_effort.unwrap();
        assert_eq!(effort.default, ReasoningEffort::Medium);
        assert_eq!(effort.values.len(), 3);
    }

    #[test]
    fn test_gpt55_versioned() {
        let profile = get_model_profile(&LlmProviderType::Openai, "gpt-5.5-2026-04-23").unwrap();
        assert_eq!(profile.name, "GPT-5.5");

        let pro = get_model_profile(&LlmProviderType::Openai, "gpt-5.5-pro-2026-04-23").unwrap();
        assert_eq!(pro.name, "GPT-5.5 Pro");
    }

    #[test]
    fn test_normalize_gpt55_model_ids() {
        assert_eq!(normalize_model_id("gpt-5.5"), "gpt-5.5");
        assert_eq!(normalize_model_id("gpt-5.5-2026-04-23"), "gpt-5.5");
        assert_eq!(normalize_model_id("gpt-5.5-pro"), "gpt-5.5-pro");
        assert_eq!(normalize_model_id("gpt-5.5-pro-2026-04-23"), "gpt-5.5-pro");
    }

    #[test]
    fn test_gpt54_profile() {
        let profile = get_model_profile(&LlmProviderType::Openai, "gpt-5.4").unwrap();
        assert_eq!(profile.name, "GPT-5.4");
        assert_eq!(profile.family, "gpt-5.4");
        assert!(profile.reasoning);
        assert!(profile.tool_call);
        assert!(profile.structured_output);

        let limits = profile.limits.unwrap();
        assert_eq!(limits.context, 1_050_000);
        assert_eq!(limits.output, 128_000);

        let cost = profile.cost.unwrap();
        assert!((cost.input - 2.50).abs() < f64::EPSILON);
        assert!((cost.output - 15.00).abs() < f64::EPSILON);
        assert!((cost.cache_read.unwrap() - 0.25).abs() < f64::EPSILON);

        // Tiered pricing above 200K tokens
        assert_eq!(cost.cost_tiers.len(), 1);
        let tier = &cost.cost_tiers[0];
        assert_eq!(tier.above_tokens, 200_000);
        assert!((tier.input - 5.00).abs() < f64::EPSILON);
        assert!((tier.output - 22.50).abs() < f64::EPSILON);
        assert!((tier.cache_read.unwrap() - 0.50).abs() < f64::EPSILON);

        assert!(profile.description.is_some());
        assert!(profile.supports_phases);
        assert!(profile.tool_search);

        // gpt-5.4: default none, supports none/low/medium/high/xhigh
        let effort = profile.reasoning_effort.unwrap();
        assert_eq!(effort.default, ReasoningEffort::None);
        assert_eq!(effort.values.len(), 5);
    }

    #[test]
    fn test_gpt54_mini_profile() {
        let profile = get_model_profile(&LlmProviderType::Openai, "gpt-5.4-mini").unwrap();
        assert_eq!(profile.name, "GPT-5.4 mini");
        assert_eq!(profile.family, "gpt-5.4-mini");
        assert!(profile.reasoning);
        assert!(profile.tool_call);
        assert!(profile.structured_output);
        assert!(profile.tool_search);
        assert!(profile.supports_phases);
        assert!(profile.description.is_some());

        let limits = profile.limits.unwrap();
        assert_eq!(limits.context, 400_000);
        assert_eq!(limits.output, 128_000);

        let cost = profile.cost.unwrap();
        assert!((cost.input - 0.75).abs() < f64::EPSILON);
        assert!((cost.output - 4.50).abs() < f64::EPSILON);
        assert!((cost.cache_read.unwrap() - 0.075).abs() < f64::EPSILON);
        assert!(cost.cost_tiers.is_empty()); // No tiered pricing
    }

    #[test]
    fn test_gpt54_nano_profile() {
        let profile = get_model_profile(&LlmProviderType::Openai, "gpt-5.4-nano").unwrap();
        assert_eq!(profile.name, "GPT-5.4 nano");
        assert_eq!(profile.family, "gpt-5.4-nano");
        assert!(profile.reasoning);
        assert!(profile.tool_call);
        assert!(profile.tool_search);
        assert!(profile.supports_phases);
        assert!(profile.description.is_some());

        let limits = profile.limits.unwrap();
        assert_eq!(limits.context, 400_000);
        assert_eq!(limits.output, 128_000);

        let cost = profile.cost.unwrap();
        assert!((cost.input - 0.20).abs() < f64::EPSILON);
        assert!((cost.output - 1.25).abs() < f64::EPSILON);
        assert!((cost.cache_read.unwrap() - 0.02).abs() < f64::EPSILON);
        assert!(cost.cost_tiers.is_empty()); // No tiered pricing
    }

    #[test]
    fn test_gpt54_pro_profile() {
        let profile = get_model_profile(&LlmProviderType::Openai, "gpt-5.4-pro").unwrap();
        assert_eq!(profile.name, "GPT-5.4 Pro");
        assert_eq!(profile.family, "gpt-5.4-pro");
        assert!(profile.reasoning);
        assert!(profile.tool_call);
        assert!(!profile.structured_output); // Not supported for pro
        assert!(profile.description.is_some());

        let limits = profile.limits.unwrap();
        assert_eq!(limits.context, 1_050_000);
        assert_eq!(limits.output, 128_000);

        let cost = profile.cost.unwrap();
        assert!((cost.input - 30.00).abs() < f64::EPSILON);
        assert!((cost.output - 180.00).abs() < f64::EPSILON);
        assert!(cost.cache_read.is_none());

        // Tiered pricing above 200K tokens
        assert_eq!(cost.cost_tiers.len(), 1);
        let tier = &cost.cost_tiers[0];
        assert_eq!(tier.above_tokens, 200_000);
        assert!((tier.input - 60.00).abs() < f64::EPSILON);
        assert!((tier.output - 270.00).abs() < f64::EPSILON);

        assert!(profile.supports_phases);

        // gpt-5.4-pro: default medium, supports medium/high/xhigh
        let effort = profile.reasoning_effort.unwrap();
        assert_eq!(effort.default, ReasoningEffort::Medium);
        assert_eq!(effort.values.len(), 3);
    }

    #[test]
    fn test_gpt54_versioned() {
        let profile = get_model_profile(&LlmProviderType::Openai, "gpt-5.4-2026-03-05").unwrap();
        assert_eq!(profile.name, "GPT-5.4");
    }

    #[test]
    fn test_gpt54_mini_versioned() {
        let profile =
            get_model_profile(&LlmProviderType::Openai, "gpt-5.4-mini-2026-03-17").unwrap();
        assert_eq!(profile.name, "GPT-5.4 mini");
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
        assert_eq!(normalize_model_id("gpt-5.3-codex"), "gpt-5.3-codex");
        assert_eq!(normalize_model_id("gpt-5.4"), "gpt-5.4");
        assert_eq!(normalize_model_id("gpt-5.4-2026-03-05"), "gpt-5.4");
        assert_eq!(normalize_model_id("gpt-5.4-pro"), "gpt-5.4-pro");
        assert_eq!(normalize_model_id("gpt-5.4-mini"), "gpt-5.4-mini");
        assert_eq!(
            normalize_model_id("gpt-5.4-mini-2026-03-17"),
            "gpt-5.4-mini"
        );
        assert_eq!(normalize_model_id("gpt-5.4-nano"), "gpt-5.4-nano");
        assert_eq!(
            normalize_model_id("gpt-5.4-nano-2026-03-17"),
            "gpt-5.4-nano"
        );
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

    // Claude 4.7 / 4.6 model tests

    #[test]
    fn test_claude_opus_47_profile() {
        let profile = get_model_profile(&LlmProviderType::Anthropic, "claude-opus-4-7").unwrap();
        assert_eq!(profile.name, "Claude Opus 4.7");
        assert_eq!(profile.family, "claude-opus-4-7");
        assert!(profile.reasoning);
        assert!(profile.tool_call);
        assert!(profile.temperature);
        assert!(profile.structured_output);

        let limits = profile.limits.unwrap();
        assert_eq!(limits.context, 1_000_000);
        assert_eq!(limits.output, 128_000);
        assert_eq!(limits.max_media, None);

        let cost = profile.cost.unwrap();
        assert!((cost.input - 5.00).abs() < f64::EPSILON);
        assert!((cost.output - 25.00).abs() < f64::EPSILON);

        let modalities = profile.modalities.unwrap();
        assert_eq!(
            modalities.input,
            vec![Modality::Text, Modality::Image, Modality::Pdf]
        );

        // Adaptive thinking: default high, supports low/medium/high/max(xhigh)
        let effort = profile.reasoning_effort.unwrap();
        assert_eq!(effort.default, ReasoningEffort::High);
        assert_eq!(effort.values.len(), 4);
        assert!(
            effort
                .values
                .iter()
                .any(|v| v.value == ReasoningEffort::Xhigh)
        );
    }

    #[test]
    fn test_claude_opus_46_profile() {
        let profile = get_model_profile(&LlmProviderType::Anthropic, "claude-opus-4-6").unwrap();
        assert_eq!(profile.name, "Claude Opus 4.6");
        assert_eq!(profile.family, "claude-opus-4-6");
        assert!(profile.reasoning);
        assert!(profile.tool_call);
        assert!(profile.temperature);
        assert!(profile.structured_output);

        let limits = profile.limits.unwrap();
        assert_eq!(limits.context, 1_000_000);
        assert_eq!(limits.output, 128_000);
        assert_eq!(limits.max_media, Some(600));

        let cost = profile.cost.unwrap();
        assert!((cost.input - 5.00).abs() < f64::EPSILON);
        assert!((cost.output - 25.00).abs() < f64::EPSILON);

        let modalities = profile.modalities.unwrap();
        assert_eq!(modalities.input, vec![Modality::Text, Modality::Image]);

        let effort = profile.reasoning_effort.unwrap();
        assert_eq!(effort.default, ReasoningEffort::High);
        assert_eq!(effort.values.len(), 4);
    }

    #[test]
    fn test_claude_sonnet_46_profile() {
        let profile = get_model_profile(&LlmProviderType::Anthropic, "claude-sonnet-4-6").unwrap();
        assert_eq!(profile.name, "Claude Sonnet 4.6");
        assert_eq!(profile.family, "claude-sonnet-4-6");
        assert!(profile.reasoning);
        assert!(profile.tool_call);
        assert!(profile.structured_output);

        let limits = profile.limits.unwrap();
        assert_eq!(limits.context, 200_000);
        assert_eq!(limits.output, 64_000);

        let cost = profile.cost.unwrap();
        assert!((cost.input - 3.00).abs() < f64::EPSILON);
        assert!((cost.output - 15.00).abs() < f64::EPSILON);

        // Adaptive thinking: default high, supports low/medium/high/max(xhigh)
        let effort = profile.reasoning_effort.unwrap();
        assert_eq!(effort.default, ReasoningEffort::High);
        assert_eq!(effort.values.len(), 4);
        assert!(
            effort
                .values
                .iter()
                .any(|v| v.value == ReasoningEffort::Xhigh)
        );
    }

    #[test]
    fn test_claude_opus_47_versioned() {
        let profile =
            get_model_profile(&LlmProviderType::Anthropic, "claude-opus-4-7-20260416").unwrap();
        assert_eq!(profile.name, "Claude Opus 4.7");
    }

    #[test]
    fn test_claude_sonnet_46_versioned() {
        let profile =
            get_model_profile(&LlmProviderType::Anthropic, "claude-sonnet-4-6-20260217").unwrap();
        assert_eq!(profile.name, "Claude Sonnet 4.6");
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

    // Claude 4.1 model tests

    #[test]
    fn test_claude_opus_41_profile() {
        let profile =
            get_model_profile(&LlmProviderType::Anthropic, "claude-opus-4-1-20250805").unwrap();
        assert_eq!(profile.name, "Claude Opus 4.1");
        assert_eq!(profile.family, "claude-opus-4-1");
        assert!(profile.reasoning);
        assert!(profile.tool_call);

        let limits = profile.limits.unwrap();
        assert_eq!(limits.context, 200_000);
        assert_eq!(limits.output, 32_000);

        let cost = profile.cost.unwrap();
        assert!((cost.input - 15.00).abs() < f64::EPSILON);
        assert!((cost.output - 75.00).abs() < f64::EPSILON);
    }

    // Claude Sonnet 4 updated output limit

    #[test]
    fn test_claude_sonnet_4_output_limit() {
        let profile =
            get_model_profile(&LlmProviderType::Anthropic, "claude-sonnet-4-20250514").unwrap();
        assert_eq!(profile.name, "Claude Sonnet 4");
        let limits = profile.limits.unwrap();
        assert_eq!(limits.output, 64_000);
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
    fn test_normalize_claude_47_and_46_model_ids() {
        assert_eq!(
            normalize_anthropic_model_id("claude-opus-4-7"),
            "claude-opus-4-7"
        );
        assert_eq!(
            normalize_anthropic_model_id("claude-opus-4-7-20260416"),
            "claude-opus-4-7"
        );
        assert_eq!(
            normalize_anthropic_model_id("claude-opus-4-6"),
            "claude-opus-4-6"
        );
        assert_eq!(
            normalize_anthropic_model_id("claude-opus-4-6-20260205"),
            "claude-opus-4-6"
        );
        assert_eq!(
            normalize_anthropic_model_id("claude-sonnet-4-6"),
            "claude-sonnet-4-6"
        );
        assert_eq!(
            normalize_anthropic_model_id("claude-sonnet-4-6-20260217"),
            "claude-sonnet-4-6"
        );
    }

    #[test]
    fn test_normalize_claude_41_model_ids() {
        assert_eq!(
            normalize_anthropic_model_id("claude-opus-4-1"),
            "claude-opus-4-1"
        );
        assert_eq!(
            normalize_anthropic_model_id("claude-opus-4-1-20250805"),
            "claude-opus-4-1"
        );
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

    // Gemini model tests

    #[test]
    fn test_gemini_25_pro_profile() {
        let profile = get_model_profile(&LlmProviderType::Gemini, "gemini-2.5-pro").unwrap();
        assert_eq!(profile.name, "Gemini 2.5 Pro");
        assert!(profile.reasoning);
        assert!(profile.tool_call);
        assert!(profile.structured_output);

        let limits = profile.limits.unwrap();
        assert_eq!(limits.context, 1_048_576);
        assert!(limits.input.is_none());
    }

    #[test]
    fn test_gemini_25_flash_profile() {
        let profile = get_model_profile(&LlmProviderType::Gemini, "gemini-2.5-flash").unwrap();
        assert_eq!(profile.name, "Gemini 2.5 Flash");
        assert!(profile.reasoning);
        assert!(profile.tool_call);
    }

    #[test]
    fn test_gemini_20_flash_profile() {
        let profile = get_model_profile(&LlmProviderType::Gemini, "gemini-2.0-flash").unwrap();
        assert_eq!(profile.name, "Gemini 2.0 Flash");
        assert!(!profile.reasoning);
        assert!(profile.tool_call);
    }

    #[test]
    fn test_normalize_gemini_model_ids() {
        assert_eq!(
            normalize_gemini_model_id("gemini-2.5-pro"),
            "gemini-2.5-pro"
        );
        assert_eq!(
            normalize_gemini_model_id("gemini-2.5-pro-preview-05-06"),
            "gemini-2.5-pro"
        );
        assert_eq!(
            normalize_gemini_model_id("gemini-2.5-flash-preview-04-17"),
            "gemini-2.5-flash"
        );
        assert_eq!(
            normalize_gemini_model_id("gemini-2.0-flash"),
            "gemini-2.0-flash"
        );
    }

    #[test]
    fn test_gemini_unknown_model() {
        let profile = get_model_profile(&LlmProviderType::Gemini, "unknown-model");
        assert!(profile.is_none());
    }

    // Newly added flagship model profiles

    #[test]
    fn test_claude_opus_4_8_profile() {
        let profile = get_model_profile(&LlmProviderType::Anthropic, "claude-opus-4-8").unwrap();
        assert_eq!(profile.name, "Claude Opus 4.8");
        assert_eq!(profile.family, "claude-opus-4-8");
        assert!(profile.reasoning);
        // Opus 4.8 removed sampling parameters (temperature returns 400).
        assert!(!profile.temperature);
        assert_eq!(profile.limits.as_ref().unwrap().context, 1_000_000);
        assert!(profile.reasoning_effort.is_some());
    }

    #[test]
    fn test_gemini_3_1_pro_preview_profile() {
        let profile =
            get_model_profile(&LlmProviderType::Gemini, "gemini-3.1-pro-preview").unwrap();
        assert_eq!(profile.name, "Gemini 3.1 Pro Preview");
        assert!(profile.reasoning);
        // >200K-token pricing tier.
        let cost = profile.cost.as_ref().unwrap();
        assert_eq!(cost.cost_tiers.len(), 1);
        assert_eq!(cost.cost_tiers[0].above_tokens, 200_000);
        assert_eq!(cost.cost_tiers[0].input, 4.00);
    }

    #[test]
    fn test_gemini_3_1_pro_preview_normalizes_dated_suffix() {
        let profile =
            get_model_profile(&LlmProviderType::Gemini, "gemini-3.1-pro-preview-02-19").unwrap();
        assert_eq!(profile.family, "gemini-3.1-pro-preview");
    }

    #[test]
    fn test_third_party_profiles_via_openai_completions() {
        // Bare ids and common vendor-prefixed aliases both resolve.
        let cases = [
            ("nemotron-3-super-120b-a12b", "Nemotron 3 Super"),
            ("nvidia/nemotron-3-super-120b-a12b", "Nemotron 3 Super"),
            ("qwen3.7-max", "Qwen3.7 Max"),
            ("MAI-1-preview", "MAI-1-preview"),
            ("MiniMax-M3", "MiniMax-M3"),
            ("kimi-k2-thinking", "Kimi K2 Thinking"),
            ("grok-4.3", "Grok 4.3"),
            ("x-ai/grok-4.3", "Grok 4.3"),
        ];
        for (id, name) in cases {
            let profile = get_model_profile(&LlmProviderType::OpenaiCompletions, id)
                .unwrap_or_else(|| panic!("missing profile for {id}"));
            assert_eq!(profile.name, name, "wrong profile for {id}");
        }
    }

    #[test]
    fn test_grok_4_3_has_context_tier() {
        let profile = get_model_profile(&LlmProviderType::OpenaiCompletions, "grok-4.3").unwrap();
        let cost = profile.cost.as_ref().unwrap();
        assert_eq!(cost.cost_tiers.len(), 1);
        assert_eq!(cost.cost_tiers[0].above_tokens, 200_000);
    }

    #[test]
    fn test_mai_preview_has_no_cost_or_limits() {
        // Microsoft never published pricing/limits for MAI-1-preview.
        let profile =
            get_model_profile(&LlmProviderType::OpenaiCompletions, "MAI-1-preview").unwrap();
        assert!(profile.cost.is_none());
        assert!(profile.limits.is_none());
        assert!(!profile.reasoning);
    }

    #[test]
    fn test_third_party_unknown_still_none() {
        let profile = get_model_profile(&LlmProviderType::OpenaiCompletions, "totally-made-up");
        assert!(profile.is_none());
    }
}
