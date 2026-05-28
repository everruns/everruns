// User Hooks capability — user-config-authored hook entries + capability-bundle
// muting. See `specs/user-hooks.md` for the design.
//
// This capability has no tools and no system prompt. Its single job is to
// surface `UserHookSpec` entries from its config (the user's own hook list)
// to the central `HookAdapterBuilder`, and to carry the
// `disabled_contributions` list that mutes capability-bundled hooks.
//
// Risk: High. Even when the user supplies no entries of their own, accepting
// arbitrary commands from config is a code-execution surface; enabling the
// capability is therefore an admin-only action through the existing
// `check_high_risk_caps` gate.

use serde::{Deserialize, Serialize};

use super::{Capability, CapabilityStatus, RiskLevel};
use crate::user_hook_types::{HookSource, UserHookSpec};

/// Config shape parsed from `AgentCapabilityConfig.config` for the
/// `user_hooks` capability. Both fields default to empty so an
/// unconfigured-but-enabled instance is a legal no-op.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(default)]
pub struct UserHooksConfig {
    pub hooks: Vec<UserHookSpec>,
    pub disabled_contributions: Vec<String>,
}

pub struct UserHooksCapability;

impl Capability for UserHooksCapability {
    fn id(&self) -> &str {
        "user_hooks"
    }

    fn name(&self) -> &str {
        "User Hooks"
    }

    fn description(&self) -> &str {
        "Run user-authored shell commands at well-defined points in the agent \
         execution lifecycle. Hooks can mutate inputs or block execution. See \
         specs/user-hooks.md for the contract."
    }

    fn status(&self) -> CapabilityStatus {
        CapabilityStatus::Available
    }

    fn category(&self) -> Option<&str> {
        Some("Automation")
    }

    fn icon(&self) -> Option<&str> {
        Some("plug")
    }

    fn risk_level(&self) -> RiskLevel {
        // Accepts arbitrary bash commands from config. Even though execution
        // is sandboxed through virtual_bash, the assignment gate must stay
        // admin-only — anyone who can configure this capability can run
        // arbitrary code in the session sandbox on every agent action.
        RiskLevel::High
    }

    fn config_schema(&self) -> Option<serde_json::Value> {
        Some(serde_json::json!({
            "type": "object",
            "properties": {
                "hooks": {
                    "type": "array",
                    "description": "User-authored hook entries (see specs/user-hooks.md#userhookspec).",
                    "items": { "$ref": "#/$defs/UserHookSpec" }
                },
                "disabled_contributions": {
                    "type": "array",
                    "description": "Stable HookId strings of capability-contributed hooks to mute. Format: \"{capability_id}:{name}\".",
                    "items": { "type": "string" }
                }
            },
            "additionalProperties": false,
            "$defs": {
                "UserHookSpec": {
                    "type": "object",
                    "required": ["event", "executor"],
                    "properties": {
                        "id": { "type": "string" },
                        "event": {
                            "type": "string",
                            "enum": [
                                "session_start",
                                "user_prompt_submit",
                                "pre_tool_use",
                                "post_tool_use",
                                "turn_end",
                                "session_end"
                            ]
                        },
                        "matcher": {
                            "type": "object",
                            "properties": {
                                "tool_name": { "type": "string" },
                                "tool_name_glob": { "type": "string" },
                                "args_jsonpath": { "type": "string" },
                                "match_regex": { "type": "string" },
                                "deny_regex": { "type": "string" }
                            },
                            "additionalProperties": false
                        },
                        "executor": {
                            "type": "object",
                            "required": ["type", "command"],
                            "properties": {
                                "type": { "const": "bash" },
                                "command": { "type": "string" },
                                "env": {
                                    "type": "object",
                                    "additionalProperties": { "type": "string" }
                                }
                            }
                        },
                        "timeout_ms": { "type": "integer", "minimum": 100, "maximum": 30000 },
                        "on_error": { "type": "string", "enum": ["block", "allow", "warn"] },
                        "description": { "type": "string" }
                    }
                }
            }
        }))
    }

    fn validate_config(&self, config: &serde_json::Value) -> Result<(), String> {
        // Empty/null is valid: the capability becomes a pass-through that
        // only applies `disabled_contributions` (which is also empty).
        if config.is_null() {
            return Ok(());
        }
        let parsed: UserHooksConfig = serde_json::from_value(config.clone())
            .map_err(|e| format!("user_hooks config parse failed: {e}"))?;

        for (idx, hook) in parsed.hooks.iter().enumerate() {
            hook.validate()
                .map_err(|e| format!("user_hooks.hooks[{idx}]: {e}"))?;
        }
        Ok(())
    }

    fn user_hooks_with_config(&self, config: &serde_json::Value) -> Vec<UserHookSpec> {
        let parsed: UserHooksConfig = if config.is_null() {
            return vec![];
        } else {
            match serde_json::from_value(config.clone()) {
                Ok(c) => c,
                Err(_) => return vec![],
            }
        };
        // Stamp source = UserConfig on each entry so resolve_id picks the
        // `user:` namespace.
        parsed
            .hooks
            .into_iter()
            .map(|mut hook| {
                hook.source = HookSource::UserConfig;
                hook
            })
            .collect()
    }
}

/// Read the `disabled_contributions` list out of the `user_hooks` capability's
/// config. Exposed as a free function so the collection pipeline can apply it
/// without round-tripping through the capability trait.
pub fn disabled_contributions(config: &serde_json::Value) -> Vec<String> {
    if config.is_null() {
        return vec![];
    }
    serde_json::from_value::<UserHooksConfig>(config.clone())
        .map(|c| c.disabled_contributions)
        .unwrap_or_default()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::user_hook_types::HookEvent;

    #[test]
    fn capability_metadata_is_stable() {
        let cap = UserHooksCapability;
        assert_eq!(cap.id(), "user_hooks");
        assert!(matches!(cap.risk_level(), RiskLevel::High));
        assert!(cap.config_schema().is_some());
    }

    #[test]
    fn empty_config_validates() {
        let cap = UserHooksCapability;
        assert!(cap.validate_config(&serde_json::Value::Null).is_ok());
        assert!(cap.validate_config(&serde_json::json!({})).is_ok());
    }

    #[test]
    fn full_config_validates_and_yields_specs() {
        let cap = UserHooksCapability;
        let config = serde_json::json!({
            "hooks": [
                {
                    "id": "fmt",
                    "event": "post_tool_use",
                    "matcher": { "tool_name": "edit_file" },
                    "executor": { "type": "bash", "command": "cargo fmt" },
                    "timeout_ms": 5000,
                    "on_error": "warn",
                    "description": "format after edits"
                }
            ],
            "disabled_contributions": ["other_pack:noisy_hook"]
        });
        cap.validate_config(&config).expect("validate");
        let specs = cap.user_hooks_with_config(&config);
        assert_eq!(specs.len(), 1);
        assert_eq!(specs[0].event, HookEvent::PostToolUse);
        assert!(matches!(specs[0].source, HookSource::UserConfig));
        assert_eq!(specs[0].resolve_id(0).as_str(), "user:fmt");
        let muted = disabled_contributions(&config);
        assert_eq!(muted, vec!["other_pack:noisy_hook"]);
    }

    #[test]
    fn invalid_hook_in_config_is_rejected() {
        let cap = UserHooksCapability;
        let config = serde_json::json!({
            "hooks": [
                {
                    "event": "pre_tool_use",
                    "executor": { "type": "bash", "command": "true" },
                    "timeout_ms": 50
                }
            ]
        });
        let err = cap.validate_config(&config).unwrap_err();
        assert!(err.contains("timeout_ms"), "{}", err);
    }

    #[test]
    fn matcher_on_non_tool_event_is_rejected_via_validate() {
        let cap = UserHooksCapability;
        let config = serde_json::json!({
            "hooks": [
                {
                    "event": "session_start",
                    "matcher": { "tool_name": "bash" },
                    "executor": { "type": "bash", "command": "true" }
                }
            ]
        });
        assert!(cap.validate_config(&config).is_err());
    }

    #[test]
    fn missing_config_yields_no_specs() {
        let cap = UserHooksCapability;
        let empty = cap.user_hooks_with_config(&serde_json::Value::Null);
        assert!(empty.is_empty());
        let none = cap.user_hooks_with_config(&serde_json::json!({}));
        assert!(none.is_empty());
    }
}
