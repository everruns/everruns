use super::*;
use crate::model_profile_data::types::Speed;

#[test]
fn versioned_and_canonical_aliases_resolve_to_the_same_profile() {
    for (provider, wire_id, canonical) in [
        ("anthropic", "claude-haiku-4-5-20251001", "claude-haiku-4-5"),
        ("anthropic", "claude-opus-4-5-20251101", "claude-opus-4-5"),
        ("anthropic", "claude-opus-4-6", "claude-opus-4-6"),
        ("anthropic", "claude-opus-4-6-20260205", "claude-opus-4-6"),
        ("anthropic", "claude-opus-4-7", "claude-opus-4-7"),
        ("anthropic", "claude-opus-4-7-20260416", "claude-opus-4-7"),
        ("anthropic", "claude-sonnet-4-6", "claude-sonnet-4-6"),
        (
            "anthropic",
            "claude-sonnet-4-6-20260217",
            "claude-sonnet-4-6",
        ),
        ("anthropic", "claude-sonnet-5", "claude-sonnet-5"),
        ("anthropic", "claude-sonnet-5-latest", "claude-sonnet-5"),
        ("anthropic", "claude-sonnet-5-5", "claude-sonnet-5-5"),
        ("anthropic", "claude-sonnet-5-5-latest", "claude-sonnet-5-5"),
        ("anthropic", "claude-haiku-5-5", "claude-haiku-5-5"),
        ("gemini", "gemini-2.0-flash", "gemini-2.0-flash"),
        (
            "gemini",
            "gemini-2.5-flash-preview-04-17",
            "gemini-2.5-flash",
        ),
        ("gemini", "gemini-2.5-pro", "gemini-2.5-pro"),
        ("gemini", "gemini-2.5-pro-preview-05-06", "gemini-2.5-pro"),
        (
            "gemini",
            "gemini-3.1-pro-preview-02-19",
            "gemini-3.1-pro-preview",
        ),
        ("openai", "gpt-4.1", "gpt-4.1"),
        ("openai", "gpt-4.1-2025-04-14", "gpt-4.1"),
        ("openai", "gpt-4.1-mini", "gpt-4.1-mini"),
        ("openai", "gpt-4.1-nano", "gpt-4.1-nano"),
        ("openai", "gpt-5", "gpt-5"),
        ("openai", "gpt-5-2025-08-07", "gpt-5"),
        ("openai", "gpt-5-codex", "gpt-5-codex"),
        ("openai", "gpt-5-mini", "gpt-5-mini"),
        ("openai", "gpt-5-nano", "gpt-5-nano"),
        ("openai", "gpt-5-pro", "gpt-5-pro"),
        ("openai", "gpt-5.1", "gpt-5.1"),
        ("openai", "gpt-5.1-codex", "gpt-5.1-codex"),
        ("openai", "gpt-5.1-codex-max", "gpt-5.1-codex-max"),
        ("openai", "gpt-5.1-codex-mini", "gpt-5.1-codex-mini"),
        ("openai", "gpt-5.2", "gpt-5.2"),
        ("openai", "gpt-5.2-2025-12-11", "gpt-5.2"),
        ("openai", "gpt-5.2-codex", "gpt-5.2-codex"),
        ("openai", "gpt-5.2-pro", "gpt-5.2-pro"),
        ("openai", "gpt-5.3-codex", "gpt-5.3-codex"),
        ("openai", "gpt-5.4", "gpt-5.4"),
        ("openai", "gpt-5.4-2026-03-05", "gpt-5.4"),
        ("openai", "gpt-5.4-mini", "gpt-5.4-mini"),
        ("openai", "gpt-5.4-mini-2026-03-17", "gpt-5.4-mini"),
        ("openai", "gpt-5.4-nano", "gpt-5.4-nano"),
        ("openai", "gpt-5.4-nano-2026-03-17", "gpt-5.4-nano"),
        ("openai", "gpt-5.4-pro", "gpt-5.4-pro"),
        ("openai", "gpt-5.5", "gpt-5.5"),
        ("openai", "gpt-5.5-2026-04-23", "gpt-5.5"),
        ("openai", "gpt-5.5-pro", "gpt-5.5-pro"),
        ("openai", "gpt-5.5-pro-2026-04-23", "gpt-5.5-pro"),
        ("openai", "gpt-5.6-luna", "gpt-5.6-luna"),
        ("openai", "gpt-5.6-sol-2026-07-09", "gpt-5.6-sol"),
        ("openai", "gpt-6-astra", "gpt-6-astra"),
        ("openai", "gpt-6-astra-2026-09-04", "gpt-6-astra"),
        ("openai", "gpt-6-sol", "gpt-6-sol"),
        ("openai", "gpt-6-sol-2026-09-22", "gpt-6-sol"),
        ("openai", "gpt-6-luna", "gpt-6-luna"),
        ("openai", "gpt-6-luna-2026-09-22", "gpt-6-luna"),
        ("openai", "o3", "o3"),
        ("openai", "o3-2025-04-16", "o3"),
        ("openai", "o3-pro", "o3-pro"),
        ("openai", "o4-mini", "o4-mini"),
    ] {
        let vendor = if provider == "gemini" {
            "google"
        } else {
            provider
        };
        assert_eq!(
            get_model_profile_key(provider, wire_id).as_deref(),
            Some(format!("{vendor}/{canonical}").as_str()),
            "{provider}/{wire_id}"
        );
        let alias = get_model_profile(provider, wire_id).unwrap();
        let base = get_model_profile(provider, canonical).unwrap();
        assert_eq!(
            serde_json::to_value(alias).unwrap(),
            serde_json::to_value(base).unwrap(),
            "{provider}/{wire_id}"
        );
    }
}

#[test]
fn cache_write_pricing_uses_disjoint_buckets_and_context_tier() {
    for (model, input) in [
        ("gpt-6-astra", 10.0),
        ("gpt-6-sol", 2.0),
        ("gpt-6-luna", 0.1),
        ("gpt-5.6-sol", 5.0),
        ("gpt-5.6-terra", 2.5),
        ("gpt-5.6-luna", 1.0),
    ] {
        let cost = estimate_cost_usd("openai", model, 1000, 0, 2000, 3000).unwrap();
        assert!((cost - input * (1000.0 + 200.0 + 3750.0) / 1_000_000.0).abs() < 1e-10);
        // Cache buckets count toward the threshold. At the boundary use base rates.
        for (written, multiplier) in [(271_000, 1.0), (271_001, 2.0)] {
            let cost = estimate_cost_usd("openai", model, 0, 0, 1000, written).unwrap();
            let expected = input * multiplier * (100.0 + written as f64 * 1.25) / 1_000_000.0;
            assert!(
                (cost - expected).abs() < 1e-10,
                "{model}: {cost} != {expected}"
            );
        }
    }
    assert_eq!(
        estimate_cost_usd("openai", "gpt-5.5", 0, 0, 0, 1_000_000),
        Some(5.0)
    );
}

// Inspect actual registry members so new models cannot silently escape the
// invariant checks through a second, manually maintained model list.
#[test]
fn registered_model_profiles_are_structurally_consistent() {
    assert!(!REGISTRY.is_empty());
    for descriptor in REGISTRY {
        assert!(!descriptor.ids.is_empty());
        assert!(!descriptor.surfaces.is_empty());
        let id = descriptor.ids[0];
        let provider = descriptor.surfaces[0];
        let p = get_model_profile(provider, id)
            .unwrap_or_else(|| panic!("{id} should resolve under {provider:?}"));

        assert!(!p.name.trim().is_empty(), "{id}: empty name");
        assert!(!p.family.trim().is_empty(), "{id}: empty family");

        if let Some(cost) = &p.cost {
            assert!(
                cost.input.is_finite() && cost.input >= 0.0,
                "{id}: bad input cost {}",
                cost.input
            );
            assert!(
                cost.output.is_finite() && cost.output >= 0.0,
                "{id}: bad output cost {}",
                cost.output
            );
            if let Some(cache_read) = cost.cache_read {
                assert!(
                    cache_read.is_finite() && cache_read >= 0.0 && cache_read <= cost.input,
                    "{id}: cache_read {cache_read} must be >=0 and <= input {}",
                    cost.input
                );
            }
        }

        if let Some(limits) = &p.limits {
            assert!(limits.context > 0, "{id}: non-positive context");
            if descriptor.service == ServiceKind::Embeddings {
                assert_eq!(
                    limits.output, 0,
                    "{id}: embeddings do not generate output tokens"
                );
            } else {
                assert!(limits.output > 0, "{id}: non-positive output");
            }
            assert!(
                limits.output <= limits.context,
                "{id}: output {} exceeds context {}",
                limits.output,
                limits.context
            );
        }

        if let Some(effort) = &p.reasoning_effort {
            assert!(
                p.reasoning,
                "{id}: has reasoning_effort but reasoning flag is false"
            );
            assert!(
                !effort.values.is_empty(),
                "{id}: empty reasoning_effort set"
            );
            let mut seen: Vec<&ReasoningEffort> = Vec::new();
            for v in &effort.values {
                assert!(
                    !v.name.trim().is_empty(),
                    "{id}: reasoning_effort value with empty display name"
                );
                assert!(
                    !seen.contains(&&v.value),
                    "{id}: duplicate reasoning_effort value {:?}",
                    v.value
                );
                seen.push(&v.value);
            }
            assert!(
                effort.values.iter().any(|v| v.value == effort.default),
                "{id}: reasoning_effort default {:?} is not among its values",
                effort.default
            );
        }

        if let Some(speed) = &p.speed {
            assert!(!speed.values.is_empty(), "{id}: empty speed set");
            let mut seen: Vec<&Speed> = Vec::new();
            for v in &speed.values {
                assert!(
                    !v.name.trim().is_empty(),
                    "{id}: speed value with empty display name"
                );
                assert!(
                    !seen.contains(&&v.value),
                    "{id}: duplicate speed value {:?}",
                    v.value
                );
                seen.push(&v.value);
            }
            assert!(
                speed.values.iter().any(|v| v.value == speed.default),
                "{id}: speed default {:?} is not among its values",
                speed.default
            );
        }
    }
}

#[test]
fn test_profile_keys_and_service_kinds() {
    // Canonical key from a dated wire id (version-suffix normalization).
    assert_eq!(
        get_model_profile_key("anthropic", "claude-sonnet-4-6-20260217").as_deref(),
        Some("anthropic/claude-sonnet-4-6")
    );
    // Gateway alias and bare id share one key (same model identity).
    for (provider, id) in [
        ("openrouter", "nvidia/nemotron-3-super-120b-a12b"),
        ("openai", "nemotron-3-super-120b-a12b"),
    ] {
        assert_eq!(
            get_model_profile_key(provider, id).as_deref(),
            Some("nvidia/nemotron-3-super-120b-a12b")
        );
    }
    // Unknown models have no key.
    assert_eq!(get_model_profile_key("openai", "not-a-model"), None);

    // By-key lookup round-trips.
    let profile = get_model_profile_by_key("openai/gpt-5.5").unwrap();
    assert_eq!(profile.name, "GPT-5.5");
    // Both key segments are matched ASCII case-insensitively.
    assert!(get_model_profile_by_key("OpenAI/GPT-5.5").is_some());
    assert!(get_model_profile_by_key("openai/not-a-model").is_none());
    assert!(get_model_profile_by_key("no-slash").is_none());

    // Service kinds: realtime models are not chat models.
    assert_eq!(
        get_model_service_kind("openai", "gpt-realtime-2"),
        ServiceKind::Realtime
    );
    assert_eq!(
        get_model_service_kind("openai", "gpt-5.5"),
        ServiceKind::Chat
    );
    // Unknown models default to chat.
    assert_eq!(
        get_model_service_kind("openai", "not-a-model"),
        ServiceKind::Chat
    );
}

#[test]
fn test_get_profile_unknown_model() {
    let profile = get_model_profile("openai", "unknown-model");
    assert!(profile.is_none());
}

#[test]
fn test_retired_semantic_variants_do_not_resolve_to_parent_profiles() {
    assert!(get_model_profile("openai", "o3-mini").is_none());
    assert!(get_model_profile("anthropic", "claude-opus-4-1").is_none());
}

#[test]
fn test_get_profile_wrong_provider() {
    // Try to get an OpenAI model with Anthropic provider
    let profile = get_model_profile("anthropic", "gpt-5.2");
    assert!(profile.is_none());
}

#[test]
fn test_openai_completions_uses_openai_profiles() {
    let profile = get_model_profile("openai_completions", "gpt-5.2");
    assert!(profile.is_some());
    assert_eq!(profile.unwrap().name, "GPT-5.2");
}

#[test]
fn test_azure_openai_uses_openai_profiles() {
    let profile = get_model_profile("azure_openai", "gpt-5.2");
    assert!(profile.is_some());
    assert_eq!(profile.unwrap().name, "GPT-5.2");
}

// Speed (service tier) availability follows OpenAI's official tier tables;
// see the speed_* helper docs.

#[test]
fn test_speed_config_matches_pricing_tiers() {
    let speeds = |model: &str| -> Vec<Speed> {
        get_model_profile("openai", model)
            .unwrap()
            .speed
            .map(|s| s.values.into_iter().map(|v| v.value).collect())
            .unwrap_or_default()
    };
    use Speed::*;
    // Flex + priority pricing rows.
    for model in [
        "gpt-6-sol",
        "gpt-6-luna",
        "gpt-5.6-sol",
        "gpt-5.6-terra",
        "gpt-5.6-luna",
        "gpt-5.5",
        "gpt-5.4",
        "gpt-5.4-mini",
    ] {
        assert_eq!(speeds(model), vec![Flex, Default, Priority], "{model}");
    }
    // Flex-only pricing rows.
    for model in ["gpt-5.5-pro", "gpt-5.4-nano", "gpt-5.4-pro"] {
        assert_eq!(speeds(model), vec![Flex, Default], "{model}");
    }
    // Priority-only pricing rows.
    for model in [
        "gpt-4.1",
        "gpt-4.1-mini",
        "gpt-4.1-nano",
        "gpt-5",
        "gpt-5-mini",
        "gpt-5-codex",
        "gpt-5.1",
        "gpt-5.1-codex",
        "gpt-5.2",
        "gpt-5.3-codex",
        "o3",
        "o4-mini",
    ] {
        assert_eq!(speeds(model), vec![Default, Priority], "{model}");
    }
    // No tier rows: unlisted variants, chat-latest, and deep research.
    for model in [
        "gpt-5-nano",
        "gpt-5-pro",
        "gpt-5.1-codex-mini",
        "gpt-5.1-codex-max",
        "gpt-5.2-pro",
        "gpt-5.2-codex",
        "gpt-5-chat-latest",
        "o3-deep-research",
    ] {
        assert_eq!(speeds(model), vec![], "{model}");
    }
}

#[test]
fn test_speed_masked_for_non_openai_surfaces() {
    assert!(
        get_model_profile("openai", "gpt-5.2")
            .unwrap()
            .speed
            .is_some()
    );
    // Service tiers are OpenAI-platform billing; other surfaces reaching
    // the same model must not advertise the selector.
    for provider in ["azure_openai", "openrouter", "openai_completions"] {
        assert!(
            get_model_profile(provider, "gpt-5.2")
                .unwrap()
                .speed
                .is_none(),
            "{provider:?}"
        );
    }
}

#[test]
fn reasoning_effort_sets_match_model_contracts() {
    use ReasoningEffort::{High, Low, Medium, None as NoReasoning, Xhigh};
    let cases: &[(&str, ReasoningEffort, &[ReasoningEffort])] = &[
        ("gpt-5", Medium, &[Low, Medium, High]),
        ("gpt-5-mini", Medium, &[Low, Medium, High]),
        ("gpt-5-pro", High, &[High]),
        ("gpt-5.1", NoReasoning, &[NoReasoning, Low, Medium, High]),
        (
            "gpt-5.1-codex-max",
            NoReasoning,
            &[NoReasoning, Low, Medium, High, Xhigh],
        ),
        ("o3", Medium, &[Low, Medium, High]),
        ("o3-pro", High, &[High]),
        ("o4-mini", Medium, &[Low, Medium, High]),
    ];
    for (id, default, values) in cases {
        let profile = get_model_profile("openai", id).unwrap();
        assert!(profile.reasoning, "{id}");
        assert!(profile.tool_call, "{id}");
        assert_eq!(profile.family, *id, "{id}");
        let effort = profile.reasoning_effort.unwrap();
        assert_eq!(effort.default, *default, "{id}");
        assert_eq!(
            effort.values.iter().map(|v| v.value).collect::<Vec<_>>(),
            *values,
            "{id}"
        );
    }
}

#[test]
fn test_gpt52_profile() {
    let profile = get_model_profile("openai", "gpt-5.2").unwrap();
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
    let profile = get_model_profile("openai", "gpt-5.2-pro").unwrap();
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
fn test_gpt55_profile() {
    let profile = get_model_profile("openai", "gpt-5.5").unwrap();
    assert_eq!(profile.name, "GPT-5.5");
    assert_eq!(profile.family, "gpt-5.5");
    assert!(profile.reasoning);
    assert!(profile.tool_call);
    assert!(profile.structured_output);
    assert!(profile.tool_search);
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
    let profile = get_model_profile("openai", "gpt-realtime-2").unwrap();
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
    let profile = get_model_profile("openai", "gpt-5.5-pro").unwrap();
    assert_eq!(profile.name, "GPT-5.5 Pro");
    assert_eq!(profile.family, "gpt-5.5-pro");
    assert!(profile.reasoning);
    assert!(profile.tool_call);
    assert!(profile.structured_output);
    assert!(profile.tool_search);
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
fn test_gpt56_profiles() {
    // Sol, Terra, Luna share the same shape: 1.05M context, 128K output,
    // 2026-02-16 knowledge cutoff, tool_search + native phases, and a
    // >272K-token pricing tier (2x input, 1.5x output).
    for (id, name, family, input, output, cache, tier_input, tier_output, tier_cache) in [
        (
            "gpt-5.6-sol",
            "GPT-5.6 Sol",
            "gpt-5.6-sol",
            5.00,
            30.00,
            0.50,
            10.00,
            45.00,
            1.00,
        ),
        (
            "gpt-5.6-terra",
            "GPT-5.6 Terra",
            "gpt-5.6-terra",
            2.50,
            15.00,
            0.25,
            5.00,
            22.50,
            0.50,
        ),
        (
            "gpt-5.6-luna",
            "GPT-5.6 Luna",
            "gpt-5.6-luna",
            1.00,
            6.00,
            0.10,
            2.00,
            9.00,
            0.20,
        ),
    ] {
        let profile = get_model_profile("openai", id).unwrap();
        assert_eq!(profile.name, name);
        assert_eq!(profile.family, family);
        assert!(profile.reasoning);
        assert!(!profile.temperature);
        assert!(profile.tool_call);
        assert!(profile.structured_output);
        assert!(profile.tool_search);
        assert!(profile.supports_phases);
        assert_eq!(profile.knowledge.as_deref(), Some("2026-02-16"));

        let limits = profile.limits.unwrap();
        assert_eq!(limits.context, 1_050_000);
        assert_eq!(limits.output, 128_000);

        let cost = profile.cost.unwrap();
        assert!((cost.input - input).abs() < f64::EPSILON);
        assert!((cost.output - output).abs() < f64::EPSILON);
        assert!((cost.cache_read.unwrap() - cache).abs() < f64::EPSILON);
        assert_eq!(cost.cost_tiers.len(), 1);
        let tier = &cost.cost_tiers[0];
        assert_eq!(tier.above_tokens, 272_000);
        assert!((tier.input - tier_input).abs() < f64::EPSILON);
        assert!((tier.output - tier_output).abs() < f64::EPSILON);
        assert!((tier.cache_read.unwrap() - tier_cache).abs() < f64::EPSILON);

        // Series default: medium, supports none/low/medium/high/xhigh.
        let effort = profile.reasoning_effort.unwrap();
        assert_eq!(effort.default, ReasoningEffort::Medium);
        assert_eq!(effort.values.len(), 5);
    }
}

#[test]
fn test_gpt54_profile() {
    let profile = get_model_profile("openai", "gpt-5.4").unwrap();
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
    let profile = get_model_profile("openai", "gpt-5.4-mini").unwrap();
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
    let profile = get_model_profile("openai", "gpt-5.4-nano").unwrap();
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
    let profile = get_model_profile("openai", "gpt-5.4-pro").unwrap();
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

// Claude 4.7 / 4.6 model tests

#[test]
fn test_claude_opus_47_profile() {
    let profile = get_model_profile("anthropic", "claude-opus-4-7").unwrap();
    assert_eq!(profile.name, "Claude Opus 4.7");
    assert_eq!(profile.family, "claude-opus-4-7");
    assert!(profile.reasoning);
    assert!(profile.tool_call);
    // Sampling parameters removed from Opus 4.7 on (API rejects `temperature`).
    assert!(!profile.temperature);
    assert!(profile.structured_output);

    let limits = profile.limits.unwrap();
    // Bare id is the 200K profile; the 1M window is `claude-opus-4-7[1m]`.
    assert_eq!(limits.context, 200_000);
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
    let profile = get_model_profile("anthropic", "claude-opus-4-6").unwrap();
    assert_eq!(profile.name, "Claude Opus 4.6");
    assert_eq!(profile.family, "claude-opus-4-6");
    assert!(profile.reasoning);
    assert!(profile.tool_call);
    assert!(profile.temperature);
    assert!(profile.structured_output);

    let limits = profile.limits.unwrap();
    // Bare id is the 200K profile; the 1M window is `claude-opus-4-6[1m]`.
    assert_eq!(limits.context, 200_000);
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
    let profile = get_model_profile("anthropic", "claude-sonnet-4-6").unwrap();
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

// Normalize tests for new models

// Gemini model tests

#[test]
fn test_gemini_unknown_model() {
    let profile = get_model_profile("gemini", "unknown-model");
    assert!(profile.is_none());
}

// Newly added flagship model profiles

// Claude `[1m]` twin tests live in `profiles_claude_tests.rs`.

#[test]
fn test_gemini_3_1_pro_preview_profile() {
    let profile = get_model_profile("gemini", "gemini-3.1-pro-preview").unwrap();
    assert_eq!(profile.name, "Gemini 3.1 Pro Preview");
    assert!(profile.reasoning);
    // >200K-token pricing tier.
    let cost = profile.cost.as_ref().unwrap();
    assert_eq!(cost.cost_tiers.len(), 1);
    assert_eq!(cost.cost_tiers[0].above_tokens, 200_000);
    assert_eq!(cost.cost_tiers[0].input, 4.00);
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
        ("kimi-k3", "Kimi K3"),
        ("moonshotai/kimi-k3", "Kimi K3"),
        ("grok-4.3", "Grok 4.3"),
        ("x-ai/grok-4.3", "Grok 4.3"),
        ("grok-4.7", "Grok 4.7"),
        ("x-ai/grok-4.7", "Grok 4.7"),
        ("qwen3.8-max", "Qwen3.8 Max"),
        ("qwen/qwen3.8-max", "Qwen3.8 Max"),
    ];
    for (id, name) in cases {
        let profile = get_model_profile("openai_completions", id)
            .unwrap_or_else(|| panic!("missing profile for {id}"));
        assert_eq!(profile.name, name, "wrong profile for {id}");
    }
}

#[test]
fn test_kimi_k3_profile() {
    let profile = get_model_profile("openai_completions", "kimi-k3").unwrap();
    assert_eq!(profile.name, "Kimi K3");
    assert_eq!(profile.family, "kimi-k3");
    assert!(profile.reasoning);
    assert!(profile.tool_call);
    assert!(profile.open_weights);
    assert!(!profile.temperature);
    let cost = profile.cost.as_ref().unwrap();
    assert_eq!(cost.input, 3.00);
    assert_eq!(cost.output, 15.00);
    assert_eq!(cost.cache_read, Some(0.30));
    assert!(cost.cost_tiers.is_empty());
    let limits = profile.limits.as_ref().unwrap();
    assert_eq!(limits.context, 1_048_576);
    assert_eq!(limits.output, 131_072);
    let modalities = profile.modalities.as_ref().unwrap();
    assert_eq!(
        modalities.input,
        vec![Modality::Text, Modality::Image, Modality::Video]
    );
}

#[test]
fn test_mistral_large_4_profile() {
    // The first-party id, its dated alias, and the gateway slugs all resolve
    // to one profile on every surface Mistral is served through.
    for (provider, id) in [
        ("mistral", "mistral-large-4"),
        ("mistral", "mistral-large-4-0"),
        ("openai_completions", "mistral-large-4"),
        ("openrouter", "mistralai/mistral-large-4-0"),
        ("openrouter", "mistral/mistral-large-4"),
    ] {
        let profile = get_model_profile(provider, id)
            .unwrap_or_else(|| panic!("missing profile for {provider}:{id}"));
        assert_eq!(profile.name, "Mistral Large 4", "{provider}:{id}");
        assert_eq!(
            get_model_profile_key(provider, id).as_deref(),
            Some("mistral/mistral-large-4")
        );
        assert_eq!(get_model_vendor(provider, id), Some(ModelVendor::Mistral));
    }
    // Mistral does not serve the Responses API, and Azure never hosts it.
    for provider in ["openai", "azure_openai", "anthropic"] {
        assert!(
            get_model_profile(provider, "mistral-large-4").is_none(),
            "{provider}"
        );
    }

    let profile = get_model_profile("mistral", "mistral-large-4").unwrap();
    assert_eq!(profile.family, "mistral-large");
    assert!(profile.reasoning && profile.tool_call && profile.structured_output);
    assert!(profile.attachment && profile.temperature);
    assert!(!profile.open_weights, "weights are not out yet");
    // List price, not the launch discount.
    let cost = profile.cost.as_ref().unwrap();
    assert_eq!((cost.input, cost.output), (1.36, 4.18));
    assert_eq!(cost.cache_read, Some(0.14));
    let limits = profile.limits.as_ref().unwrap();
    assert_eq!((limits.context, limits.output), (524_288, 262_144));
    assert_eq!(
        profile.modalities.as_ref().unwrap().input,
        vec![Modality::Text, Modality::Image]
    );
    // The API takes only "none" and "high"; offering a middle grade would be a
    // 400 at request time.
    let effort = profile.reasoning_effort.as_ref().unwrap();
    assert_eq!(
        effort.values.iter().map(|v| v.value).collect::<Vec<_>>(),
        vec![ReasoningEffort::None, ReasoningEffort::High]
    );
    assert_eq!(effort.default, ReasoningEffort::None);
    assert!(!profile.supports_phases && !profile.tool_search);
}

#[test]
fn test_mistral_medium_and_small_profiles() {
    for (provider, id, name) in [
        ("mistral", "mistral-medium-2604", "Mistral Medium 3.5"),
        ("mistral", "mistral-medium-latest", "Mistral Medium 3.5"),
        (
            "openrouter",
            "mistralai/mistral-medium-3-5",
            "Mistral Medium 3.5",
        ),
        ("mistral", "mistral-small-2603", "Mistral Small 4"),
        ("mistral", "mistral-small-latest", "Mistral Small 4"),
        (
            "openai_completions",
            "mistral-small-2603",
            "Mistral Small 4",
        ),
    ] {
        let profile = get_model_profile(provider, id)
            .unwrap_or_else(|| panic!("missing profile for {provider}:{id}"));
        assert_eq!(profile.name, name, "{provider}:{id}");
        assert_eq!(get_model_vendor(provider, id), Some(ModelVendor::Mistral));
        // Same none/high toggle as Large 4.
        let effort = profile.reasoning_effort.as_ref().unwrap();
        assert_eq!(
            effort.values.iter().map(|v| v.value).collect::<Vec<_>>(),
            vec![ReasoningEffort::None, ReasoningEffort::High]
        );
        assert!(profile.open_weights);
    }
    // Older dated Medium/Small ids are different models and stay unprofiled.
    assert!(get_model_profile("mistral", "mistral-medium-2508").is_none());
    assert!(get_model_profile("mistral", "mistral-small-2506").is_none());
    assert!(get_model_profile("openai", "mistral-small-2603").is_none());

    let medium = get_model_profile("mistral", "mistral-medium-2604").unwrap();
    let cost = medium.cost.as_ref().unwrap();
    assert_eq!((cost.input, cost.output), (1.50, 7.50));
    assert!(medium.structured_output);
    let small = get_model_profile("mistral", "mistral-small-2603").unwrap();
    assert_eq!(small.limits.as_ref().unwrap().output, 256_000);
    assert!(!small.structured_output, "models.dev does not assert it");
}

#[test]
fn test_recent_gemini_flash_profiles() {
    for (id, name, input) in [
        ("gemini-3.8-flash", "Gemini 3.8 Flash", 0.75),
        ("gemini-3.7-flash", "Gemini 3.7 Flash", 0.75),
        ("gemini-3.6-flash", "Gemini 3.6 Flash", 0.75),
        ("gemini-3.5-flash-lite", "Gemini 3.5 Flash Lite", 0.30),
    ] {
        let profile =
            get_model_profile("gemini", id).unwrap_or_else(|| panic!("missing profile for {id}"));
        assert_eq!(profile.name, name);
        assert_eq!(profile.cost.as_ref().unwrap().input, input, "{id}");
        assert_eq!(profile.limits.as_ref().unwrap().context, 1_048_576);
    }
    // The Lite id must not fall back to the 3.5 Flash profile.
    assert_eq!(
        get_model_profile("gemini", "gemini-3.5-flash-lite")
            .unwrap()
            .family,
        "gemini-3.5-flash-lite"
    );
}

#[test]
fn test_recent_openai_aliases_and_realtime() {
    let sol = get_model_profile("openai", "gpt-5.6").unwrap();
    assert_eq!(
        sol.name,
        get_model_profile("openai", "gpt-5.6-sol").unwrap().name
    );
    assert_eq!(
        get_model_service_kind("openai", "gpt-realtime-2.1"),
        ServiceKind::Realtime
    );
    assert_eq!(
        get_model_profile("openai", "gpt-realtime-2.1")
            .unwrap()
            .name,
        "GPT Realtime 2.1"
    );
}

#[test]
fn test_grok_4_7_has_context_tier() {
    let profile = get_model_profile("openrouter", "x-ai/grok-4.7").unwrap();
    let cost = profile.cost.as_ref().unwrap();
    assert_eq!((cost.input, cost.output), (2.00, 6.00));
    assert_eq!(cost.cost_tiers[0].above_tokens, 200_000);
    assert_eq!(cost.cost_tiers[0].input, 4.00);
}

#[test]
fn test_grok_4_3_has_context_tier() {
    let profile = get_model_profile("openai_completions", "grok-4.3").unwrap();
    let cost = profile.cost.as_ref().unwrap();
    assert_eq!(cost.cost_tiers.len(), 1);
    assert_eq!(cost.cost_tiers[0].above_tokens, 200_000);
}

#[test]
fn test_mai_preview_has_no_cost_or_limits() {
    // Microsoft never published pricing/limits for MAI-1-preview.
    let profile = get_model_profile("openai_completions", "MAI-1-preview").unwrap();
    assert!(profile.cost.is_none());
    assert!(profile.limits.is_none());
    assert!(!profile.reasoning);
}

#[test]
fn test_third_party_unknown_still_none() {
    let profile = get_model_profile("openai_completions", "totally-made-up");
    assert!(profile.is_none());
}

#[test]
fn test_third_party_surfaces() {
    // Third-party, OpenAI-compatible models are reachable via the Chat
    // Completions path AND via Responses-capable gateways (e.g. OpenRouter
    // configured as an `openai` provider) — but never Azure OpenAI.
    for id in [
        "qwen3.7-max",
        "MiniMax-M3",
        "grok-4.3",
        "nemotron-3-super-120b-a12b",
    ] {
        assert!(
            get_model_profile("openai_completions", id).is_some(),
            "{id} should resolve under openai_completions"
        );
        assert!(
            get_model_profile("openai", id).is_some(),
            "{id} should resolve under openai (Open Responses gateway)"
        );
        assert!(
            get_model_profile("azure_openai", id).is_none(),
            "{id} must not resolve under azure_openai"
        );
        assert!(
            get_model_profile("anthropic", id).is_none(),
            "{id} must not resolve under anthropic"
        );
    }
    // Native phases / tool_search are advertised only on the Responses
    // surface, never on Chat Completions.
    let grok_completions = get_model_profile("openai_completions", "grok-4.3").unwrap();
    assert!(!grok_completions.supports_phases);
    assert!(!grok_completions.tool_search);

    // Genuine OpenAI models still resolve under all OpenAI-family types.
    assert!(get_model_profile("openai", "gpt-5.2").is_some());
    assert!(get_model_profile("azure_openai", "gpt-5.2").is_some());
}

#[test]
fn test_phases_and_tool_search_gated_to_responses_surface() {
    // GPT-5.4 advertises phases + tool_search on the Responses surface...
    let responses = get_model_profile("openai", "gpt-5.4").unwrap();
    assert!(responses.supports_phases);
    assert!(responses.tool_search);
    // ...but not when reached via Chat Completions or Azure.
    for provider in ["openai_completions", "azure_openai", "openrouter"] {
        let profile = get_model_profile(provider, "gpt-5.4").unwrap();
        assert!(!profile.supports_phases, "{provider}");
        assert!(!profile.tool_search, "{provider}");
    }
}

#[test]
fn test_anthropic_native_tool_search_by_family() {
    // Claude 4-family + Fable advertise Anthropic's hosted tool_search.
    for id in [
        "claude-fable-5-1",
        "claude-fable-5",
        "claude-opus-5-5",
        "claude-opus-5",
        "claude-opus-4-8",
        "claude-opus-4-7",
        "claude-opus-4-6",
        "claude-opus-4-5",
        "claude-opus-4",
        "claude-sonnet-5-5",
        "claude-sonnet-4-6",
        "claude-haiku-5-5",
        "claude-haiku-4-5",
    ] {
        let p = get_model_profile("anthropic", id)
            .unwrap_or_else(|| panic!("{id} should resolve under anthropic"));
        assert!(p.tool_search, "{id} should advertise native tool_search");
    }
    // The 1M twin inherits the flag.
    assert!(
        get_model_profile("anthropic", "claude-opus-4-8[1m]")
            .unwrap()
            .tool_search
    );
    // Retired pre-4 Claude models are no longer in the registry at all.
    assert!(get_model_profile("anthropic", "claude-3-5-haiku").is_none());
    // This registry does not expose Claude through Bedrock. An optional
    // assertion on a nonexistent profile would never exercise masking.
    assert!(get_model_profile("bedrock", "claude-opus-4-8").is_none());
}

#[test]
fn test_model_vendor_lookup() {
    assert_eq!(
        get_model_vendor("anthropic", "claude-opus-4-8"),
        Some(ModelVendor::Anthropic)
    );
    assert_eq!(
        get_model_vendor("openai_completions", "nvidia/nemotron-3-super-120b-a12b"),
        Some(ModelVendor::Nvidia)
    );
    assert_eq!(
        get_model_vendor("openai", "gpt-5.4"),
        Some(ModelVendor::OpenAi)
    );
    assert_eq!(get_model_vendor("openai", "made-up"), None);
}

#[test]
fn test_muse_spark_1_3_profiles_and_tiers() {
    let standard = get_model_profile("meta", "muse-spark-1.3").unwrap();
    assert_eq!(standard.name, "Muse Spark 1.3");
    assert_eq!(standard.family, "muse-spark-1.3");
    assert_eq!(standard.limits.as_ref().unwrap().context, 1_048_576);
    assert!(standard.tool_call);
    assert!(standard.tool_search);
    assert!(standard.structured_output);
    assert!(standard.supports_phases);
    assert_eq!(
        standard.modalities.as_ref().unwrap().input,
        vec![
            Modality::Text,
            Modality::Image,
            Modality::Audio,
            Modality::Video,
            Modality::Pdf,
        ]
    );
    let standard_cost = standard.cost.unwrap();
    assert_eq!(standard_cost.input, 1.25);
    assert_eq!(standard_cost.cache_read, Some(0.15));
    assert_eq!(standard_cost.output, 4.25);

    let contributor = get_model_profile("meta", "muse-spark-1.3-contributor").unwrap();
    assert_eq!(contributor.name, "Muse Spark 1.3 Contributor");
    assert_eq!(contributor.family, "muse-spark-1.3");
    let contributor_cost = contributor.cost.unwrap();
    assert_eq!(contributor_cost.input, 0.10);
    assert_eq!(contributor_cost.cache_read, Some(0.002));
    assert_eq!(contributor_cost.output, 0.20);
    assert!(
        contributor
            .description
            .as_deref()
            .unwrap()
            .contains("used to train")
    );
    assert_eq!(
        get_model_profile_key("meta", "muse-spark-1.3-contributor").as_deref(),
        Some("meta/muse-spark-1.3-contributor")
    );
}

#[test]
fn test_muse_spark_1_2_profiles_stay_resolvable() {
    // 1.2 remains served by Meta and referenced by existing model records,
    // so it keeps its own family and profile next to 1.3.
    let standard = get_model_profile("meta", "muse-spark-1.2").unwrap();
    assert_eq!(standard.name, "Muse Spark 1.2");
    assert_eq!(standard.family, "muse-spark-1.2");
    let contributor = get_model_profile("meta", "muse-spark-1.2-contributor").unwrap();
    assert_eq!(contributor.name, "Muse Spark 1.2 Contributor");
    assert_eq!(contributor.family, "muse-spark-1.2");
    // Both tiers reach the gateways under their vendor-prefixed ids, which
    // is how OpenRouter spells them.
    assert!(get_model_profile("openrouter", "meta/muse-spark-1.2-contributor").is_some());
}

#[test]
fn test_muse_surface_capabilities_are_transport_gated() {
    let direct = get_model_profile("meta", "muse-spark-1.3").unwrap();
    assert!(direct.supports_phases);
    assert!(direct.tool_search);

    let openrouter = get_model_profile("openrouter", "meta/muse-spark-1.3").unwrap();
    assert!(!openrouter.supports_phases);
    assert!(!openrouter.tool_search);

    // The Contributor tier is gated the same way, not withheld. It was
    // listed first-party only on the assumption that its data-use bargain
    // was a Meta Model API product term; OpenRouter serves it and passes
    // Meta's tier pricing through unchanged.
    let contributor_direct = get_model_profile("meta", "muse-spark-1.3-contributor").unwrap();
    assert!(contributor_direct.supports_phases);
    assert!(contributor_direct.tool_search);

    let contributor_gateway =
        get_model_profile("openrouter", "meta/muse-spark-1.3-contributor").unwrap();
    assert!(!contributor_gateway.supports_phases);
    assert!(!contributor_gateway.tool_search);
    // Same id, same tier: the discount is the reason to pick it, so the
    // profile must not quietly hand back standard-tier pricing.
    let cost = contributor_gateway
        .cost
        .expect("contributor tier is priced");
    assert_eq!(cost.input, 0.10);
    assert_eq!(cost.output, 0.20);

    assert_eq!(
        get_model_vendor("meta", "muse-spark-1.3"),
        Some(ModelVendor::Meta)
    );
}

#[test]
fn test_third_party_alias_matching_is_case_insensitive() {
    // Lowercased and vendor-prefixed variants all resolve to the same model.
    let cases = [
        ("minimax-m3", "MiniMax-M3"),
        ("minimax/minimax-m3", "MiniMax-M3"),
        ("MiniMax-M3", "MiniMax-M3"),
        ("microsoft/mai-1-preview", "MAI-1-preview"),
        ("MAI-1-PREVIEW", "MAI-1-preview"),
        ("X-AI/Grok-4.3", "Grok 4.3"),
    ];
    for (id, name) in cases {
        let profile = get_model_profile("openai_completions", id)
            .unwrap_or_else(|| panic!("missing profile for {id}"));
        assert_eq!(profile.name, name, "wrong profile for {id}");
    }
}

#[test]
fn test_estimate_cost_usd_known_model() {
    // gpt-5.2 profile: input $1.75/M, output $14.00/M.
    let est = estimate_cost_usd("openai", "gpt-5.2", 1_000_000, 500_000, 0, 0)
        .expect("known model should yield an estimate");
    // 1M input * 1.75 + 0.5M output * 14.00 = 1.75 + 7.00 = 8.75
    assert!((est - 8.75).abs() < 1e-9, "got {est}");
}

#[test]
fn test_estimate_cost_usd_unknown_model_is_none() {
    assert!(estimate_cost_usd("openai", "no-such-model", 100, 50, 0, 0).is_none());
}

#[test]
fn test_estimate_cost_usd_openai_embedding_model() {
    let estimate = estimate_cost_usd("openai", "text-embedding-3-small", 1_000_000, 0, 0, 0);

    assert_eq!(estimate, Some(0.02));
    assert_eq!(
        get_model_service_kind("openai", "text-embedding-3-small"),
        ServiceKind::Embeddings
    );
}

#[test]
fn test_estimate_cost_usd_bills_disjoint_buckets() {
    // Disjoint convention: `input_tokens` is non-cached, `cache_read_tokens`
    // additive. gpt-5.2: input $1.75/M, cache_read $0.175/M. 200K non-cached
    // input + 800K cache reads each bill at their own rate.
    let est = estimate_cost_usd("openai", "gpt-5.2", 200_000, 0, 800_000, 0)
        .expect("known model should yield an estimate");
    // 200K * 1.75 + 800K * 0.175 = 0.35 + 0.14 = 0.49.
    assert!((est - 0.49).abs() < 1e-9, "got {est}");
}

#[test]
fn test_estimate_cost_usd_cache_heavy_run_is_cheap() {
    // Regression for EVE-599 / EVE-661: gpt-5.5 (input $5/M, output $30/M,
    // cache_read $0.50/M) on a cache-heavy run. With disjoint buckets the
    // driver reports the non-cached remainder (42K) plus 285K cache reads
    // (87% of the original 327K prompt was cached).
    let input = 42_407;
    let cache_read = 284_672;
    let output = 2_096;
    let est =
        estimate_cost_usd("openai", "gpt-5.5", input, output, cache_read, 0).expect("known model");
    // 42K*5 + 285K*0.5 + 2K*30 (per M) ≈ $0.42 — far below the ~$1.70 that
    // billing the whole 327K prompt at the full input rate would produce.
    assert!((est - 0.417251).abs() < 1e-9, "est {est}");
}

#[test]
fn test_estimate_cost_usd_applies_tier_to_whole_request() {
    // GPT-5.6 Sol charges prompts above 272K input tokens at the tiered
    // rates for the whole request; the exact threshold stays at base rates.
    for (input, expected) in [(272_000, 4.36), (272_001, 7.22001), (300_000, 7.50)] {
        let est = estimate_cost_usd("openai", "gpt-5.6-sol", input, 100_000, 0, 0)
            .expect("known tiered model should yield an estimate");
        assert!((est - expected).abs() < 1e-9, "input {input}: got {est}");
    }
}

#[test]
fn test_estimate_cost_usd_uses_cache_tokens_for_tier_threshold() {
    let est = estimate_cost_usd("openai", "gpt-5.6-luna", 42_000, 100_000, 260_000, 0)
        .expect("known tiered model should yield an estimate");
    // The prompt exceeds the 272K tier threshold after cached reads are
    // included, so non-cached input, cached reads, and output use tier rates.
    // 42K*$2/M + 260K*$0.20/M + 100K*$9/M = $1.036.
    assert!((est - 1.036).abs() < 1e-9, "got {est}");
}

#[test]
fn test_estimate_cost_usd_anthropic_cache_is_additive() {
    // Anthropic reports cached tokens separately from `input_tokens`, so a
    // cached read must add cost rather than be subtracted out of the input.
    let model = "claude-haiku-4-5";
    let base =
        estimate_cost_usd("anthropic", model, 1_000, 0, 0, 0).expect("known anthropic model");
    let with_cache =
        estimate_cost_usd("anthropic", model, 1_000, 0, 5_000, 0).expect("known anthropic model");
    // $1/M noncached input plus $0.10/M cache reads: the discounted bucket
    // is additive, but must not be charged at the full input rate.
    assert!((base - 0.001).abs() < 1e-9, "base={base}");
    assert!(
        (with_cache - 0.0015).abs() < 1e-9,
        "with_cache={with_cache}"
    );
}
