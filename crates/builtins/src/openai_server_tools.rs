// OpenAI Server Tools Capability (EVE-1115)
//
// Enables OpenAI's hosted, provider-executed tools on OpenAI Responses agents.
// Like `openrouter_server_tools`, it contributes request intent, not executable
// tools: the selection is stashed in `LlmCallConfig.driver_options` under
// `openai/hosted_tools`, and the OpenAI Responses driver appends it to the
// request `tools` array. OpenAI runs the tool inside the response, so the
// agent loop never dispatches it.
//
// Unlike the OpenRouter precedent, a non-OpenAI agent does not ignore it: the
// reason step fails the turn with a message naming the provider, because an
// agent configured to search the web must not quietly answer without it.
//
// `web_search` ships first. Code interpreter, file search and remote MCP are
// follow-ups under the same capability.

use async_trait::async_trait;
use everruns_provider::openai_hosted_tools::{
    OpenAiHostedTools, SearchContextSize, WebSearchTool, WebSearchUserLocation,
};
use serde_json::{Value, json};

use everruns_core::capabilities::{
    Capability, CapabilityLocalization, CapabilityStatus, RiskLevel, SystemPromptContext,
};

/// Capability ID for OpenAI hosted tools.
pub const OPENAI_SERVER_TOOLS_CAPABILITY_ID: &str = "openai_server_tools";

const TOOLS_KEY: &str = "tools";
const CONTEXT_SIZE_KEY: &str = "web_search_context_size";
const ALLOWED_DOMAINS_KEY: &str = "web_search_allowed_domains";
const LOCATION_KEY: &str = "web_search_user_location";
const LOCATION_FIELDS: [&str; 4] = ["country", "region", "city", "timezone"];
const WEB_SEARCH: &str = "web_search";

/// Hosted tool names this capability accepts, in UI order.
const TOOL_NAMES: [(&str, &str); 1] = [(WEB_SEARCH, "Web search")];

/// OpenAI server tools capability.
pub struct OpenAiServerToolsCapability;

/// Compile the per-agent config into the hosted tools to request.
///
/// Read path: defensive about configs `validate_config` would reject (legacy
/// or hand-edited). Unknown tool names and malformed options are dropped.
pub fn hosted_tools_from_config(config: &Value) -> OpenAiHostedTools {
    let enabled = |name: &str| {
        config
            .get(TOOLS_KEY)
            .and_then(Value::as_array)
            .is_some_and(|tools| tools.iter().any(|tool| tool.as_str() == Some(name)))
    };
    let web_search = enabled(WEB_SEARCH).then(|| WebSearchTool {
        search_context_size: config
            .get(CONTEXT_SIZE_KEY)
            .and_then(|size| serde_json::from_value::<SearchContextSize>(size.clone()).ok()),
        user_location: config.get(LOCATION_KEY).and_then(|location| {
            let field = |key: &str| {
                location
                    .get(key)
                    .and_then(Value::as_str)
                    .map(str::trim)
                    .filter(|value| !value.is_empty())
                    .map(str::to_string)
            };
            let location = WebSearchUserLocation {
                country: field("country"),
                region: field("region"),
                city: field("city"),
                timezone: field("timezone"),
            };
            (location != WebSearchUserLocation::default()).then_some(location)
        }),
        allowed_domains: config
            .get(ALLOWED_DOMAINS_KEY)
            .and_then(Value::as_array)
            .map(|domains| {
                domains
                    .iter()
                    .filter_map(Value::as_str)
                    .map(str::trim)
                    .filter(|domain| !domain.is_empty())
                    .map(str::to_string)
                    .collect()
            })
            .unwrap_or_default(),
    });
    OpenAiHostedTools { web_search }
}

#[async_trait]
impl Capability for OpenAiServerToolsCapability {
    fn id(&self) -> &str {
        OPENAI_SERVER_TOOLS_CAPABILITY_ID
    }

    fn name(&self) -> &str {
        "OpenAI Server Tools"
    }

    fn description(&self) -> &str {
        "Enables OpenAI's hosted tools, starting with web search, on agents that \
         run on the OpenAI or Azure OpenAI Responses API. OpenAI runs these tools \
         and sends conversation content to them. Agents on other providers fail \
         with an explanation instead of running without the tools."
    }

    fn localizations(&self) -> Vec<CapabilityLocalization> {
        vec![
            CapabilityLocalization {
                locale: "en",
                name: None,
                description: None,
                config_description: Some(
                    "Choose which OpenAI hosted tools the model may invoke and how web search behaves.",
                ),
                config_overlay: None,
            },
            CapabilityLocalization {
                locale: "uk",
                name: Some("Серверні інструменти OpenAI"),
                description: Some(
                    "Вмикає розміщені інструменти OpenAI, починаючи з веб-пошуку, для агентів на OpenAI або Azure OpenAI Responses API. Інструменти виконує OpenAI, і вони отримують вміст розмови. Агенти на інших провайдерах завершуються з поясненням, а не працюють без інструментів.",
                ),
                config_description: Some(
                    "Визначає, які розміщені інструменти OpenAI може викликати модель і як працює веб-пошук.",
                ),
                config_overlay: Some(json!({
                    "properties": {
                        TOOLS_KEY: {
                            "title": "Увімкнені інструменти",
                            "description": "Розміщені інструменти OpenAI, які може викликати модель.",
                            "items": {
                                "title": "Інструмент",
                                "enum_labels": { WEB_SEARCH: "Веб-пошук" },
                            },
                        },
                        CONTEXT_SIZE_KEY: {
                            "title": "Обсяг контексту веб-пошуку",
                            "description": "Скільки знайденого контексту модель може використати за один пошук.",
                        },
                        ALLOWED_DOMAINS_KEY: {
                            "title": "Дозволені домени",
                            "description": "Обмежити результати пошуку цими доменами, наприклад openai.com.",
                        },
                        LOCATION_KEY: {
                            "title": "Приблизне розташування",
                            "description": "Локалізує результати пошуку.",
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

    /// THREAT[TM-AGENT-029]: hosted web search gives the model provider-run web
    /// reach and sends conversation content to OpenAI-side tools. Everruns'
    /// egress controls (TM-AGENT-018) do not apply, the same exfil class as
    /// web_fetch (TM-AGENT-013), so assignment uses the admin-only trust gate.
    fn risk_level(&self) -> RiskLevel {
        RiskLevel::High
    }

    async fn system_prompt_contribution(&self, _ctx: &SystemPromptContext) -> Option<String> {
        None
    }

    fn config_schema(&self) -> Option<Value> {
        let tool_one_of: Vec<Value> = TOOL_NAMES
            .iter()
            .map(|(name, title)| json!({ "const": name, "title": title }))
            .collect();
        let context_sizes: Vec<&str> = SearchContextSize::ALL
            .iter()
            .map(|size| size.as_str())
            .collect();
        Some(json!({
            "type": "object",
            "properties": {
                TOOLS_KEY: {
                    "type": "array",
                    "title": "Enabled tools",
                    "description": "OpenAI hosted tools the model may invoke.",
                    "items": { "type": "string", "title": "Tool", "oneOf": tool_one_of },
                    "uniqueItems": true,
                },
                CONTEXT_SIZE_KEY: {
                    "type": "string",
                    "title": "Web search context size",
                    "description": "How much retrieved context the model may use per search. OpenAI defaults to medium.",
                    "enum": context_sizes,
                },
                ALLOWED_DOMAINS_KEY: {
                    "type": "array",
                    "title": "Allowed domains",
                    "description": "Restrict search results to these domains, for example openai.com.",
                    "items": { "type": "string" },
                    "uniqueItems": true,
                },
                LOCATION_KEY: {
                    "type": "object",
                    "title": "Approximate user location",
                    "description": "Localizes search results.",
                    "properties": {
                        "country": { "type": "string", "title": "Country", "description": "Two-letter ISO code, for example US." },
                        "region": { "type": "string", "title": "Region" },
                        "city": { "type": "string", "title": "City" },
                        "timezone": { "type": "string", "title": "Time zone", "description": "IANA name, for example America/Chicago." },
                    },
                    "additionalProperties": false,
                },
            },
            "additionalProperties": false,
        }))
    }

    fn config_ui_schema(&self) -> Option<Value> {
        Some(json!({
            "ui:order": [TOOLS_KEY, CONTEXT_SIZE_KEY, ALLOWED_DOMAINS_KEY, LOCATION_KEY],
            TOOLS_KEY: { "ui:widget": "checkboxes" },
        }))
    }

    fn validate_config(&self, config: &Value) -> Result<(), String> {
        if config.is_null() {
            return Ok(());
        }
        let obj = config
            .as_object()
            .ok_or_else(|| "config must be an object".to_string())?;
        for key in obj.keys() {
            if ![
                TOOLS_KEY,
                CONTEXT_SIZE_KEY,
                ALLOWED_DOMAINS_KEY,
                LOCATION_KEY,
            ]
            .contains(&key.as_str())
            {
                return Err(format!("unknown config key: {key}"));
            }
        }
        if let Some(tools) = obj.get(TOOLS_KEY) {
            let tools = tools
                .as_array()
                .ok_or_else(|| format!("`{TOOLS_KEY}` must be an array of tool names"))?;
            for tool in tools {
                let name = tool
                    .as_str()
                    .ok_or_else(|| format!("`{TOOLS_KEY}` entries must be strings"))?;
                if !TOOL_NAMES.iter().any(|(known, _)| *known == name) {
                    return Err(format!("unknown OpenAI server tool: {name}"));
                }
            }
        }
        if let Some(size) = obj.get(CONTEXT_SIZE_KEY)
            && serde_json::from_value::<SearchContextSize>(size.clone()).is_err()
        {
            return Err(format!("`{CONTEXT_SIZE_KEY}` must be low, medium or high"));
        }
        if let Some(domains) = obj.get(ALLOWED_DOMAINS_KEY) {
            let domains = domains
                .as_array()
                .ok_or_else(|| format!("`{ALLOWED_DOMAINS_KEY}` must be an array of domains"))?;
            for domain in domains {
                let domain = domain
                    .as_str()
                    .map(str::trim)
                    .filter(|domain| !domain.is_empty())
                    .ok_or_else(|| format!("`{ALLOWED_DOMAINS_KEY}` entries must be domains"))?;
                if domain.contains("://") || domain.contains('/') {
                    return Err(format!(
                        "`{ALLOWED_DOMAINS_KEY}` takes bare domains like openai.com, not {domain}"
                    ));
                }
            }
        }
        if let Some(location) = obj.get(LOCATION_KEY) {
            let location = location
                .as_object()
                .ok_or_else(|| format!("`{LOCATION_KEY}` must be an object"))?;
            for (key, value) in location {
                if !LOCATION_FIELDS.contains(&key.as_str()) {
                    return Err(format!("unknown `{LOCATION_KEY}` field: {key}"));
                }
                if !value.is_string() {
                    return Err(format!("`{LOCATION_KEY}.{key}` must be a string"));
                }
            }
        }
        Ok(())
    }

    fn driver_options(&self, config: &Value) -> Vec<(String, Value)> {
        hosted_tools_from_config(config)
            .to_driver_option()
            .into_iter()
            .collect()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use everruns_provider::openai_hosted_tools::OPENAI_HOSTED_TOOLS_OPTION;

    #[test]
    fn no_selected_tool_contributes_nothing() {
        let cap = OpenAiServerToolsCapability;
        assert!(cap.driver_options(&json!({})).is_empty());
        assert!(cap.driver_options(&Value::Null).is_empty());
        // Options without the tool itself do not enable it.
        assert!(
            cap.driver_options(&json!({ CONTEXT_SIZE_KEY: "high" }))
                .is_empty()
        );
    }

    #[test]
    fn web_search_contributes_the_driver_option() {
        let options = OpenAiServerToolsCapability.driver_options(&json!({
            "tools": ["web_search"],
            CONTEXT_SIZE_KEY: "low",
            ALLOWED_DOMAINS_KEY: ["openai.com", " "],
            LOCATION_KEY: { "country": "US", "city": "" },
        }));
        assert_eq!(options.len(), 1);
        assert_eq!(options[0].0, OPENAI_HOSTED_TOOLS_OPTION);
        let tools: OpenAiHostedTools = serde_json::from_value(options[0].1.clone()).unwrap();
        assert_eq!(
            tools.wire_tools(),
            vec![json!({
                "type": "web_search",
                "search_context_size": "low",
                "user_location": { "type": "approximate", "country": "US" },
                "filters": { "allowed_domains": ["openai.com"] },
            })]
        );
    }

    #[test]
    fn read_path_drops_what_validation_would_reject() {
        let tools = hosted_tools_from_config(&json!({
            "tools": ["web_search", "bogus"],
            CONTEXT_SIZE_KEY: "huge",
            LOCATION_KEY: "Kyiv",
        }));
        assert_eq!(tools.web_search, Some(WebSearchTool::default()));
    }

    #[test]
    fn validation_accepts_the_documented_shape_and_rejects_the_rest() {
        let cap = OpenAiServerToolsCapability;
        assert!(cap.validate_config(&Value::Null).is_ok());
        assert!(
            cap.validate_config(&json!({
                "tools": ["web_search"],
                CONTEXT_SIZE_KEY: "high",
                ALLOWED_DOMAINS_KEY: ["openai.com"],
                LOCATION_KEY: { "country": "UA", "timezone": "Europe/Kyiv" },
            }))
            .is_ok()
        );
        for bad in [
            json!({ "tools": ["code_interpreter"] }),
            json!({ "tools": "web_search" }),
            json!({ CONTEXT_SIZE_KEY: "huge" }),
            json!({ ALLOWED_DOMAINS_KEY: ["https://openai.com"] }),
            json!({ ALLOWED_DOMAINS_KEY: [""] }),
            json!({ LOCATION_KEY: { "street": "Main" } }),
            json!({ LOCATION_KEY: { "country": 1 } }),
            json!({ "extra": true }),
        ] {
            assert!(cap.validate_config(&bad).is_err(), "accepted {bad}");
        }
    }

    #[test]
    fn schema_matches_validation() {
        let schema = OpenAiServerToolsCapability.config_schema().unwrap();
        let one_of = schema["properties"][TOOLS_KEY]["items"]["oneOf"]
            .as_array()
            .unwrap();
        assert_eq!(one_of.len(), TOOL_NAMES.len());
        assert_eq!(
            schema["properties"][CONTEXT_SIZE_KEY]["enum"],
            json!(["low", "medium", "high"])
        );
    }

    #[test]
    fn ukrainian_overlay_labels_every_tool() {
        let loc = OpenAiServerToolsCapability
            .localizations()
            .into_iter()
            .find(|l| l.locale == "uk")
            .unwrap();
        let labels = &loc.config_overlay.unwrap()["properties"][TOOLS_KEY]["items"]["enum_labels"];
        for (name, _) in TOOL_NAMES {
            assert!(labels[name].as_str().is_some_and(|s| !s.is_empty()));
        }
    }
}
