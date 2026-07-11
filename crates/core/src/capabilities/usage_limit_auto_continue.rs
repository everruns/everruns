// Usage-Limit Auto-Continue capability
//
// When an LLM subscription/plan usage limit is hit — classified as
// `provider_usage_limit_reached` with a concrete `resets_at` timestamp (e.g. the
// ChatGPT/Codex `usage_limit_reached` body) — this capability makes the session
// resume on its own once the limit resets. It contributes no tools; its presence
// is the toggle, consumed by the reason atom's terminal-error path, which:
//
//   1. appends a "we'll continue automatically" promise to the user-facing error
//      copy (by setting the `auto_continue` error field), and
//   2. schedules a one-shot session continuation at `resets_at + delay_seconds`
//      that re-injects `prompt` to resume the interrupted work.
//
// The promise (1) is only added when the continuation (2) was actually
// scheduled, so the copy never over-promises.

use super::{Capability, CapabilityLocalization, CapabilityStatus, RiskLevel};
use crate::capability_types::AgentCapabilityConfig;
use serde_json::{Value, json};

pub const USAGE_LIMIT_AUTO_CONTINUE_CAPABILITY_ID: &str = "usage_limit_auto_continue";

/// Default delay, in seconds, after the reported reset time before the
/// continuation fires. The reset is a lower bound; a small buffer avoids racing
/// the provider's own clock (the motivating Codex case wants reset + 2 min).
pub const DEFAULT_CONTINUATION_DELAY_SECS: i64 = 120;

/// Default prompt re-injected to resume work once the limit resets.
pub const DEFAULT_CONTINUATION_PROMPT: &str = "Continue tasks";

/// Upper bound on the configurable delay (24h) so a misconfiguration cannot park
/// a continuation arbitrarily far in the future.
const MAX_CONTINUATION_DELAY_SECS: i64 = 86_400;

/// Resolved auto-continue settings for a turn.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AutoContinueConfig {
    /// Seconds to wait after `resets_at` before firing the continuation.
    pub delay_seconds: i64,
    /// Prompt injected as the continuation turn's user message.
    pub prompt: String,
}

impl Default for AutoContinueConfig {
    fn default() -> Self {
        Self {
            delay_seconds: DEFAULT_CONTINUATION_DELAY_SECS,
            prompt: DEFAULT_CONTINUATION_PROMPT.to_string(),
        }
    }
}

/// Resolve the auto-continue settings for a turn, or `None` when the capability
/// is not enabled for the agent/session. Mirrors `resolve_error_disclosure`:
/// config is read from the enabled capability's entry, falling back to defaults
/// for any missing or out-of-range field.
pub fn resolve_usage_limit_auto_continue(
    configs: &[AgentCapabilityConfig],
) -> Option<AutoContinueConfig> {
    let config = configs
        .iter()
        .find(|config| config.capability_id() == USAGE_LIMIT_AUTO_CONTINUE_CAPABILITY_ID)?;

    let delay_seconds = config
        .config
        .get("delay_seconds")
        .and_then(Value::as_i64)
        .filter(|secs| (0..=MAX_CONTINUATION_DELAY_SECS).contains(secs))
        .unwrap_or(DEFAULT_CONTINUATION_DELAY_SECS);

    let prompt = config
        .config
        .get("prompt")
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|p| !p.is_empty())
        .unwrap_or(DEFAULT_CONTINUATION_PROMPT)
        .to_string();

    Some(AutoContinueConfig {
        delay_seconds,
        prompt,
    })
}

pub struct UsageLimitAutoContinueCapability;

impl Capability for UsageLimitAutoContinueCapability {
    fn id(&self) -> &str {
        USAGE_LIMIT_AUTO_CONTINUE_CAPABILITY_ID
    }

    fn name(&self) -> &str {
        "Auto-Continue After Usage Limit"
    }

    fn description(&self) -> &str {
        "When an LLM usage limit is reached, automatically resume the interrupted work shortly after the limit resets."
    }

    fn localizations(&self) -> Vec<CapabilityLocalization> {
        vec![CapabilityLocalization::text(
            "uk",
            "Автопродовження після ліміту використання",
            "Коли досягнуто ліміт використання LLM, автоматично відновлює перервану роботу невдовзі після скидання ліміту.",
        )]
    }

    fn status(&self) -> CapabilityStatus {
        CapabilityStatus::Available
    }

    fn risk_level(&self) -> RiskLevel {
        RiskLevel::Low
    }

    fn icon(&self) -> Option<&str> {
        Some("clock")
    }

    fn category(&self) -> Option<&str> {
        Some("Core")
    }

    fn config_schema(&self) -> Option<Value> {
        Some(json!({
            "type": "object",
            "properties": {
                "delay_seconds": {
                    "type": "integer",
                    "title": "Continuation delay (seconds)",
                    "description": "How long to wait after the reported reset time before resuming work. A small buffer avoids racing the provider's reset clock.",
                    "minimum": 0,
                    "maximum": MAX_CONTINUATION_DELAY_SECS,
                    "default": DEFAULT_CONTINUATION_DELAY_SECS
                },
                "prompt": {
                    "type": "string",
                    "title": "Continuation prompt",
                    "description": "Message injected to resume the interrupted work when the limit resets.",
                    "default": DEFAULT_CONTINUATION_PROMPT
                }
            },
            "additionalProperties": false
        }))
    }

    fn validate_config(&self, config: &Value) -> Result<(), String> {
        if config.is_null() {
            return Ok(());
        }
        if let Some(delay) = config.get("delay_seconds") {
            let secs = delay
                .as_i64()
                .ok_or_else(|| "delay_seconds must be an integer".to_string())?;
            if !(0..=MAX_CONTINUATION_DELAY_SECS).contains(&secs) {
                return Err(format!(
                    "delay_seconds must be between 0 and {MAX_CONTINUATION_DELAY_SECS}"
                ));
            }
        }
        if let Some(prompt) = config.get("prompt")
            && !prompt.is_string()
        {
            return Err("prompt must be a string".to_string());
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn cap_config(config: Value) -> AgentCapabilityConfig {
        AgentCapabilityConfig {
            capability_ref: USAGE_LIMIT_AUTO_CONTINUE_CAPABILITY_ID.into(),
            config,
        }
    }

    #[test]
    fn resolve_returns_none_without_capability() {
        assert_eq!(resolve_usage_limit_auto_continue(&[]), None);
    }

    #[test]
    fn resolve_uses_defaults_for_empty_config() {
        let resolved = resolve_usage_limit_auto_continue(&[cap_config(json!({}))]).unwrap();
        assert_eq!(resolved, AutoContinueConfig::default());
    }

    #[test]
    fn resolve_reads_custom_delay_and_prompt() {
        let resolved = resolve_usage_limit_auto_continue(&[cap_config(json!({
            "delay_seconds": 300,
            "prompt": "  Resume the migration  "
        }))])
        .unwrap();
        assert_eq!(resolved.delay_seconds, 300);
        assert_eq!(resolved.prompt, "Resume the migration");
    }

    #[test]
    fn resolve_falls_back_on_out_of_range_or_blank_fields() {
        let resolved = resolve_usage_limit_auto_continue(&[cap_config(json!({
            "delay_seconds": -5,
            "prompt": "   "
        }))])
        .unwrap();
        assert_eq!(resolved.delay_seconds, DEFAULT_CONTINUATION_DELAY_SECS);
        assert_eq!(resolved.prompt, DEFAULT_CONTINUATION_PROMPT);
    }

    #[test]
    fn validate_rejects_bad_types_and_ranges() {
        let cap = UsageLimitAutoContinueCapability;
        assert!(
            cap.validate_config(&json!({ "delay_seconds": 120 }))
                .is_ok()
        );
        assert!(
            cap.validate_config(&json!({ "delay_seconds": "x" }))
                .is_err()
        );
        assert!(
            cap.validate_config(&json!({ "delay_seconds": 999999999 }))
                .is_err()
        );
        assert!(cap.validate_config(&json!({ "prompt": 5 })).is_err());
    }
}
