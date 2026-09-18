//! Tests moved out of events.rs: token_usage_tests.

use super::TokenUsage;

#[test]
fn effective_cost_precedence_preserves_zero_and_missing_values() {
    for (actual, estimated, explicit, expected) in [
        (None, None, None, None),
        (None, Some(2.0), None, Some(2.0)),
        (Some(1.0), Some(2.0), None, Some(1.0)),
        (Some(0.0), Some(2.0), None, Some(0.0)),
        (Some(1.0), Some(2.0), Some(3.0), Some(3.0)),
        (Some(1.0), Some(2.0), Some(0.0), Some(0.0)),
    ] {
        let usage = TokenUsage::new(10, 5)
            .with_cost(actual, estimated)
            .with_effective_cost(explicit);
        assert_eq!(usage.actual_cost_usd, actual);
        assert_eq!(usage.estimated_cost_usd, estimated);
        assert_eq!(usage.effective_cost_usd(), expected);
    }
}

#[test]
fn add_seeds_effective_cost_from_accumulator_implicit_cost() {
    let mut aggregate = TokenUsage::new(10, 5).with_cost(Some(1.0), Some(10.0));
    let generation = TokenUsage::new(20, 10).with_cost(None, Some(2.0));

    aggregate.add(&generation);

    assert_eq!(aggregate.actual_cost_usd, Some(1.0));
    assert_eq!(aggregate.estimated_cost_usd, Some(12.0));
    assert_eq!(aggregate.effective_cost_usd(), Some(3.0));
}

#[test]
fn add_effective_cost_does_not_double_count_when_actual_is_mutated() {
    // Neither side carries an explicit effective total, so each side's
    // effective cost falls back to `actual`. Adding must sum the two
    // actuals once (1.0 + 3.0), not fold the accumulator's post-add actual
    // back into the effective total.
    let mut aggregate = TokenUsage::new(10, 5).with_cost(Some(1.0), None);
    let generation = TokenUsage::new(20, 10).with_cost(Some(3.0), None);

    aggregate.add(&generation);

    assert_eq!(aggregate.actual_cost_usd, Some(4.0));
    assert_eq!(aggregate.effective_cost_usd(), Some(4.0));
}

#[test]
fn aggregate_token_counters_saturate_at_their_bound() {
    let mut ordinary = TokenUsage::with_cache(7, 3, None, Some(2));
    ordinary.add(&TokenUsage::with_cache(11, 5, Some(4), None));
    assert_eq!((ordinary.input_tokens, ordinary.output_tokens), (18, 8));
    assert_eq!(ordinary.total_tokens(), 26);
    assert_eq!(ordinary.cache_read_tokens, Some(4));
    assert_eq!(ordinary.cache_creation_tokens, Some(2));

    let mut aggregate = TokenUsage::with_cache(
        u32::MAX - 1,
        u32::MAX - 1,
        Some(u32::MAX - 1),
        Some(u32::MAX - 1),
    );
    aggregate.add(&TokenUsage::with_cache(10, 10, Some(10), Some(10)));

    assert_eq!(aggregate.input_tokens, u32::MAX);
    assert_eq!(aggregate.output_tokens, u32::MAX);
    assert_eq!(aggregate.cache_read_tokens, Some(u32::MAX));
    assert_eq!(aggregate.cache_creation_tokens, Some(u32::MAX));
    assert_eq!(aggregate.total_tokens(), u32::MAX);
}
