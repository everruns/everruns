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
        ("openai", "gpt-5", "gpt-5"),
        ("openai", "gpt-5-2025-08-07", "gpt-5"),
        ("openai", "gpt-5-mini", "gpt-5-mini"),
        ("openai", "gpt-5-nano", "gpt-5-nano"),
        ("openai", "gpt-5-pro", "gpt-5-pro"),
        ("openai", "gpt-5.1", "gpt-5.1"),
        ("openai", "gpt-5.2", "gpt-5.2"),
        ("openai", "gpt-5.2-2025-12-11", "gpt-5.2"),
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
fn test_retired_semantic_variants_do_not_resolve_to_parent_profiles() {
    assert!(get_model_profile("openai", "o3-mini").is_none());
    assert!(get_model_profile("anthropic", "claude-opus-4-1").is_none());
}

/// Models past their vendor shutdown date leave the registry (OpenAI
/// deprecations of 2026-04-22, Anthropic's 2026-06-15 retirement, Gemini's
/// 2026-06-01 shutdown).
#[test]
fn test_models_past_vendor_shutdown_have_no_profile() {
    for model in [
        "gpt-5-chat-latest",
        "gpt-5-codex",
        "gpt-5.1-chat-latest",
        "gpt-5.1-codex",
        "gpt-5.1-codex-mini",
        "gpt-5.1-codex-max",
        "gpt-5.2-chat-latest",
        "gpt-5.2-codex",
        "o3-deep-research",
        "o4-mini",
        "o4-mini-deep-research",
        "gpt-4.1-nano",
    ] {
        assert!(get_model_profile("openai", model).is_none(), "{model}");
    }
    assert!(get_model_profile("anthropic", "claude-opus-4").is_none());
    assert!(get_model_profile("gemini", "gemini-2.0-flash").is_none());
}

// Speed (service tier) availability follows OpenAI's official tier tables;
// see the speed_* helper docs.

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

// Claude 4.7 / 4.6 model tests

// Normalize tests for new models

// Gemini model tests

// Newly added flagship model profiles

// Claude `[1m]` twin tests live in `profiles_claude_tests.rs`.

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
fn test_estimate_cost_usd_unknown_model_is_none() {
    assert!(estimate_cost_usd("openai", "no-such-model", 100, 50, 0, 0).is_none());
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

#[test]
fn openrouter_gpt_61_wire_id_keeps_canonical_prices_and_gateway_capabilities() {
    let canonical = get_model_profile("openai", "gpt-6.1-sol").unwrap();
    let routed = get_model_profile("openrouter", "openai/gpt-6.1-sol").unwrap();
    assert_eq!(routed.cost, canonical.cost);
    assert!(!routed.supports_phases);
    assert!(routed.speed.is_none());
    assert!(get_model_profile("openrouter", "untrusted/gpt-6.1-sol").is_none());
    assert!(get_model_profile("openrouter", "openai/gpt-6.1-sol-pro").is_none());
}

#[test]
fn terra_profiles_follow_verified_api_and_openrouter_contracts() {
    let direct = get_model_profile("openai", "gpt-6.1-terra").unwrap();
    let routed = get_model_profile("openrouter", "openai/gpt-6.1-terra").unwrap();
    assert_eq!(direct.family, "gpt-6.1-terra");
    assert_eq!(direct.cost, routed.cost);
    assert!(direct.tool_call && routed.tool_call && direct.attachment);
    assert_eq!(direct.limits.as_ref().unwrap().context, 1_050_000);
    assert_eq!(direct.limits.as_ref().unwrap().output, 128_000);
    let cost = direct.cost.as_ref().unwrap();
    assert_eq!(
        (cost.input, cost.output, cost.cache_read, cost.cache_write),
        (1.0, 5.0, Some(0.05), Some(1.25))
    );
    assert_eq!(cost.cost_tiers[0].above_tokens, 272_000);
    assert_eq!(cost.cost_tiers[0].input, 2.0);
    assert_eq!(cost.cost_tiers[0].output, 7.5);
    assert!(!direct.supports_phases && direct.supported_parameters.is_empty());
    assert!(get_model_profile("openai_completions", "gpt-6.1-terra").is_none());
    assert!(get_model_profile("azure_openai", "gpt-6.1-terra").is_none());
    assert!(get_model_profile("openrouter", "untrusted/gpt-6.1-terra").is_none());
    assert!(get_model_profile("openrouter", "openai/gpt-6.1-terra-pro").is_none());
    assert!(
        super::all_profile_entries()
            .iter()
            .any(|entry| entry.key == "openai/gpt-6.1-terra")
    );
}

#[test]
fn jev_1_14_is_a_text_decision_profile_on_supported_accounts() {
    for (provider, model) in [
        ("typesafe", "jev-1.14.0"),
        ("openrouter", "typesafe/jev-1.14"),
    ] {
        assert_eq!(
            get_model_service_kind(provider, model),
            ServiceKind::Decisions
        );
        assert_eq!(
            get_model_profile_key(provider, model).as_deref(),
            Some("typesafe/jev-1.14.0")
        );
        let profile = get_model_profile(provider, model).unwrap();
        assert_eq!(profile.name, "Jev 1.14");
        assert_eq!(profile.release_date.as_deref(), Some("2026-10-14"));
        assert!(!profile.attachment && !profile.reasoning && !profile.tool_call);
        let cost = profile.cost.unwrap();
        assert_eq!((cost.input, cost.output), (0.042, 0.0));
        let decisions = profile.decisions.unwrap();
        assert_eq!(decisions.primitives, ["noul", "choice", "score"]);
        assert!(decisions.calibrated);
        assert_eq!(decisions.state_tokens, Some(32_000));
        assert_eq!(decisions.request_tokens, Some(64_000));
        let entries = profile_entries_for_provider(provider);
        assert!(entries.iter().any(|entry| entry.model_id == "jev-1.14.0"));
        assert!(entries.iter().any(|entry| entry.model_id == "jev-1.13.0"));
    }
    assert!(get_model_profile("openai", "jev-1.14.0").is_none());
    assert_eq!(
        get_model_profile_key("openrouter", "~typesafe/jev-latest").as_deref(),
        Some("typesafe/jev-latest")
    );
}
