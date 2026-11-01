//! Curated model recommendations for the Anthropic provider.
//!
//! Refresh when the vendor rotates flagships; keep every role default inside
//! `recommended_models`. Lives here (not in `driver.rs`) because that file is
//! frozen by the file-size guard: allowlisted files over 1500 lines may not grow.

/// Curated model recommendations for Anthropic.
pub const ANTHROPIC_RECOMMENDED: everruns_contracts::recommended_models::ProviderRecommendedModels =
    everruns_contracts::recommended_models::ProviderRecommendedModels {
        provider: "anthropic",
        recommended_models: &[
            "claude-opus-5-5",
            "claude-sonnet-5-5",
            "claude-fable-5-1",
            "claude-haiku-5-5",
        ],
        coding_model: "claude-opus-5-5",
        research_model: "claude-fable-5-1",
        chat_model: "claude-sonnet-5-5",
        small_model: "claude-haiku-5-5",
        decision_model: "claude-haiku-5-5",
    };
