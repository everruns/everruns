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
// See specs/llm-drivers.md ("OpenRouter Server Tools") and
// https://openrouter.ai/docs/guides/features/server-tools.

use super::{Capability, CapabilityLocalization, CapabilityStatus, SystemPromptContext};
use crate::driver_registry::{OpenRouterServerTool, OpenRouterServerToolKind};
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
/// Unknown tool names are skipped (forward-compatible with OpenRouter adding or
/// renaming tools); `validate_config` rejects them on the write path so stored
/// configs stay clean. Tool order follows the config, de-duplicated by kind.
pub fn server_tools_from_config(config: &Value) -> Vec<OpenRouterServerTool> {
    let Some(names) = config.get(TOOLS_KEY).and_then(Value::as_array) else {
        return Vec::new();
    };
    let max_results = config
        .get(WEB_SEARCH_MAX_RESULTS_KEY)
        .and_then(Value::as_u64);

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
    fn tool_enum() -> Vec<Value> {
        OpenRouterServerToolKind::ALL
            .iter()
            .map(|kind| json!(kind.name()))
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
        vec![CapabilityLocalization::text(
            "uk",
            "Серверні інструменти OpenRouter",
            "Вмикає серверні інструменти OpenRouter (веб-пошук, веб-завантаження, дата й час, генерація зображень тощо), які виконуються на боці OpenRouter. Інші провайдери ігнорують це налаштування.",
        )]
    }

    fn status(&self) -> CapabilityStatus {
        CapabilityStatus::Available
    }

    fn category(&self) -> Option<&str> {
        Some("Tools")
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
                        "enum": Self::tool_enum(),
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

    fn validate_config(&self, config: &Value) -> Result<(), String> {
        if config.is_null() {
            return Ok(());
        }
        let obj = config
            .as_object()
            .ok_or_else(|| "config must be an object".to_string())?;

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
    use super::*;

    #[test]
    fn metadata_and_no_tools() {
        let cap = OpenRouterServerToolsCapability;
        assert_eq!(cap.id(), OPENROUTER_SERVER_TOOLS_CAPABILITY_ID);
        // Provider-executed: contributes no executable tools or prompt text.
        assert!(cap.tools().is_empty());
        assert!(cap.config_schema().is_some());
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
        assert!(cap.validate_config(&Value::Null).is_ok());
    }
}
