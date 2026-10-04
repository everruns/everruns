// User Hooks capability — user-config-authored hook entries + capability-bundle
// muting. See `knowledge/runtime-resources/user-hooks.md` for the design.
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

use super::{Capability, CapabilityLocalization, CapabilityStatus, RiskLevel};
use everruns_core::user_hook_types::{HookSource, UserHookSpec};

/// Config shape parsed from `AgentCapabilityConfig.config` for the
/// `user_hooks` capability. Both fields default to empty so an
/// unconfigured-but-enabled instance is a legal no-op.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(default)]
pub struct UserHooksConfig {
    pub hooks: Vec<UserHookSpec>,
    pub disabled_contributions: Vec<String>,
}

pub const USER_HOOKS_CAPABILITY_ID: &str = "user_hooks";
pub const MAX_USER_HOOKS_CONFIG_BYTES: usize = 256 * 1024;
pub const MAX_USER_HOOKS: usize = 64;

pub struct UserHooksCapability;

impl Capability for UserHooksCapability {
    fn id(&self) -> &str {
        USER_HOOKS_CAPABILITY_ID
    }

    fn name(&self) -> &str {
        "User Hooks"
    }

    fn description(&self) -> &str {
        "Run user-authored shell commands at well-defined points in the agent \
         execution lifecycle. Hooks can mutate inputs or block execution. See \
         knowledge/runtime-resources/user-hooks.md for the contract."
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
        // is sandboxed through bashkit_shell, the assignment gate must stay
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
                    "title": "Hooks",
                    "description": "User-authored hook entries (see knowledge/runtime-resources/user-hooks.md#userhookspec).",
                    "items": { "$ref": "#/$defs/UserHookSpec" }
                },
                "disabled_contributions": {
                    "type": "array",
                    "title": "Disabled contributed hooks",
                    "description": "Stable HookId strings of capability-contributed hooks to mute. Format: \"{capability_id}:{name}\".",
                    "items": {
                        "type": "string",
                        "title": "Hook ID",
                        "description": "Stable HookId of a capability-contributed hook."
                    }
                }
            },
            "additionalProperties": false,
            "$defs": {
                "UserHookSpec": {
                    "type": "object",
                    "required": ["event", "executor"],
                    "properties": {
                        "id": {
                            "type": "string",
                            "title": "Hook ID",
                            "description": "Optional stable identifier for this hook."
                        },
                        "event": {
                            "type": "string",
                            "title": "Event",
                            "description": "Lifecycle event that triggers this hook.",
                            "oneOf": [
                                { "const": "session_start", "title": "Session start" },
                                { "const": "user_prompt_submit", "title": "User prompt submit" },
                                { "const": "pre_tool_use", "title": "Before tool use" },
                                { "const": "post_tool_use", "title": "After tool use" },
                                { "const": "turn_end", "title": "Turn end" },
                                { "const": "session_end", "title": "Session end" }
                            ]
                        },
                        "matcher": {
                            "type": "object",
                            "title": "Matcher",
                            "description": "Optional filter that limits which tool calls trigger this hook (tool events only).",
                            "properties": {
                                "tool_name": {
                                    "type": "string",
                                    "title": "Tool name",
                                    "description": "Exact tool name to match."
                                },
                                "tool_name_glob": {
                                    "type": "string",
                                    "title": "Tool name glob",
                                    "description": "Glob pattern matched against the tool name."
                                },
                                "args_jsonpath": {
                                    "type": "string",
                                    "title": "Arguments JSONPath",
                                    "description": "JSONPath selecting the tool argument value to test."
                                },
                                "match_regex": {
                                    "type": "string",
                                    "title": "Match regex",
                                    "description": "Regex the selected value must match."
                                },
                                "deny_regex": {
                                    "type": "string",
                                    "title": "Deny regex",
                                    "description": "Regex the selected value must not match."
                                }
                            },
                            "additionalProperties": false
                        },
                        "executor": {
                            "type": "object",
                            "title": "Executor",
                            "description": "Command executed when the hook fires.",
                            "required": ["type", "command"],
                            "properties": {
                                "type": {
                                    "title": "Executor type",
                                    "description": "Executor kind; only bash is supported.",
                                    "const": "bash"
                                },
                                "command": {
                                    "type": "string",
                                    "title": "Command",
                                    "description": "Shell command run in the session sandbox."
                                },
                                "env": {
                                    "type": "object",
                                    "title": "Environment variables",
                                    "description": "Extra environment variables passed to the command.",
                                    "additionalProperties": { "type": "string" }
                                }
                            }
                        },
                        "timeout_ms": {
                            "type": "integer",
                            "title": "Timeout (ms)",
                            "description": "Maximum hook run time in milliseconds (100-30000).",
                            "minimum": 100,
                            "maximum": 30000
                        },
                        "on_error": {
                            "type": "string",
                            "title": "On error",
                            "description": "What to do when the hook command fails.",
                            "oneOf": [
                                { "const": "block", "title": "Block" },
                                { "const": "allow", "title": "Allow" },
                                { "const": "warn", "title": "Warn" }
                            ]
                        },
                        "description": {
                            "type": "string",
                            "title": "Description",
                            "description": "Optional human-readable note about what this hook does."
                        }
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
        let config_bytes = serde_json::to_vec(config)
            .map_err(|e| format!("user_hooks config serialization failed: {e}"))?
            .len();
        if config_bytes > MAX_USER_HOOKS_CONFIG_BYTES {
            return Err(format!(
                "user_hooks config is {config_bytes} bytes; maximum is {MAX_USER_HOOKS_CONFIG_BYTES}"
            ));
        }
        let parsed: UserHooksConfig = serde_json::from_value(config.clone())
            .map_err(|e| format!("user_hooks config parse failed: {e}"))?;
        if parsed.hooks.len() > MAX_USER_HOOKS {
            return Err(format!(
                "user_hooks.hooks has {} entries; maximum is {MAX_USER_HOOKS}",
                parsed.hooks.len()
            ));
        }

        for (idx, hook) in parsed.hooks.iter().enumerate() {
            hook.validate()
                .map_err(|e| format!("user_hooks.hooks[{idx}]: {e}"))?;
        }
        Ok(())
    }

    fn localizations(&self) -> Vec<CapabilityLocalization> {
        vec![
            CapabilityLocalization {
                locale: "en",
                name: None,
                description: None,
                config_description: Some(
                    "Defines user-authored hook commands that run at agent lifecycle \
                     events and which capability-contributed hooks to mute.",
                ),
                config_overlay: None,
            },
            CapabilityLocalization {
                locale: "uk",
                name: Some("Хуки користувача"),
                description: Some(
                    "Виконує написані користувачем шелл-команди у визначених точках \
                     життєвого циклу виконання агента. Хуки можуть змінювати вхідні дані \
                     або блокувати виконання.",
                ),
                config_description: Some(
                    "Визначає хуки користувача, що запускаються на подіях життєвого циклу агента, та які хуки від можливостей вимкнути.",
                ),
                config_overlay: Some(serde_json::json!({
                    "properties": {
                        "hooks": {
                            "title": "Хуки",
                            "description": "Хуки, створені користувачем (див. knowledge/runtime-resources/user-hooks.md#userhookspec).",
                            "items": {
                                "properties": {
                                    "id": {
                                        "title": "Ідентифікатор хука",
                                        "description": "Необов'язковий стабільний ідентифікатор цього хука."
                                    },
                                    "event": {
                                        "title": "Подія",
                                        "description": "Подія життєвого циклу, що запускає цей хук.",
                                        "enum_labels": {
                                            "session_start": "Початок сесії",
                                            "user_prompt_submit": "Надсилання запиту користувача",
                                            "pre_tool_use": "Перед викликом інструмента",
                                            "post_tool_use": "Після виклику інструмента",
                                            "turn_end": "Завершення ходу",
                                            "session_end": "Завершення сесії"
                                        }
                                    },
                                    "matcher": {
                                        "title": "Фільтр",
                                        "description": "Необов'язковий фільтр, що обмежує, які виклики інструментів запускають цей хук (лише для подій інструментів).",
                                        "properties": {
                                            "tool_name": {
                                                "title": "Назва інструмента",
                                                "description": "Точна назва інструмента для збігу."
                                            },
                                            "tool_name_glob": {
                                                "title": "Glob назви інструмента",
                                                "description": "Glob-шаблон, що зіставляється з назвою інструмента."
                                            },
                                            "args_jsonpath": {
                                                "title": "JSONPath аргументів",
                                                "description": "JSONPath, що вибирає значення аргументу інструмента для перевірки."
                                            },
                                            "match_regex": {
                                                "title": "Регулярний вираз збігу",
                                                "description": "Регулярний вираз, якому має відповідати вибране значення."
                                            },
                                            "deny_regex": {
                                                "title": "Регулярний вираз заборони",
                                                "description": "Регулярний вираз, якому вибране значення не повинно відповідати."
                                            }
                                        }
                                    },
                                    "executor": {
                                        "title": "Виконавець",
                                        "description": "Команда, що виконується, коли спрацьовує хук.",
                                        "properties": {
                                            "type": {
                                                "title": "Тип виконавця",
                                                "description": "Вид виконавця; підтримується лише bash."
                                            },
                                            "command": {
                                                "title": "Команда",
                                                "description": "Шелл-команда, що виконується в пісочниці сесії."
                                            },
                                            "env": {
                                                "title": "Змінні середовища",
                                                "description": "Додаткові змінні середовища, що передаються команді."
                                            }
                                        }
                                    },
                                    "timeout_ms": {
                                        "title": "Тайм-аут (мс)",
                                        "description": "Максимальний час виконання хука в мілісекундах (100-30000)."
                                    },
                                    "on_error": {
                                        "title": "У разі помилки",
                                        "description": "Що робити, коли команда хука завершується з помилкою.",
                                        "enum_labels": {
                                            "block": "Блокувати",
                                            "allow": "Дозволити",
                                            "warn": "Попередити"
                                        }
                                    },
                                    "description": {
                                        "title": "Опис",
                                        "description": "Необов'язкова зрозуміла людині нотатка про призначення цього хука."
                                    }
                                }
                            }
                        },
                        "disabled_contributions": {
                            "title": "Вимкнені хуки можливостей",
                            "description": "Стабільні ідентифікатори HookId хуків, доданих можливостями, які потрібно вимкнути. Формат: \"{capability_id}:{name}\".",
                            "items": {
                                "title": "Ідентифікатор хука",
                                "description": "Стабільний HookId хука, доданого можливістю."
                            }
                        }
                    }
                })),
            },
        ]
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
    use everruns_core::user_hook_types::HookEvent;

    // Metadata constants covered by builtin_capabilities_satisfy_registry_invariants.

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
    fn too_many_user_hooks_are_rejected_before_per_hook_validation() {
        let cap = UserHooksCapability;
        let hooks = (0..=MAX_USER_HOOKS)
            .map(|_| {
                serde_json::json!({
                    "event": "pre_tool_use",
                    "matcher": { "match_regex": "[" },
                    "executor": { "type": "bash", "command": "true" }
                })
            })
            .collect::<Vec<_>>();
        let config = serde_json::json!({ "hooks": hooks });

        let err = cap.validate_config(&config).unwrap_err();
        assert!(
            err.contains("maximum is") && !err.contains("regex"),
            "{}",
            err
        );
    }

    #[test]
    fn oversized_user_hooks_config_is_rejected() {
        let cap = UserHooksCapability;
        let config = serde_json::json!({
            "hooks": [
                {
                    "event": "pre_tool_use",
                    "description": "x".repeat(MAX_USER_HOOKS_CONFIG_BYTES),
                    "executor": { "type": "bash", "command": "true" }
                }
            ]
        });

        let err = cap.validate_config(&config).unwrap_err();
        assert!(err.contains("maximum is"), "{}", err);
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
    fn uk_localization_and_schema_one_of_match_validation() {
        let cap = UserHooksCapability;
        assert_eq!(cap.localized_name(Some("uk-UA")), "Хуки користувача");
        assert!(
            cap.localized_description(Some("uk-UA"))
                .contains("шелл-команди")
        );
        assert!(cap.describe_schema(Some("uk")).is_some());
        assert!(cap.describe_schema(None).is_some());

        // Every event/on_error oneOf const must be accepted by validate_config.
        let schema = cap.config_schema().expect("config schema");
        let spec = &schema["$defs"]["UserHookSpec"]["properties"];
        let consts = |prop: &serde_json::Value| -> Vec<String> {
            prop["oneOf"]
                .as_array()
                .expect("oneOf")
                .iter()
                .map(|v| v["const"].as_str().expect("const").to_string())
                .collect()
        };
        let events = consts(&spec["event"]);
        let on_errors = consts(&spec["on_error"]);
        assert_eq!(events.len(), 6);
        assert_eq!(on_errors, vec!["block", "allow", "warn"]);
        for event in &events {
            for on_error in &on_errors {
                let config = serde_json::json!({
                    "hooks": [
                        {
                            "event": event,
                            "executor": { "type": "bash", "command": "true" },
                            "on_error": on_error
                        }
                    ]
                });
                cap.validate_config(&config)
                    .unwrap_or_else(|e| panic!("{event}/{on_error} should validate: {e}"));
            }
        }
    }

    #[test]
    fn missing_config_yields_no_specs() {
        let cap = UserHooksCapability;
        let empty = cap.user_hooks_with_config(&serde_json::Value::Null);
        assert!(empty.is_empty());
        let none = cap.user_hooks_with_config(&serde_json::json!({}));
        assert!(none.is_empty());
    }

    #[test]
    fn all_example_bundles_validate_against_user_hooks_schema() {
        let dir = format!("{}/../../examples/hook-bundles", env!("CARGO_MANIFEST_DIR"));
        let cap = UserHooksCapability;
        let mut checked = 0;
        for entry in std::fs::read_dir(&dir).unwrap() {
            let path = entry.unwrap().path();
            if path.extension().and_then(|extension| extension.to_str()) != Some("json") {
                continue;
            }
            let raw = std::fs::read_to_string(&path).unwrap();
            let config: serde_json::Value = serde_json::from_str(&raw)
                .unwrap_or_else(|error| panic!("{}: invalid JSON: {error}", path.display()));
            cap.validate_config(&config).unwrap_or_else(|error| {
                panic!("{}: failed user_hooks validation: {error}", path.display())
            });
            checked += 1;
        }
        assert!(checked >= 6, "expected shipped bundles, saw {checked}");
    }
}
