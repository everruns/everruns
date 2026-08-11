//! Effective error-disclosure resolution for a turn.
//!
//! The `error_disclosure` capability (in `everruns-builtins`) advertises and
//! validates the configuration; the kernel is what applies it when a turn
//! fails, so the resolver and the capability id it keys on live here (EVE-884).

use crate::capability_types::AgentCapabilityConfig;
use crate::user_facing_error::ErrorDisclosure;

/// Stable id of the capability whose configuration sets the disclosure ceiling.
pub const ERROR_DISCLOSURE_CAPABILITY_ID: &str = "error_disclosure";

/// Disclosure mode configured on the enabled `error_disclosure` capability,
/// or `None` when the capability is not enabled for the agent.
fn configured_mode(configs: &[AgentCapabilityConfig]) -> Option<ErrorDisclosure> {
    let config = configs
        .iter()
        .find(|config| config.capability_id() == ERROR_DISCLOSURE_CAPABILITY_ID)?;
    Some(
        config
            .config_value()
            .clone()
            .get("mode")
            .and_then(|mode| mode.as_str())
            .and_then(ErrorDisclosure::parse)
            .unwrap_or_default(),
    )
}

/// Resolve the effective error-disclosure mode for a turn.
///
/// Precedence: per-message `controls.error_disclosure` override, clamped to
/// the capability-configured ceiling (capability absent => `standard`).
///
/// THREAT[TM-LLM-024]: the clamp is the security boundary — message controls
/// are client-supplied, so they may only narrow disclosure, never widen it
/// beyond what the agent operator configured.
pub fn resolve_error_disclosure(
    configs: &[AgentCapabilityConfig],
    requested: Option<&str>,
) -> ErrorDisclosure {
    let ceiling = configured_mode(configs).unwrap_or_default();
    match requested.and_then(ErrorDisclosure::parse) {
        Some(requested) => requested.min(ceiling),
        None => ceiling,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn cap_config(mode: &str) -> AgentCapabilityConfig {
        AgentCapabilityConfig::with_config(
            ERROR_DISCLOSURE_CAPABILITY_ID,
            serde_json::json!({ "mode": mode }),
        )
    }

    #[test]
    fn resolve_defaults_to_standard_without_capability() {
        assert_eq!(
            resolve_error_disclosure(&[], None),
            ErrorDisclosure::Standard
        );
    }

    #[test]
    fn resolve_uses_capability_mode() {
        assert_eq!(
            resolve_error_disclosure(&[cap_config("detailed")], None),
            ErrorDisclosure::Detailed
        );
    }

    #[test]
    fn message_control_can_only_narrow_disclosure() {
        assert_eq!(
            resolve_error_disclosure(&[cap_config("generic")], Some("detailed")),
            ErrorDisclosure::Generic
        );
        assert_eq!(
            resolve_error_disclosure(&[cap_config("detailed")], Some("generic")),
            ErrorDisclosure::Generic
        );
    }
}
