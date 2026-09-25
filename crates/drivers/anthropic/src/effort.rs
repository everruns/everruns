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
/// Thinking counts toward `max_tokens`, so a caller cap sized for the visible
/// answer (a 64-token classifier, a 700-token judge) is spent entirely on
/// thinking and comes back empty. A caller's cap is therefore treated as the
/// answer budget and thinking room is added on top: the budget itself for
/// budget-based thinking, or an effort-sized allowance for adaptive thinking,
/// capped at the model's output limit. With no caller cap the model's output
/// limit is used as-is, which already leaves room for thinking.
pub(crate) fn max_tokens(
    caller: Option<u32>,
    profile: Option<&ModelProfile>,
    thinking_budget: Option<u32>,
    adaptive_effort: Option<&str>,
) -> u32 {
    let model_limit = profile
        .and_then(|p| p.limits.as_ref())
        .and_then(|l| u32::try_from(l.output).ok())
        .filter(|v| *v > 0);
    let base = caller.or(model_limit).unwrap_or(16_384);
    if let Some(budget) = thinking_budget {
        // The API requires `max_tokens` above the budget.
        return base.max(budget + 1024);
    }
    let Some(caller) = caller else {
        return base;
    };
    let room = match adaptive_effort {
        Some("low") => 4_096,
        Some("medium") => 8_192,
        Some("high") => 16_384,
        Some(_) => 32_768,
        None => return caller,
    };
    let wanted = caller.saturating_add(room);
    model_limit.map_or(wanted, |limit| wanted.min(limit.max(caller)))
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
    fn a_caller_cap_gets_thinking_room_on_top() {
        let profile = everruns_provider::get_model_profile(&DriverId::Anthropic, "claude-opus-5-5");
        let p = profile.as_ref();
        // Adaptive thinking: the caller's cap stays the answer budget.
        assert_eq!(max_tokens(Some(64), p, None, Some("low")), 64 + 4_096);
        assert_eq!(max_tokens(Some(700), p, None, Some("high")), 700 + 16_384);
        assert_eq!(max_tokens(Some(64), p, None, Some("max")), 64 + 32_768);
        // Never above the model's output limit, never below the caller's cap.
        assert_eq!(max_tokens(Some(127_000), p, None, Some("max")), 128_000);
        assert_eq!(max_tokens(Some(200_000), p, None, Some("max")), 200_000);
        // No thinking: the caller's cap as-is. No cap: the model limit.
        assert_eq!(max_tokens(Some(64), p, None, None), 64);
        assert_eq!(max_tokens(None, p, None, Some("high")), 128_000);
        assert_eq!(max_tokens(None, None, None, None), 16_384);
        // Budget-based thinking keeps the API's "above the budget" rule.
        assert_eq!(max_tokens(Some(64), p, Some(4_096), None), 5_120);
        assert_eq!(max_tokens(None, None, Some(32_768), None), 33_792);
    }

    #[test]
    fn a_missing_profile_sends_nothing() {
        assert_eq!(default_effort("claude-opus-5-5", None), None);
    }
}
