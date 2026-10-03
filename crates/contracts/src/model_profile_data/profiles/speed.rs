// Speed (service tier) configurations and tier-aware cost estimates.
//
// Availability is sourced from OpenAI's official tier tables: the API pricing
// page for Flex, the Priority processing page for first-party priority models,
// and the specialized pricing table for Codex priority. A model gets a speed
// config only when it has a Flex and/or Priority row. Chat-latest,
// deep-research, and unlisted variants have no speed config. Display names
// follow Codex's speed selector ("Fast" for priority).
//
// Tier prices: a `SpeedValue` carries a `cost_multiplier` only where OpenAI
// states the tier as a flat multiple of Standard for every token bucket
// (GPT-6 series: Flex 0.5x, Fast 2x, Ultrafast 6x; developers.openai.com
// guides/fast-mode and guides/ultrafast-mode). Older models' Priority rows
// are not a flat multiple (gpt-4.1 priority is 1.75x, gpt-5-mini 1.8x), so
// they carry none and keep being estimated at the Standard rate.

use super::estimate_cost_usd;
use crate::model_profile_data::types::{Speed, SpeedConfig, SpeedValue};

pub(super) fn speed(value: Speed, name: &str) -> SpeedValue {
    SpeedValue {
        value,
        name: name.into(),
        cost_multiplier: None,
    }
}

pub(super) fn priced_speed(value: Speed, name: &str, multiplier: f64) -> SpeedValue {
    SpeedValue {
        cost_multiplier: Some(multiplier),
        ..speed(value, name)
    }
}

/// Speed for models with both flex and priority pricing rows
/// (gpt-5.4, gpt-5.4-mini, gpt-5.5, gpt-5.6 series).
pub(super) fn speed_flex_priority() -> SpeedConfig {
    SpeedConfig {
        values: vec![
            speed(Speed::Flex, "Flex"),
            speed(Speed::Default, "Standard"),
            speed(Speed::Priority, "Fast"),
        ],
        default: Speed::Default,
    }
}

/// Speed for models with only a flex pricing row
/// (gpt-5.4-nano, gpt-5.4-pro, gpt-5.5-pro).
pub(super) fn speed_flex_only() -> SpeedConfig {
    SpeedConfig {
        values: vec![
            speed(Speed::Flex, "Flex"),
            speed(Speed::Default, "Standard"),
        ],
        default: Speed::Default,
    }
}

/// Speed for models with only a priority pricing row
/// (gpt-4.1 family, gpt-5/gpt-5-mini,
/// gpt-5-codex, gpt-5.1/gpt-5.1-codex, gpt-5.2, gpt-5.3-codex,
/// o3, o4-mini).
pub(super) fn speed_priority_only() -> SpeedConfig {
    SpeedConfig {
        values: vec![
            speed(Speed::Default, "Standard"),
            speed(Speed::Priority, "Fast"),
        ],
        default: Speed::Default,
    }
}

/// [`estimate_cost_usd`] priced at the service tier that served the request.
///
/// `served_tier` is the provider's echoed `service_tier` (OpenAI returns the
/// tier actually used, which can differ from the one requested when a premium
/// tier is ramp-limited and degrades to `default`). The standard estimate is
/// scaled by that tier's `cost_multiplier`; an unknown tier, a tier without a
/// recorded multiplier, or no tier at all prices at Standard.
pub fn estimate_cost_usd_for_speed(
    provider_type: &str,
    model_id: &str,
    input_tokens: u32,
    output_tokens: u32,
    cache_read_tokens: u32,
    cache_creation_tokens: u32,
    served_tier: Option<&str>,
) -> Option<f64> {
    let standard = estimate_cost_usd(
        provider_type,
        model_id,
        input_tokens,
        output_tokens,
        cache_read_tokens,
        cache_creation_tokens,
    )?;
    let multiplier = served_tier
        .and_then(|tier| {
            super::get_model_profile(provider_type, model_id)?
                .speed?
                .values
                .into_iter()
                .find(|value| value.value.matches_tier(tier))?
                .cost_multiplier
        })
        .unwrap_or(1.0);
    Some(standard * multiplier)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn fast_and_priority_are_one_tier() {
        for speed in [Speed::Fast, Speed::Priority] {
            assert!(speed.matches_tier("fast"));
            assert!(speed.matches_tier("priority"));
            assert!(!speed.matches_tier("ultrafast"));
        }
        assert!(Speed::Ultrafast.matches_tier("ultrafast"));
        assert!(!Speed::Default.matches_tier("flex"));
    }

    #[test]
    fn served_tier_scales_the_standard_estimate() {
        let tokens = (100_000, 100_000, 0, 0);
        let at = |model: &str, tier: Option<&str>| {
            estimate_cost_usd_for_speed("openai", model, tokens.0, tokens.1, 0, 0, tier).unwrap()
        };
        // GPT-6 Astra: $10 in / $50 out standard.
        let standard = 1.0 + 5.0;
        let close = |a: f64, b: f64| (a - b).abs() < 1e-9;
        assert!(close(at("gpt-6-astra", None), standard));
        assert!(close(at("gpt-6-astra", Some("default")), standard));
        assert!(close(at("gpt-6-astra", Some("flex")), standard * 0.5));
        assert!(close(at("gpt-6-astra", Some("priority")), standard * 2.0));
        assert!(close(at("gpt-6-astra", Some("fast")), standard * 2.0));
        assert!(close(at("gpt-6-astra", Some("ultrafast")), standard * 6.0));
        // 6.1 Sol has no Ultrafast yet: an unexpected echo prices at Standard.
        assert!(close(at("gpt-6.1-sol", Some("ultrafast")), 0.2 + 1.0));
        assert!(close(at("gpt-6.1-sol", Some("fast")), (0.2 + 1.0) * 2.0));
        // Older priority rows are not a flat multiple, so they stay Standard.
        let gpt55 = estimate_cost_usd("openai", "gpt-5.5", tokens.0, tokens.1, 0, 0);
        assert_eq!(
            estimate_cost_usd_for_speed(
                "openai",
                "gpt-5.5",
                tokens.0,
                tokens.1,
                0,
                0,
                Some("priority")
            ),
            gpt55
        );
    }

    #[test]
    fn tier_multiplier_applies_on_top_of_the_long_context_tier() {
        // Ultrafast Astra above 272K: 2x input and 6x tier, per the pricing table.
        let cost = estimate_cost_usd_for_speed(
            "openai",
            "gpt-6-astra",
            300_000,
            0,
            0,
            0,
            Some("ultrafast"),
        )
        .unwrap();
        assert!((cost - 0.3 * 20.0 * 6.0).abs() < 1e-9, "{cost}");
    }
}
