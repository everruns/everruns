//! Each builtin provider declares its curated recommended models on its own
//! descriptor (`recommended_models.rs` holds only the generic contract).
//!
//! To refresh a provider's picks, edit that driver's `*_RECOMMENDED` const.

#![cfg_attr(test, allow(clippy::unwrap_used, clippy::expect_used))]

use everruns_contracts::{DriverId, DriverRegistry};
use everruns_drivers::register_drivers;

// Note: LlmSim lives in the separate `everruns-llmsim` crate (the drivers crate
// cannot depend on it without a dependency cycle); its descriptor coverage is
// asserted by that crate's own `test_recommended_models_cover_all_roles` test.
fn registered() -> DriverRegistry {
    let mut registry = DriverRegistry::default();
    register_drivers(&mut registry);
    registry
}

fn builtin_providers() -> [DriverId; 13] {
    [
        DriverId::OpenAI,
        DriverId::OpenRouter,
        DriverId::AzureOpenAI,
        DriverId::OpenAICompletions,
        DriverId::Anthropic,
        DriverId::Gemini,
        DriverId::Bedrock,
        DriverId::Mai,
        DriverId::Fireworks,
        DriverId::Meta,
        DriverId::Cloudflare,
        DriverId::Vercel,
        DriverId::Mistral,
    ]
}

#[test]
fn covers_every_builtin_provider() {
    let registry = registered();
    for provider in builtin_providers() {
        let entry = registry
            .recommended_models(&provider)
            .unwrap_or_else(|| panic!("missing recommendations for {}", provider.as_str()));
        assert_eq!(entry.provider, provider.as_str());
        assert!(
            !entry.recommended_models.is_empty(),
            "{}: recommended list must not be empty",
            entry.provider
        );
    }
}

#[test]
fn role_defaults_are_subset_of_recommended() {
    let registry = registered();
    for provider in builtin_providers() {
        let entry = registry.recommended_models(&provider).expect("covered");
        for role in [
            entry.coding_model,
            entry.research_model,
            entry.chat_model,
            entry.small_model,
            entry.decision_model,
        ] {
            assert!(
                entry.recommended_models.contains(&role),
                "{}: role default {role} not in recommended list",
                entry.provider
            );
        }
    }
}

#[test]
fn spot_checks_match_requested_defaults() {
    // Anchors from the product request; when vendors rotate flagships, update the
    // owning driver's const and these anchors together.
    let registry = registered();
    let openai = registry
        .recommended_models(&DriverId::OpenAI)
        .expect("openai");
    assert_eq!(openai.coding_model, "gpt-6.1-sol");
    assert_eq!(openai.research_model, "gpt-6-astra");
    let anthropic = registry
        .recommended_models(&DriverId::Anthropic)
        .expect("anthropic");
    assert_eq!(anthropic.coding_model, "claude-opus-5-5");
    assert_eq!(anthropic.research_model, "claude-fable-5-1");
    let openrouter = registry
        .recommended_models(&DriverId::OpenRouter)
        .expect("openrouter");
    assert_eq!(openrouter.coding_model, "muse-spark-1.3-contributor");
}
