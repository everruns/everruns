//! Default effort for Claude families whose thinking cannot be disabled, split
//! out of `driver.rs` to keep that file under its size ratchet.

use everruns_provider::LlmCallConfig;
use everruns_provider::model::{ModelProfile, ReasoningEffort};

use crate::driver::normalize_anthropic_id;

/// Families where thinking cannot be disabled: omitting `thinking` still runs
/// adaptive thinking at the API's default effort.
const THINKING_ALWAYS_ON_FAMILIES: &[&str] =
    &["claude-fable-5-1", "claude-fable-5", "claude-opus-5-5"];

/// The caller's effort, or [`default_effort`] when it chose none.
pub(crate) fn resolve(
    config: &LlmCallConfig,
    wire_model: &str,
    profile: &Option<ModelProfile>,
) -> Option<ReasoningEffort> {
    config
        .reasoning_effort
        .or_else(|| default_effort(wire_model, profile.as_ref()))
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
    let family = normalize_anthropic_id(wire_model);
    if !THINKING_ALWAYS_ON_FAMILIES
        .iter()
        .any(|f| family.eq_ignore_ascii_case(f))
    {
        return None;
    }
    profile?.reasoning_effort.as_ref().map(|r| r.default)
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
    fn a_missing_profile_sends_nothing() {
        assert_eq!(default_effort("claude-opus-5-5", None), None);
    }
}
