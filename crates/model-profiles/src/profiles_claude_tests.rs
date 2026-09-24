//! Claude `[1m]` twin and flagship profile tests, split out of `profiles.rs`
//! to keep that file under its size ratchet.

use super::*;
use crate::{CLEAR_AT_PARAMETER, MID_CONVERSATION_SYSTEM_PARAMETER};

#[test]
fn test_claude_opus_4_8_1m_variant() {
    // `[1m]` is the large-context twin: same flat pricing, 1M context,
    // "(1M)" display suffix, shared family for grouping.
    let base = get_model_profile("anthropic", "claude-opus-4-8").unwrap();
    assert_eq!(base.limits.as_ref().unwrap().context, 200_000);

    let m1 = get_model_profile("anthropic", "claude-opus-4-8[1m]").unwrap();
    assert_eq!(m1.name, "Claude Opus 4.8 (1M)");
    assert_eq!(m1.family, "claude-opus-4-8");
    assert_eq!(m1.limits.as_ref().unwrap().context, 1_000_000);
    assert_eq!(m1.limits.as_ref().unwrap().output, 128_000);

    // Flat standard pricing — Anthropic serves the 1M window with no
    // long-context premium, so cost matches the 200K base exactly.
    let (base_cost, m1_cost) = (base.cost.unwrap(), m1.cost.unwrap());
    assert_eq!(m1_cost.input, base_cost.input);
    assert_eq!(m1_cost.output, base_cost.output);
    assert_eq!(m1_cost.cache_read, base_cost.cache_read);
    assert!(m1_cost.cost_tiers.is_empty());
}

#[test]
fn test_claude_opus_5_1m_variant() {
    let base = get_model_profile("anthropic", "claude-opus-5").unwrap();
    assert_eq!(base.limits.as_ref().unwrap().context, 200_000);

    let m1 = get_model_profile("anthropic", "claude-opus-5[1m]").unwrap();
    assert_eq!(m1.name, "Claude Opus 5 (1M)");
    assert_eq!(m1.family, "claude-opus-5");
    assert_eq!(m1.limits.as_ref().unwrap().context, 1_000_000);
    assert_eq!(m1.limits.as_ref().unwrap().output, 128_000);

    // Flat standard pricing — the 1M window carries no long-context premium,
    // so cost matches the 200K base exactly.
    let (base_cost, m1_cost) = (base.cost.unwrap(), m1.cost.unwrap());
    assert_eq!(m1_cost.input, base_cost.input);
    assert_eq!(m1_cost.output, base_cost.output);
    assert_eq!(m1_cost.cache_read, base_cost.cache_read);
    assert!(m1_cost.cost_tiers.is_empty());
}

#[test]
fn test_claude_opus_5_5_1m_variant() {
    let base = get_model_profile("anthropic", "claude-opus-5-5").unwrap();
    assert_eq!(base.name, "Claude Opus 5.5");
    assert!(base.reasoning);
    assert!(!base.temperature);
    let base_cost = base.cost.as_ref().unwrap();
    assert_eq!((base_cost.input, base_cost.output), (4.00, 20.00));
    assert_eq!(base_cost.cache_read, Some(0.20));
    assert_eq!(base.limits.as_ref().unwrap().context, 200_000);

    let m1 = get_model_profile("anthropic", "claude-opus-5-5[1m]").unwrap();
    assert_eq!(m1.name, "Claude Opus 5.5 (1M)");
    assert_eq!(m1.family, "claude-opus-5-5");
    assert_eq!(m1.limits.as_ref().unwrap().context, 1_000_000);
    assert_eq!(m1.limits.as_ref().unwrap().output, 128_000);

    // Flat standard pricing — the 1M window carries no long-context premium,
    // so cost matches the 200K base exactly.
    let (base_cost, m1_cost) = (base.cost.unwrap(), m1.cost.unwrap());
    assert_eq!(m1_cost.input, base_cost.input);
    assert_eq!(m1_cost.output, base_cost.output);
    assert_eq!(m1_cost.cache_read, base_cost.cache_read);
    assert!(m1_cost.cost_tiers.is_empty());
}

#[test]
fn test_claude_fable_5_1m_variant() {
    let base = get_model_profile("anthropic", "claude-fable-5").unwrap();
    assert_eq!(base.limits.as_ref().unwrap().context, 200_000);

    let m1 = get_model_profile("anthropic", "claude-fable-5[1m]").unwrap();
    assert_eq!(m1.name, "Claude Fable 5 (1M)");
    assert_eq!(m1.family, "claude-fable-5");
    assert_eq!(m1.limits.as_ref().unwrap().context, 1_000_000);
    assert_eq!(m1.cost.unwrap().input, base.cost.unwrap().input);
}

#[test]
fn test_clear_at_capability_is_profile_gated() {
    for id in [
        "claude-fable-5-1",
        "claude-fable-5",
        "claude-opus-5-5",
        "claude-opus-5",
        "claude-opus-4-8",
        "claude-opus-5-5[1m]",
        "claude-opus-5-5-20260101[1m]",
        "claude-fable-5-1-20260901[1m]",
    ] {
        let profile = get_model_profile("anthropic", id).unwrap();
        assert!(
            profile.supports_parameter(MID_CONVERSATION_SYSTEM_PARAMETER),
            "{id}"
        );
        assert!(profile.supports_parameter(CLEAR_AT_PARAMETER), "{id}");
    }

    for id in [
        "claude-opus-5-5-20260101[1m]",
        "claude-fable-5-1-20260901[1m]",
    ] {
        let profile = get_model_profile("anthropic", id).unwrap();
        assert_eq!(profile.limits.unwrap().context, 1_000_000, "{id}");
    }

    for id in [
        "claude-opus-4-7",
        "claude-opus-4-6",
        "claude-sonnet-5",
        "claude-sonnet-4-6",
        "claude-haiku-4-5",
    ] {
        let profile = get_model_profile("anthropic", id).unwrap();
        assert!(
            !profile.supports_parameter(MID_CONVERSATION_SYSTEM_PARAMETER),
            "{id}"
        );
        assert!(!profile.supports_parameter(CLEAR_AT_PARAMETER), "{id}");
    }
}

#[test]
fn test_claude_fable_5_1_profile_and_1m_variant() {
    // `claude-fable-5-1` is its own family: the `-1` is not a version
    // suffix, so it must not fall back to the Fable 5 descriptor.
    let base = get_model_profile("anthropic", "claude-fable-5-1").unwrap();
    assert_eq!(base.name, "Claude Fable 5.1");
    assert_eq!(base.family, "claude-fable-5-1");
    assert_eq!(base.limits.as_ref().unwrap().context, 200_000);
    assert!(base.reasoning);
    assert!(!base.temperature);
    assert!(base.tool_search);
    let base_cost = base.cost.as_ref().unwrap();
    let fable_5_cost = get_model_profile("anthropic", "claude-fable-5")
        .unwrap()
        .cost
        .unwrap();
    // Same per-token price as Fable 5; cache reads are a quarter of it.
    assert_eq!(base_cost.input, fable_5_cost.input);
    assert_eq!(base_cost.output, fable_5_cost.output);
    assert_eq!(base_cost.cache_read, Some(0.25));

    let m1 = get_model_profile("anthropic", "claude-fable-5-1[1m]").unwrap();
    assert_eq!(m1.name, "Claude Fable 5.1 (1M)");
    assert_eq!(m1.family, "claude-fable-5-1");
    assert_eq!(m1.limits.as_ref().unwrap().context, 1_000_000);
    assert_eq!(m1.cost.unwrap().input, base_cost.input);
}

#[test]
fn test_claude_opus_4_7_and_4_6_have_1m_variants() {
    for id in ["claude-opus-4-7[1m]", "claude-opus-4-6[1m]"] {
        let m1 = get_model_profile("anthropic", id).unwrap();
        assert_eq!(m1.limits.as_ref().unwrap().context, 1_000_000);
        assert!(m1.name.ends_with("(1M)"));
    }
}

#[test]
fn test_claude_sonnet_5_1m_variant() {
    let base = get_model_profile("anthropic", "claude-sonnet-5").unwrap();
    assert_eq!(base.limits.as_ref().unwrap().context, 200_000);

    let m1 = get_model_profile("anthropic", "claude-sonnet-5[1m]").unwrap();
    assert_eq!(m1.name, "Claude Sonnet 5 (1M)");
    assert_eq!(m1.family, "claude-sonnet-5");
    assert_eq!(m1.limits.as_ref().unwrap().context, 1_000_000);
    assert_eq!(m1.cost.unwrap().input, base.cost.unwrap().input);
}
