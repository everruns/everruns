// OpenRouter Server Tools Capability
//
// When added to an agent, enables OpenRouter's provider-executed "server tools"
// (beta). Unlike normal function tools, these are run by OpenRouter server-side
// — it loops internally and returns the final answer, so the agent loop never
// dispatches them. This capability therefore contributes *request intent*, not
// executable tools: the selected tools are compiled into
// `LlmCallConfig.openrouter_routing.server_tools` and serialized by the
// OpenRouter driver into the request `tools` array as `{"type":"openrouter:…"}`.
//
// Non-OpenRouter providers ignore the routing config entirely, so enabling this
// capability on a non-OpenRouter agent is a harmless no-op.
//
// See knowledge/foundations/llm-drivers.md ("OpenRouter Server Tools") and
// https://openrouter.ai/docs/guides/features/server-tools.

use everruns_core::capabilities::{Capability, CapabilityLocalization, CapabilityStatus, SystemPromptContext};
use everruns_core::capabilities::RiskLevel;
use everruns_core::driver_registry::{OpenRouterServerTool, OpenRouterServerToolKind};
use async_trait::async_trait;
use serde_json::{Value, json};

/// Capability ID for OpenRouter server tools.
pub const OPENROUTER_SERVER_TOOLS_CAPABILITY_ID: &str = "openrouter_server_tools";

/// Config key holding the list of enabled server-tool names.
const TOOLS_KEY: &str = "tools";
/// Config key holding the optional web-search `max_results` override.
const WEB_SEARCH_MAX_RESULTS_KEY: &str = "web_search_max_results";

/// OpenRouter server tools capability.
///
/// Drivers translate the collected `server_tools` into provider-executed tool
/// entries when the resolved provider is OpenRouter; other providers ignore it.
pub struct OpenRouterServerToolsCapability;

/// Compile the per-agent config into the list of activated server tools.
///
/// This is the read path; it is defensive about malformed input. The write
/// path's `validate_config` already rejects unknown tool names and a
/// non-positive `web_search_max_results`, so a normally-validated config never
/// hits these guards — they only matter for legacy or hand-edited/corrupt
/// configs. Unknown tool names are dropped, `max_results` is forwarded only when
/// `>= 1`, and tools are de-duplicated by kind in config order.
pub fn server_tools_from_config(config: &Value) -> Vec<OpenRouterServerTool> {
    let Some(names) = config.get(TOOLS_KEY).and_then(Value::as_array) else {
        return Vec::new();
    };
    // Enforce the schema's `>= 1` constraint here too: a stale `0` must not be
    // forwarded as an invalid OpenRouter `max_results`.
    let max_results = config
        .get(WEB_SEARCH_MAX_RESULTS_KEY)
        .and_then(Value::as_u64)
        .filter(|n| *n >= 1);

    let mut seen: Vec<OpenRouterServerToolKind> = Vec::new();
    let mut tools: Vec<OpenRouterServerTool> = Vec::new();
    for name in names.iter().filter_map(Value::as_str) {
        let Some(kind) = OpenRouterServerToolKind::from_name(name) else {
            continue;
        };
        if seen.contains(&kind) {
            continue;
        }
        seen.push(kind);

        // web_search is the only server tool that takes parameters today.
        let parameters = match (kind, max_results) {
            (OpenRouterServerToolKind::WebSearch, Some(max)) => Some(json!({ "max_results": max })),
            _ => None,
        };
        tools.push(OpenRouterServerTool { kind, parameters });
    }
    tools
}

impl OpenRouterServerToolsCapability {
    /// `oneOf` entries (`{const, title}`) for the server-tool enum, so the UI
    /// renders a readable label per tool and `enum_labels` overlays can localize
    /// each one. Title source of truth is `OpenRouterServerToolKind::display_name`.
    fn tool_one_of() -> Vec<Value> {
        OpenRouterServerToolKind::ALL
            .iter()
            .map(|kind| json!({ "const": kind.name(), "title": kind.display_name() }))
            .collect()
    }
}

#[async_trait]
impl Capability for OpenRouterServerToolsCapability {
    fn id(&self) -> &str {
        OPENROUTER_SERVER_TOOLS_CAPABILITY_ID
    }

    fn name(&self) -> &str {
        "OpenRouter Server Tools"
    }

    fn description(&self) -> &str {
        "Enables OpenRouter's provider-executed server tools (web search, web \
         fetch, datetime, image generation, and more). OpenRouter runs these \
         server-side; non-OpenRouter providers ignore the setting."
    }

    fn localizations(&self) -> Vec<CapabilityLocalization> {
        vec![
            CapabilityLocalization {
                locale: "en",
                name: None,
                description: None,
                config_description: Some(
                    "Choose which OpenRouter server tools the model may invoke.",
                ),
                config_overlay: None,
            },
            CapabilityLocalization {
                locale: "uk",
                name: Some("Серверні інструменти OpenRouter"),
                description: Some(
                    "Вмикає серверні інструменти OpenRouter (веб-пошук, веб-завантаження, дата й час, генерація зображень тощо), які виконуються на боці OpenRouter. Інші провайдери ігнорують це налаштування.",
                ),
                config_description: Some(
                    "Визначає, які серверні інструменти OpenRouter може викликати модель.",
                ),
                config_overlay: Some(json!({
                    "properties": {
                        TOOLS_KEY: {
                            "title": "Увімкнені серверні інструменти",
                            "description": "Серверні інструменти OpenRouter, які може викликати модель.",
                            "items": {
                                "title": "Серверний інструмент",
                                "enum_labels": {
                                    "web_search": "Веб-пошук",
                                    "web_fetch": "Веб-завантаження",
                                    "datetime": "Дата й час",
                                    "image_generation": "Генерація зображень",
                                    "apply_patch": "Застосування патчів",
                                    "fusion": "Fusion",
                                    "advisor": "Порадник",
                                    "subagent": "Субагент",
                                },
                            },
                        },
                        WEB_SEARCH_MAX_RESULTS_KEY: {
                            "title": "Максимум результатів веб-пошуку",
                            "description": "Максимальна кількість результатів для інструмента веб-пошуку.",
                        },
                    },
                })),
            },
        ]
    }

    fn status(&self) -> CapabilityStatus {
        CapabilityStatus::Available
    }

    fn category(&self) -> Option<&str> {
        Some("Tools")
    }

    /// THREAT[TM-AGENT-026]: enabling server tools grants the model
    /// provider-executed web reach (`web_search`/`web_fetch`). OpenRouter runs
    /// these, so Everruns' egress controls (TM-AGENT-018) do not apply — same
    /// exfil class as web_fetch (TM-AGENT-013). High so assignment uses the
    /// same admin-only trust gate as other outbound web egress capabilities.
    fn risk_level(&self) -> RiskLevel {
        RiskLevel::High
    }

    async fn system_prompt_contribution(&self, _ctx: &SystemPromptContext) -> Option<String> {
        None
    }

    fn config_schema(&self) -> Option<Value> {
        Some(json!({
            "type": "object",
            "properties": {
                TOOLS_KEY: {
                    "type": "array",
                    "title": "Enabled server tools",
                    "description": "OpenRouter server tools the model may invoke.",
                    "items": {
                        "type": "string",
                        "title": "Server tool",
                        "oneOf": Self::tool_one_of(),
                    },
                    "uniqueItems": true,
                },
                WEB_SEARCH_MAX_RESULTS_KEY: {
                    "type": "integer",
                    "title": "Web search max results",
                    "description": "Maximum results for the web_search server tool.",
                    "minimum": 1,
                },
            },
            "additionalProperties": false,
        }))
    }

    fn config_ui_schema(&self) -> Option<Value> {
        Some(json!({
            // Render the tool list as a multi-select of checkboxes, and show it
            // before the web-search-only numeric option.
            "ui:order": [TOOLS_KEY, WEB_SEARCH_MAX_RESULTS_KEY],
            TOOLS_KEY: {
                "ui:widget": "checkboxes",
            },
        }))
    }

    fn validate_config(&self, config: &Value) -> Result<(), String> {
        if config.is_null() {
            return Ok(());
        }
        let obj = config
            .as_object()
            .ok_or_else(|| "config must be an object".to_string())?;

        // Honor the schema's `additionalProperties: false` — server write paths
        // gate on this method, not on JSON-schema validation.
        for key in obj.keys() {
            if key != TOOLS_KEY && key != WEB_SEARCH_MAX_RESULTS_KEY {
                return Err(format!("unknown config key: {key}"));
            }
        }

        if let Some(tools) = obj.get(TOOLS_KEY) {
            let arr = tools
                .as_array()
                .ok_or_else(|| format!("`{TOOLS_KEY}` must be an array of tool names"))?;
            for entry in arr {
                let name = entry
                    .as_str()
                    .ok_or_else(|| format!("`{TOOLS_KEY}` entries must be strings"))?;
                if OpenRouterServerToolKind::from_name(name).is_none() {
                    return Err(format!("unknown OpenRouter server tool: {name}"));
                }
            }
        }

        if let Some(max) = obj.get(WEB_SEARCH_MAX_RESULTS_KEY)
            && !matches!(max.as_u64(), Some(n) if n >= 1)
        {
            return Err(format!(
                "`{WEB_SEARCH_MAX_RESULTS_KEY}` must be a positive integer"
            ));
        }

        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use everruns_core::capabilities::*;

    // Metadata/tool-list constants covered by builtin_capabilities_satisfy_registry_invariants.

    #[test]
    fn schema_lists_every_tool_with_a_title() {
        let schema = OpenRouterServerToolsCapability.config_schema().unwrap();
        let one_of = schema["properties"][TOOLS_KEY]["items"]["oneOf"]
            .as_array()
            .expect("tools render as a oneOf of labeled consts");
        assert_eq!(one_of.len(), OpenRouterServerToolKind::ALL.len());
        for entry in one_of {
            assert!(entry["const"].is_string());
            assert!(
                entry["title"].as_str().is_some_and(|t| !t.is_empty()),
                "every server tool needs a UI title: {entry}"
            );
        }
    }

    #[test]
    fn localization_resolves_ukrainian() {
        let cap = OpenRouterServerToolsCapability;
        assert_eq!(
            cap.localized_name(Some("uk")),
            "Серверні інструменти OpenRouter"
        );
        // config_description has an `en` base and a `uk` override.
        assert!(cap.describe_schema(Some("en")).is_some());
        assert!(cap.describe_schema(Some("uk")).is_some());
        assert_ne!(
            cap.describe_schema(Some("en")),
            cap.describe_schema(Some("uk"))
        );
    }

    #[test]
    fn ukrainian_overlay_labels_every_tool() {
        // Drift guard: adding a server tool must also add its localized label.
        let loc = OpenRouterServerToolsCapability
            .localizations()
            .into_iter()
            .find(|l| l.locale == "uk")
            .expect("uk localization present");
        let overlay = loc.config_overlay.expect("uk overlay present");
        let labels = &overlay["properties"][TOOLS_KEY]["items"]["enum_labels"];
        for kind in OpenRouterServerToolKind::ALL {
            assert!(
                labels[kind.name()].as_str().is_some_and(|s| !s.is_empty()),
                "missing uk enum_label for {}",
                kind.name()
            );
        }
    }

    #[test]
    fn empty_config_yields_no_server_tools() {
        assert!(server_tools_from_config(&json!({})).is_empty());
        assert!(server_tools_from_config(&Value::Null).is_empty());
        assert!(server_tools_from_config(&json!({ "tools": [] })).is_empty());
    }

    #[test]
    fn maps_known_tools_and_skips_unknown() {
        let tools = server_tools_from_config(&json!({
            "tools": ["web_fetch", "datetime", "not_a_real_tool"],
        }));
        let kinds: Vec<_> = tools.iter().map(|t| t.kind).collect();
        assert_eq!(
            kinds,
            vec![
                OpenRouterServerToolKind::WebFetch,
                OpenRouterServerToolKind::Datetime
            ]
        );
        assert!(tools.iter().all(|t| t.parameters.is_none()));
    }

    #[test]
    fn web_search_attaches_max_results() {
        let tools = server_tools_from_config(&json!({
            "tools": ["web_search", "datetime"],
            "web_search_max_results": 5,
        }));
        let web = tools
            .iter()
            .find(|t| t.kind == OpenRouterServerToolKind::WebSearch)
            .expect("web_search present");
        assert_eq!(web.parameters, Some(json!({ "max_results": 5 })));
        // max_results only decorates web_search.
        let datetime = tools
            .iter()
            .find(|t| t.kind == OpenRouterServerToolKind::Datetime)
            .expect("datetime present");
        assert!(datetime.parameters.is_none());
    }

    #[test]
    fn stale_zero_max_results_is_not_forwarded() {
        // A legacy/corrupt `0` must not become an invalid OpenRouter request.
        let tools = server_tools_from_config(&json!({
            "tools": ["web_search"],
            "web_search_max_results": 0,
        }));
        let web = tools
            .iter()
            .find(|t| t.kind == OpenRouterServerToolKind::WebSearch)
            .expect("web_search present");
        assert!(web.parameters.is_none());
    }

    #[test]
    fn duplicate_tool_names_are_deduped() {
        let tools = server_tools_from_config(&json!({
            "tools": ["datetime", "datetime"],
        }));
        assert_eq!(tools.len(), 1);
    }

    #[test]
    fn validate_rejects_unknown_tool_and_bad_max_results() {
        let cap = OpenRouterServerToolsCapability;
        assert!(
            cap.validate_config(&json!({ "tools": ["web_search"] }))
                .is_ok()
        );
        assert!(cap.validate_config(&json!({ "tools": ["bogus"] })).is_err());
        assert!(
            cap.validate_config(&json!({ "tools": ["web_search"], "web_search_max_results": 0 }))
                .is_err()
        );
        // additionalProperties: false — unknown keys are rejected.
        assert!(
            cap.validate_config(&json!({ "tools": ["web_search"], "extra": true }))
                .is_err()
        );
        assert!(
            cap.validate_config(&json!({ "web_search_max_results": 3 }))
                .is_ok()
        );
        assert!(cap.validate_config(&Value::Null).is_ok());
    }
}
