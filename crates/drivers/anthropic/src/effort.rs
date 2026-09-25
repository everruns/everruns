//! Effort and output-cap rules for Claude's thinking models, split out of
//! `driver.rs` to keep that file under its size ratchet.

use everruns_provider::LlmCallConfig;
use everruns_provider::model::{ModelProfile, ReasoningEffort};

use crate::driver::normalize_anthropic_id;

/// Families where thinking cannot be disabled: omitting `thinking` still runs
/// adaptive thinking at the API's default effort.
const THINKING_ALWAYS_ON_FAMILIES: &[&str] =
    &["claude-fable-5-1", "claude-fable-5", "claude-opus-5-5"];

/// The effort to send: the caller's, or [`default_effort`] when it chose none.
///
/// On families where thinking cannot be disabled, an explicit
/// `ReasoningEffort::None` becomes `Low`, the closest request the API accepts.
/// Sending nothing would run at the API default (`medium` on Opus 5.5), the
/// opposite of what the caller asked for.
pub(crate) fn resolve(
    config: &LlmCallConfig,
    wire_model: &str,
    profile: &Option<ModelProfile>,
) -> Option<ReasoningEffort> {
    match config.reasoning_effort {
        Some(ReasoningEffort::None) if thinking_always_on(wire_model) => Some(ReasoningEffort::Low),
        Some(effort) => Some(effort),
        None => default_effort(wire_model, profile.as_ref()),
    }
}

/// The request's `max_tokens`.
///
/// An explicit caller value is a hard cap on all generated tokens, including
/// thinking. Without one, the model's output limit is used as-is. The fallback
/// for budget-based thinking must exceed its thinking budget because the API
/// rejects requests that do not leave room for an answer.
/// Output room an adaptive-thinking request needs, by effort level.
///
/// Adaptive thinking carries no `budget_tokens` to measure against, so this is
/// the allowance the driver previously added on top of a caller's cap, reused
/// as the test for whether thinking fits underneath one.
pub(crate) fn adaptive_room(effort: &str) -> u32 {
    match effort {
        "low" => 4_096,
        "medium" => 8_192,
        "high" => 16_384,
        _ => 32_768,
    }
}

/// Whether a thinking configuration fits under an explicit caller cap.
///
/// Thinking counts toward `max_tokens`. A cap sized for the visible answer (a
/// 64-token classifier, a 700-token judge) would otherwise be spent entirely on
/// thinking and come back empty, so the caller asks for more than the cap can
/// give. Rather than overrun a limit the caller set deliberately, the driver
/// drops thinking when this returns false and spends the whole cap on the
/// answer.
pub(crate) fn thinking_fits(cap: u32, budget: Option<u32>, adaptive_effort: Option<&str>) -> bool {
    match (budget, adaptive_effort) {
        // The API rejects a request whose `max_tokens` does not exceed the budget.
        (Some(budget), _) => cap > budget,
        (None, Some(effort)) => cap > adaptive_room(effort),
        (None, None) => true,
    }
}

pub(crate) fn max_tokens(
    caller: Option<u32>,
    profile: Option<&ModelProfile>,
    thinking_budget: Option<u32>,
) -> u32 {
    if let Some(caller) = caller {
        return caller;
    }

    let model_limit = profile
        .and_then(|p| p.limits.as_ref())
        .and_then(|l| u32::try_from(l.output).ok())
        .filter(|v| *v > 0);
    let base = model_limit.unwrap_or(16_384);
    if let Some(budget) = thinking_budget {
        // The API requires `max_tokens` above the budget.
        return base.max(budget + 1024);
    }
    base
}

/// The effort to send when the caller chose none.
///
/// Opus 5.5 and Fable 5.x think whether or not `thinking` is sent, and Opus
/// 5.5's API default effort (`medium`) sits below the profile's. Sending the
/// profile default keeps effort explicit instead of silently falling back to
/// the API's. Other families return `None`: omitting `thinking` there turns it
/// off, which is what a caller that set no effort gets today.
pub(crate) fn default_effort(
    wire_model: &str,
    profile: Option<&ModelProfile>,
) -> Option<ReasoningEffort> {
    if !thinking_always_on(wire_model) {
        return None;
    }
    profile?.reasoning_effort.as_ref().map(|r| r.default)
}

fn thinking_always_on(wire_model: &str) -> bool {
    let family = normalize_anthropic_id(wire_model);
    THINKING_ALWAYS_ON_FAMILIES
        .iter()
        .any(|f| family.eq_ignore_ascii_case(f))
}

#[cfg(test)]
mod tests {
    use super::*;
    use everruns_provider::DriverId;

    fn effort_for(model: &str) -> Option<ReasoningEffort> {
        let profile = everruns_provider::get_model_profile(&DriverId::Anthropic, model);
        default_effort(model, profile.as_ref())
    }

    #[test]
    fn thinking_always_on_families_get_the_profile_default() {
        for model in ["claude-opus-5-5", "claude-fable-5-1", "claude-fable-5"] {
            assert_eq!(effort_for(model), Some(ReasoningEffort::High), "{model}");
        }
    }

    /// Either omitting `thinking` turns it off, or the API default already
    /// matches the profile, so no effort is invented.
    #[test]
    fn other_families_get_none() {
        for model in ["claude-opus-4-8", "claude-sonnet-5", "claude-sonnet-4-5"] {
            assert_eq!(effort_for(model), None, "{model}");
        }
    }

    #[test]
    fn explicit_none_becomes_low_where_thinking_cannot_be_off() {
        let profile = |m| everruns_provider::get_model_profile(&DriverId::Anthropic, m);
        let mut config = LlmCallConfig::new("claude-opus-5-5");
        config.reasoning_effort = Some(ReasoningEffort::None);
        let opus55 = resolve(&config, "claude-opus-5-5", &profile("claude-opus-5-5"));
        assert_eq!(opus55, Some(ReasoningEffort::Low));
        // Where omitting `thinking` turns it off, `None` keeps meaning off.
        let opus48 = resolve(&config, "claude-opus-4-8", &profile("claude-opus-4-8"));
        assert_eq!(opus48, Some(ReasoningEffort::None));
    }

    #[test]
    fn an_explicit_caller_cap_is_never_expanded() {
        let profile = everruns_provider::get_model_profile(&DriverId::Anthropic, "claude-opus-5-5");
        let p = profile.as_ref();
        // Adaptive and budget-based thinking both stay within the caller's
        // resource limit, even when the model supports a larger output.
        assert_eq!(max_tokens(Some(64), p, None), 64);
        assert_eq!(max_tokens(Some(700), p, None), 700);
        assert_eq!(max_tokens(Some(200_000), p, None), 200_000);
        assert_eq!(max_tokens(Some(64), p, Some(4_096)), 64);

        // No cap uses the model limit. The missing-profile fallback still
        // accommodates budget-based thinking so Anthropic accepts the request.
        assert_eq!(max_tokens(None, p, None), 128_000);
        assert_eq!(max_tokens(None, None, None), 16_384);
        assert_eq!(max_tokens(None, None, Some(32_768)), 33_792);
    }

    #[test]
    fn thinking_has_to_fit_under_an_explicit_cap() {
        // Budget-based: the API needs max_tokens strictly above the budget.
        assert!(!thinking_fits(4_096, Some(4_096), None));
        assert!(!thinking_fits(64, Some(4_096), None));
        assert!(thinking_fits(4_097, Some(4_096), None));

        // Adaptive carries no budget, so the effort-sized room is the test.
        // Without this, a 64-token cap on an adaptive model keeps thinking on
        // and the answer comes back empty.
        assert!(!thinking_fits(64, None, Some("low")));
        assert!(!thinking_fits(4_096, None, Some("low")));
        assert!(thinking_fits(4_097, None, Some("low")));
        assert!(!thinking_fits(16_384, None, Some("high")));
        assert!(!thinking_fits(32_768, None, Some("max")));
        assert!(thinking_fits(32_769, None, Some("max")));

        // No thinking always fits.
        assert!(thinking_fits(1, None, None));
    }

    #[test]
    fn a_missing_profile_sends_nothing() {
        assert_eq!(default_effort("claude-opus-5-5", None), None);
    }
}
