//! Claude `[1m]` twin and flagship profile tests, split out of `profiles.rs`
//! to keep that file under its size ratchet.

use super::*;
use crate::model_profile_data::{CLEAR_AT_PARAMETER, MID_CONVERSATION_SYSTEM_PARAMETER};

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
fn threshold_server_compaction_is_explicitly_profile_gated() {
    for id in [
        "claude-fable-5-1",
        "claude-fable-5-20260901",
        "claude-opus-5",
        "claude-opus-4-8-20260101[1m]",
        "claude-opus-4-7",
        "claude-opus-4-6[1m]",
        "claude-sonnet-5-latest",
        "claude-sonnet-4-6-20260217",
    ] {
        assert!(
            get_model_profile("anthropic", id)
                .unwrap()
                .supports_server_compaction,
            "{id}"
        );
    }

    for id in [
        "claude-opus-5-5",
        "claude-opus-5-5[1m]",
        "claude-sonnet-5-5",
        "claude-sonnet-5-5[1m]",
        "claude-haiku-5-5",
        "claude-haiku-5-5[1m]",
        "claude-haiku-4-5",
    ] {
        assert!(
            !get_model_profile("anthropic", id)
                .unwrap()
                .supports_server_compaction,
            "{id}"
        );
    }
    assert!(get_model_profile("anthropic", "claude-unknown-future").is_none());
}

#[test]
fn test_clear_at_capability_is_profile_gated() {
    for id in [
        "claude-fable-5-1",
        "claude-fable-5",
        "claude-opus-5-5",
        "claude-opus-5",
        "claude-opus-4-8",
        "claude-sonnet-5-5",
        "claude-opus-5-5[1m]",
        "claude-sonnet-5-5[1m]",
        "claude-haiku-5-5",
        "claude-haiku-5-5[1m]",
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
fn one_hour_cache_writes_are_surcharged_up_to_twice_the_input_rate() {
    let cost = get_model_profile("anthropic", "claude-sonnet-5-5")
        .unwrap()
        .cost
        .unwrap();
    let charged = cost.cache_write.unwrap_or(cost.input);
    let surcharge =
        cache_write_1h_surcharge_usd("anthropic", "claude-sonnet-5-5", 60_000, 50_000).unwrap();
    assert!((surcharge - 0.05 * (2.0 * cost.input - charged)).abs() < 1e-12);
    // Together with the cache-write charge the long writes cost 2x input.
    let base = estimate_cost_usd("anthropic", "claude-sonnet-5-5", 0, 0, 0, 50_000).unwrap();
    assert!((base + surcharge - 0.05 * 2.0 * cost.input).abs() < 1e-12);

    assert_eq!(
        cache_write_1h_surcharge_usd("anthropic", "claude-sonnet-5-5", 60_000, 0),
        None
    );
    assert_eq!(
        cache_write_1h_surcharge_usd("anthropic", "no-such-model", 1, 1),
        None
    );
}
