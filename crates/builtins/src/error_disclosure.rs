// Error disclosure capability
//
// Controls how much detail about run-blocking errors (provider failures,
// quota exhaustion, misconfiguration, …) is shown to session viewers:
//
// - `generic`: every blocking error collapses into one generic, localizable
//   message. For public-facing agents where provider/billing state must not
//   leak.
// - `standard`: stable error code + structured fields (platform default,
//   also used when this capability is not enabled).
// - `detailed`: standard plus the underlying driver error text. For trusted
//   surfaces such as coding-agent harnesses built on the runtime.
//
// Per-message override: `controls.error_disclosure` on the session input
// message can request a mode, but it is clamped to at most the mode this
// capability allows (capability absent => `standard` ceiling), so a client
// can never widen disclosure beyond what the agent operator configured.
//
// The applied mode and pre-disclosure error code are recorded in message
// metadata (`error_disclosure`, `source_error_code`) for tracking.

use everruns_core::capabilities::{Capability, CapabilityLocalization};
use everruns_core::capability_types::AgentCapabilityConfig;
use everruns_core::user_facing_error::ErrorDisclosure;

pub const ERROR_DISCLOSURE_CAPABILITY_ID: &str = "error_disclosure";

pub struct ErrorDisclosureCapability;

impl Capability for ErrorDisclosureCapability {
    fn id(&self) -> &str {
        ERROR_DISCLOSURE_CAPABILITY_ID
    }

    fn name(&self) -> &str {
        "Error Disclosure"
    }

    fn description(&self) -> &str {
        "Controls how much detail about run-blocking errors is shown in the session: \
         a single generic message (public agents), stable error codes (default), \
         or full provider error details (trusted surfaces)."
    }

    fn config_schema(&self) -> Option<serde_json::Value> {
        Some(serde_json::json!({
            "type": "object",
            "properties": {
                "mode": {
                    "type": "string",
                    "title": "Disclosure mode",
                    "description": "generic: one generic localized message; standard: stable error code + fields; detailed: standard plus the underlying provider error text.",
                    "enum": ["generic", "standard", "detailed"],
                    "default": "standard"
                }
            }
        }))
    }

    fn validate_config(&self, config: &serde_json::Value) -> Result<(), String> {
        if config.is_null() {
            return Ok(());
        }
        if !config.is_object() {
            return Err("error_disclosure config must be an object".to_string());
        }
        match config.get("mode") {
            None => Ok(()),
            Some(serde_json::Value::String(mode)) if ErrorDisclosure::parse(mode).is_some() => {
                Ok(())
            }
            Some(value) => Err(format!(
                "mode must be one of \"generic\", \"standard\", \"detailed\", got {value}"
            )),
        }
    }

    fn localizations(&self) -> Vec<CapabilityLocalization> {
        vec![
            CapabilityLocalization {
                locale: "en",
                name: None,
                description: None,
                config_description: Some(
                    "Chooses how much error detail session viewers see when a turn fails.",
                ),
                config_overlay: None,
            },
            CapabilityLocalization {
                locale: "uk",
                name: Some("Розкриття помилок"),
                description: Some(
                    "Визначає, скільки деталей про блокуючі помилки показувати в сесії: \
                     одне загальне повідомлення (публічні агенти), стабільні коди помилок \
                     (за замовчуванням) або повні деталі помилки провайдера (довірені середовища).",
                ),
                config_description: Some(
                    "Визначає, скільки деталей про помилку бачать користувачі сесії, коли хід завершується невдало.",
                ),
                config_overlay: Some(serde_json::json!({
                    "properties": {
                        "mode": {
                            "title": "Режим розкриття",
                            "description": "generic: одне загальне локалізоване повідомлення; standard: стабільний код помилки з полями; detailed: standard плюс текст помилки провайдера."
                        }
                    }
                })),
            },
        ]
    }
}

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
    use everruns_core::capabilities::*;

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
        assert_eq!(
            resolve_error_disclosure(&[cap_config("generic")], None),
            ErrorDisclosure::Generic
        );
    }

    #[test]
    fn resolve_capability_without_mode_defaults_to_standard() {
        let config = AgentCapabilityConfig::new(ERROR_DISCLOSURE_CAPABILITY_ID);
        assert_eq!(
            resolve_error_disclosure(&[config], None),
            ErrorDisclosure::Standard
        );
    }

    #[test]
    fn controls_can_narrow_but_not_widen() {
        // Detailed ceiling: controls may narrow to generic.
        assert_eq!(
            resolve_error_disclosure(&[cap_config("detailed")], Some("generic")),
            ErrorDisclosure::Generic
        );
        // Generic ceiling: controls cannot widen to detailed.
        assert_eq!(
            resolve_error_disclosure(&[cap_config("generic")], Some("detailed")),
            ErrorDisclosure::Generic
        );
        // No capability: standard ceiling, detailed request is clamped.
        assert_eq!(
            resolve_error_disclosure(&[], Some("detailed")),
            ErrorDisclosure::Standard
        );
        // Unknown values are ignored.
        assert_eq!(
            resolve_error_disclosure(&[cap_config("detailed")], Some("everything")),
            ErrorDisclosure::Detailed
        );
    }

    #[test]
    fn validate_config_accepts_known_modes_only() {
        let cap = ErrorDisclosureCapability;
        assert!(cap.validate_config(&serde_json::Value::Null).is_ok());
        assert!(cap.validate_config(&serde_json::json!({})).is_ok());
        assert!(
            cap.validate_config(&serde_json::json!({"mode": "generic"}))
                .is_ok()
        );
        assert!(
            cap.validate_config(&serde_json::json!({"mode": "loud"}))
                .is_err()
        );
        assert!(cap.validate_config(&serde_json::json!([])).is_err());
    }

    #[test]
    fn localizations_resolve_uk() {
        let cap = ErrorDisclosureCapability;
        assert_eq!(cap.localized_name(Some("uk-UA")), "Розкриття помилок");
    }
}
