//! Output-truncation policy: what a turn does when a model response loses tool
//! calls because it hit the output-token limit.
//!
//! Why: a response that ends with finish reason `length` can stop in the middle
//! of a tool call. Drivers never run such a call (and never substitute `{}` for
//! its arguments); they drop it and count it in
//! `LlmCompletionMetadata::tool_calls_dropped`. Without a gate the turn then
//! ended on a half-written answer, the model never learning its call did not
//! run. The same applies to a call whose arguments did not parse (finish reason
//! `tool_calls`), which a driver now drops the same way.
//!
//! Decision: configurable per agent or harness through the `output_truncation`
//! capability, defaulting to [`OutputTruncationPolicy::Continue`] with
//! [`DEFAULT_MAX_RETRIES`] consecutive retries, so an agent without the
//! capability gets the gate. `off` keeps the previous turn behaviour (the turn
//! just ends); calls with complete arguments still run under every policy.

use everruns_contracts::CapabilityRef;
use serde::{Deserialize, Serialize};

/// Capability id whose config selects the policy.
pub const OUTPUT_TRUNCATION_CAPABILITY_ID: &str = "output_truncation";

/// Consecutive truncated generations a turn retries before it fails.
pub const DEFAULT_MAX_RETRIES: u32 = 2;

/// Upper bound for `max_retries`: each retry is a full model call.
pub const MAX_RETRIES_LIMIT: u32 = 10;

/// What a turn does when a generation loses tool calls to truncation.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum OutputTruncationPolicy {
    /// Tell the model its calls did not run and run another generation, up to
    /// `max_retries` consecutive times; then fail the turn.
    #[default]
    Continue,
    /// Fail the turn with an error.
    Fail,
    /// Previous behaviour: the lost calls are not run and the turn carries on
    /// (or ends) as if the model had not asked for them.
    Off,
}

impl OutputTruncationPolicy {
    /// The config spelling (`continue`, `fail`, `off`).
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Continue => "continue",
            Self::Fail => "fail",
            Self::Off => "off",
        }
    }
}

fn default_max_retries() -> u32 {
    DEFAULT_MAX_RETRIES
}

/// Resolved `output_truncation` capability config.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct OutputTruncationConfig {
    /// The policy; `continue` when unset.
    #[serde(default)]
    pub policy: OutputTruncationPolicy,
    /// Consecutive truncated generations `continue` retries before failing.
    #[serde(default = "default_max_retries")]
    pub max_retries: u32,
}

impl Default for OutputTruncationConfig {
    fn default() -> Self {
        Self {
            policy: OutputTruncationPolicy::default(),
            max_retries: DEFAULT_MAX_RETRIES,
        }
    }
}

/// What the gate does with one generation.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum OutputTruncationAction {
    /// Nothing was lost, or the policy is `off`: carry on as usual.
    Proceed,
    /// Tell the model and run another generation.
    Retry,
    /// End the turn with an error.
    Fail,
}

impl OutputTruncationConfig {
    /// Parse a capability config. `null` and `{}` are the defaults.
    pub fn from_capability_config(config: &serde_json::Value) -> Result<Self, String> {
        if config.is_null() {
            return Ok(Self::default());
        }
        if !config.is_object() {
            return Err("output_truncation config must be an object".to_string());
        }
        let parsed: Self = serde_json::from_value(config.clone())
            .map_err(|error| format!("invalid output_truncation config: {error}"))?;
        if parsed.max_retries > MAX_RETRIES_LIMIT {
            return Err(format!(
                "output_truncation max_retries must be at most {MAX_RETRIES_LIMIT}, got {}",
                parsed.max_retries
            ));
        }
        Ok(parsed)
    }

    /// The config in effect for a resolved capability list: the last
    /// `output_truncation` entry (the most specific layer), else the default.
    /// A malformed runtime config falls back to the default rather than
    /// disabling the gate; the write path already rejects it.
    pub fn resolve(capabilities: &[CapabilityRef]) -> Self {
        capabilities
            .iter()
            .rev()
            .find(|capability| capability.capability_id() == OUTPUT_TRUNCATION_CAPABILITY_ID)
            .map(|capability| {
                Self::from_capability_config(capability.config_value()).unwrap_or_else(|error| {
                    tracing::warn!(%error, "output_truncation: falling back to the default");
                    Self::default()
                })
            })
            .unwrap_or_default()
    }

    /// Decide for one generation. `finish_reason` is the normalized reason,
    /// `tool_calls_lost` the calls the driver dropped, and `prior` how many
    /// generations immediately before this one in the turn also lost calls.
    ///
    /// Only `length` (cut off at the output limit) and `tool_calls` (the model
    /// ended on calls, some unparseable) count: a call dropped because the
    /// response was refused or filtered is not something a retry fixes.
    pub fn decide(
        &self,
        finish_reason: &str,
        tool_calls_lost: u32,
        prior: u32,
    ) -> OutputTruncationAction {
        if tool_calls_lost == 0 || !matches!(finish_reason, "length" | "tool_calls") {
            return OutputTruncationAction::Proceed;
        }
        match self.policy {
            OutputTruncationPolicy::Off => OutputTruncationAction::Proceed,
            OutputTruncationPolicy::Fail => OutputTruncationAction::Fail,
            OutputTruncationPolicy::Continue if prior >= self.max_retries => {
                OutputTruncationAction::Fail
            }
            OutputTruncationPolicy::Continue => OutputTruncationAction::Retry,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn config_parses_defaults_and_rejects_bad_values() {
        for (config, expected) in [
            (json!(null), Some(OutputTruncationConfig::default())),
            (json!({}), Some(OutputTruncationConfig::default())),
            (
                json!({"policy": "fail"}),
                Some(OutputTruncationConfig {
                    policy: OutputTruncationPolicy::Fail,
                    max_retries: 2,
                }),
            ),
            (
                json!({"policy": "off", "max_retries": 0}),
                Some(OutputTruncationConfig {
                    policy: OutputTruncationPolicy::Off,
                    max_retries: 0,
                }),
            ),
            (json!({"policy": "loud"}), None),
            (json!({"policy": "Continue"}), None),
            (json!({"max_retries": 11}), None),
            (json!({"max_retries": -1}), None),
            (json!({"retries": 1}), None),
            (json!("continue"), None),
            (json!([]), None),
        ] {
            assert_eq!(
                OutputTruncationConfig::from_capability_config(&config).ok(),
                expected,
                "{config}"
            );
        }
    }

    #[test]
    fn resolve_takes_the_last_entry_and_defaults_otherwise() {
        assert_eq!(
            OutputTruncationConfig::resolve(&[]),
            OutputTruncationConfig::default()
        );
        let harness =
            CapabilityRef::with_config(OUTPUT_TRUNCATION_CAPABILITY_ID, json!({"policy": "fail"}));
        let agent =
            CapabilityRef::with_config(OUTPUT_TRUNCATION_CAPABILITY_ID, json!({"policy": "off"}));
        assert_eq!(
            OutputTruncationConfig::resolve(&[harness.clone(), agent]).policy,
            OutputTruncationPolicy::Off
        );
        let broken =
            CapabilityRef::with_config(OUTPUT_TRUNCATION_CAPABILITY_ID, json!({"policy": 1}));
        assert_eq!(
            OutputTruncationConfig::resolve(&[harness, broken]),
            OutputTruncationConfig::default()
        );
    }

    #[test]
    fn decide_retries_up_to_the_limit_then_fails() {
        use OutputTruncationAction::*;
        let continue_ = OutputTruncationConfig::default();
        assert_eq!(continue_.decide("length", 1, 0), Retry);
        assert_eq!(continue_.decide("length", 1, 1), Retry);
        assert_eq!(continue_.decide("length", 1, 2), Fail);
        assert_eq!(continue_.decide("tool_calls", 2, 0), Retry);
        // Nothing lost, or lost to a refusal: not the gate's business.
        assert_eq!(continue_.decide("length", 0, 5), Proceed);
        assert_eq!(continue_.decide("content_filter", 1, 0), Proceed);
        assert_eq!(continue_.decide("refusal", 1, 0), Proceed);

        let fail = OutputTruncationConfig {
            policy: OutputTruncationPolicy::Fail,
            ..Default::default()
        };
        assert_eq!(fail.decide("length", 1, 0), Fail);
        let off = OutputTruncationConfig {
            policy: OutputTruncationPolicy::Off,
            ..Default::default()
        };
        assert_eq!(off.decide("length", 3, 0), Proceed);
        let no_retries = OutputTruncationConfig {
            max_retries: 0,
            ..Default::default()
        };
        assert_eq!(no_retries.decide("length", 1, 0), Fail);
    }
}
