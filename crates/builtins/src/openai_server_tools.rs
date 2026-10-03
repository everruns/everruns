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
// Web search, code interpreter, hosted shell, file search and remote MCP.
//
// Remote MCP approvals default to `always`: OpenAI stops at each call and the
// turn pauses on a synthetic approval call until a person answers it through
// the session's tool-results path. `never` is accepted only with an explicit
// `allowed_tools` list, so skipping approval is a per-tool decision.
//
// A server that needs credentials is named by `mcp_server`, a registered
// Everruns MCP server; the host resolves its URL, API key or OAuth token per
// call (`everruns_contracts::hosted_mcp`). Config never holds a credential:
// headers are not an accepted field and URLs with userinfo are rejected.

use async_trait::async_trait;
use everruns_contracts::openai_hosted_tools::{
    ContainerMemory, ContainerTool, FileSearchTool, McpApproval, McpServerTool, OpenAiHostedTools,
    SearchContextSize, WebSearchTool, WebSearchUserLocation,
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
const MEMORY_KEY: &str = "container_memory_limit";
const VECTOR_STORES_KEY: &str = "file_search_vector_store_ids";
const MAX_RESULTS_KEY: &str = "file_search_max_results";
const MCP_SERVERS_KEY: &str = "mcp_servers";
const MCP_FIELDS: [&str; 5] = [
    "server_label",
    "server_url",
    "mcp_server",
    "allowed_tools",
    "require_approval",
];
const CONFIG_KEYS: [&str; 8] = [
    TOOLS_KEY,
    CONTEXT_SIZE_KEY,
    ALLOWED_DOMAINS_KEY,
    LOCATION_KEY,
    MEMORY_KEY,
    VECTOR_STORES_KEY,
    MAX_RESULTS_KEY,
    MCP_SERVERS_KEY,
];
const WEB_SEARCH: &str = "web_search";
const CODE_INTERPRETER: &str = "code_interpreter";
const SHELL: &str = "shell";
const FILE_SEARCH: &str = "file_search";
const MCP: &str = "mcp";

/// Hosted tool names this capability accepts, in UI order.
const TOOL_NAMES: [(&str, &str); 5] = [
    (WEB_SEARCH, "Web search"),
    (CODE_INTERPRETER, "Code interpreter"),
    (SHELL, "Hosted shell"),
    (FILE_SEARCH, "File search"),
    (MCP, "Remote MCP"),
];

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
        allowed_domains: string_list(config.get(ALLOWED_DOMAINS_KEY)),
    });
    let container = || ContainerTool {
        memory_limit: config
            .get(MEMORY_KEY)
            .and_then(|memory| serde_json::from_value::<ContainerMemory>(memory.clone()).ok()),
    };
    let vector_store_ids: Vec<String> = string_list(config.get(VECTOR_STORES_KEY));
    OpenAiHostedTools {
        web_search,
        code_interpreter: enabled(CODE_INTERPRETER).then(container),
        shell: enabled(SHELL).then(container),
        // OpenAI rejects file search without a vector store, so no ids means off.
        file_search: (enabled(FILE_SEARCH) && !vector_store_ids.is_empty()).then(|| {
            FileSearchTool {
                vector_store_ids,
                max_num_results: config
                    .get(MAX_RESULTS_KEY)
                    .and_then(Value::as_u64)
                    .filter(|max| (1..=50).contains(max))
                    .map(|max| max as u32),
            }
        }),
        mcp_servers: if enabled(MCP) {
            mcp_servers_from_config(config.get(MCP_SERVERS_KEY))
        } else {
            Vec::new()
        },
        // Native computer use is requested by the computer_use capability,
        // not configured here.
        computer: None,
    }
}

/// Servers from the read path. A server without a label or an https URL is
/// dropped, and `never` without an allow-list falls back to `always`.
fn mcp_servers_from_config(value: Option<&Value>) -> Vec<McpServerTool> {
    let Some(servers) = value.and_then(Value::as_array) else {
        return Vec::new();
    };
    servers
        .iter()
        .filter_map(|server| {
            let text = |key: &str| {
                server
                    .get(key)
                    .and_then(Value::as_str)
                    .map(str::trim)
                    .filter(|value| !value.is_empty())
            };
            let server_label = text("server_label")?.to_string();
            // A registered server's URL and credentials are resolved per call
            // by the host, so config names it and carries neither.
            let mcp_server = text("mcp_server")
                .filter(|name| everruns_core::mcp_server::is_valid_mcp_server_name(name));
            let server_url = match mcp_server {
                Some(_) => "",
                None => text("server_url").filter(|url| mcp_url_error(url).is_none())?,
            };
            let allowed_tools = string_list(server.get("allowed_tools"));
            let require_approval = match text("require_approval") {
                Some("never") if !allowed_tools.is_empty() => McpApproval::Never,
                _ => McpApproval::Always,
            };
            Some(McpServerTool {
                server_label,
                server_url: server_url.to_string(),
                mcp_server: mcp_server.map(str::to_string),
                allowed_tools,
                require_approval,
                ..Default::default()
            })
        })
        .collect()
}

/// Why `url` is not an acceptable remote MCP server URL.
///
/// Credentials never ride in the URL: userinfo is rejected because it would be
/// stored in agent config and echoed to OpenAI (THREAT TM-AGENT-029).
fn mcp_url_error(url: &str) -> Option<String> {
    let Some(rest) = url.strip_prefix("https://") else {
        return Some(format!("MCP server URL must use https: {url}"));
    };
    let authority = rest.split(['/', '?', '#']).next().unwrap_or_default();
    if authority.is_empty() {
        return Some(format!("MCP server URL has no host: {url}"));
    }
    if authority.contains('@') {
        return Some("MCP server URL must not contain credentials".to_string());
    }
    None
}

/// Trimmed, non-empty strings from a JSON array; anything else is dropped.
fn string_list(value: Option<&Value>) -> Vec<String> {
    value
        .and_then(Value::as_array)
        .map(|items| {
            items
                .iter()
                .filter_map(Value::as_str)
                .map(str::trim)
                .filter(|item| !item.is_empty())
                .map(str::to_string)
                .collect()
        })
        .unwrap_or_default()
}

fn validate_mcp_servers(servers: &Value) -> Result<(), String> {
    let servers = servers
        .as_array()
        .ok_or_else(|| format!("`{MCP_SERVERS_KEY}` must be an array of servers"))?;
    let mut labels = std::collections::HashSet::new();
    for server in servers {
        let server = server
            .as_object()
            .ok_or_else(|| format!("`{MCP_SERVERS_KEY}` entries must be objects"))?;
        if let Some(key) = server
            .keys()
            .find(|key| !MCP_FIELDS.contains(&key.as_str()))
        {
            return Err(format!("unknown MCP server field: {key}"));
        }
        let label = server
            .get("server_label")
            .and_then(Value::as_str)
            .unwrap_or_default();
        if label.is_empty()
            || !label
                .chars()
                .all(|c| c.is_ascii_alphanumeric() || c == '-' || c == '_')
        {
            return Err(format!(
                "MCP `server_label` must be letters, digits, - or _, got {label:?}"
            ));
        }
        if !labels.insert(label) {
            return Err(format!("duplicate MCP server_label: {label}"));
        }
        match (server.get("server_url"), server.get("mcp_server")) {
            (Some(_), Some(_)) => {
                return Err(format!(
                    "MCP server {label} takes `server_url` or `mcp_server`, not both"
                ));
            }
            (_, Some(name)) => {
                let name = name.as_str().unwrap_or_default();
                if !everruns_core::mcp_server::is_valid_mcp_server_name(name) {
                    return Err(format!(
                        "MCP `mcp_server` must name a registered server, got {name:?}"
                    ));
                }
            }
            (url, None) => {
                let url = url.and_then(Value::as_str).unwrap_or_default();
                if let Some(error) = mcp_url_error(url.trim()) {
                    return Err(error);
                }
            }
        }
        if let Some(tools) = server.get("allowed_tools")
            && !tools.as_array().is_some_and(|tools| {
                tools
                    .iter()
                    .all(|t| t.as_str().is_some_and(|t| !t.trim().is_empty()))
            })
        {
            return Err("MCP `allowed_tools` must be an array of tool names".to_string());
        }
        match server.get("require_approval").map(|v| v.as_str()) {
            None | Some(Some("always")) => {}
            Some(Some("never")) if !string_list(server.get("allowed_tools")).is_empty() => {}
            Some(Some("never")) => {
                return Err(format!(
                    "MCP server {label} can skip approval only for an explicit `allowed_tools` list"
                ));
            }
            _ => return Err("MCP `require_approval` must be always or never".to_string()),
        }
    }
    Ok(())
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
        "Enables OpenAI's hosted tools (web search, code interpreter, hosted shell, \
         file search, remote MCP) on agents that run on the OpenAI or Azure OpenAI \
         Responses API. OpenAI runs these tools in its own infrastructure and sends \
         conversation content to them and to the remote MCP servers you list. Agents on other providers fail with an explanation instead \
         of running without the tools."
    }

    fn localizations(&self) -> Vec<CapabilityLocalization> {
        vec![
            CapabilityLocalization {
                locale: "en",
                name: None,
                description: None,
                config_description: Some(
                    "Choose which OpenAI hosted tools the model may invoke and how they behave.",
                ),
                config_overlay: None,
            },
            CapabilityLocalization {
                locale: "uk",
                name: Some("Серверні інструменти OpenAI"),
                description: Some(
                    "Вмикає розміщені інструменти OpenAI (веб-пошук, інтерпретатор коду, розміщений командний рядок, пошук у файлах, віддалений MCP) для агентів на OpenAI або Azure OpenAI Responses API. Інструменти виконує OpenAI у власній інфраструктурі, і вони та вказані віддалені MCP-сервери отримують вміст розмови. Агенти на інших провайдерах завершуються з поясненням, а не працюють без інструментів.",
                ),
                config_description: Some(
                    "Визначає, які розміщені інструменти OpenAI може викликати модель і як вони працюють.",
                ),
                config_overlay: Some(json!({
                    "properties": {
                        TOOLS_KEY: {
                            "title": "Увімкнені інструменти",
                            "description": "Розміщені інструменти OpenAI, які може викликати модель.",
                            "items": {
                                "title": "Інструмент",
                                "enum_labels": {
                                    WEB_SEARCH: "Веб-пошук",
                                    CODE_INTERPRETER: "Інтерпретатор коду",
                                    SHELL: "Розміщений командний рядок",
                                    FILE_SEARCH: "Пошук у файлах",
                                    MCP: "Віддалений MCP",
                                },
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
                        MEMORY_KEY: {
                            "title": "Пам'ять контейнера",
                            "description": "Пам'ять контейнера OpenAI для інтерпретатора коду й командного рядка. Більший контейнер коштує дорожче.",
                        },
                        VECTOR_STORES_KEY: {
                            "title": "Векторні сховища",
                            "description": "Ідентифікатори векторних сховищ OpenAI (vs_...), у яких шукає пошук у файлах.",
                        },
                        MAX_RESULTS_KEY: {
                            "title": "Максимум результатів",
                            "description": "Скільки результатів пошуку у файлах повертати, від 1 до 50.",
                        },
                        MCP_SERVERS_KEY: {
                            "title": "Віддалені MCP-сервери",
                            "description": "Сервери, до яких OpenAI підключається від імені моделі. Кожен виклик чекає на схвалення, якщо не вказано дозволені інструменти й require_approval: never.",
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
        let memory_limits: Vec<&str> = ContainerMemory::ALL
            .iter()
            .map(|memory| memory.as_str())
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
                MEMORY_KEY: {
                    "type": "string",
                    "title": "Container memory",
                    "description": "Memory for the OpenAI container that runs code interpreter and hosted shell. Larger containers cost more. OpenAI defaults to 1g.",
                    "enum": memory_limits,
                },
                VECTOR_STORES_KEY: {
                    "type": "array",
                    "title": "Vector stores",
                    "description": "OpenAI vector store ids (vs_...) that file search reads. Required for file search.",
                    "items": { "type": "string" },
                    "uniqueItems": true,
                },
                MCP_SERVERS_KEY: {
                    "type": "array",
                    "title": "Remote MCP servers",
                    "description": "Servers OpenAI connects to for the model. Each call waits for approval unless the server lists allowed tools and sets require_approval to never.",
                    "items": {
                        "type": "object",
                        "properties": {
                            "server_label": { "type": "string", "title": "Label", "description": "Letters, digits, - and _; unique." },
                            "server_url": { "type": "string", "title": "URL", "description": "https URL of a public MCP server, without credentials. Use this or mcp_server." },
                            "mcp_server": { "type": "string", "title": "Registered server", "description": "Name of an MCP server registered in Everruns. Its URL and credentials are resolved for every call and never stored here." },
                            "allowed_tools": { "type": "array", "title": "Allowed tools", "items": { "type": "string" }, "uniqueItems": true },
                            "require_approval": { "type": "string", "title": "Approval", "enum": ["always", "never"], "default": "always" },
                        },
                        "required": ["server_label"],
                        "additionalProperties": false,
                    },
                },
                MAX_RESULTS_KEY: {
                    "type": "integer",
                    "title": "Max file search results",
                    "description": "Results per file search, 1 to 50.",
                    "minimum": 1,
                    "maximum": 50,
                },
            },
            "additionalProperties": false,
        }))
    }

    fn config_ui_schema(&self) -> Option<Value> {
        Some(json!({
            "ui:order": CONFIG_KEYS,
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
            if !CONFIG_KEYS.contains(&key.as_str()) {
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
        if let Some(memory) = obj.get(MEMORY_KEY)
            && serde_json::from_value::<ContainerMemory>(memory.clone()).is_err()
        {
            return Err(format!("`{MEMORY_KEY}` must be 1g, 4g, 16g or 64g"));
        }
        if let Some(ids) = obj.get(VECTOR_STORES_KEY) {
            let ids = ids
                .as_array()
                .ok_or_else(|| format!("`{VECTOR_STORES_KEY}` must be an array of ids"))?;
            if ids
                .iter()
                .any(|id| id.as_str().is_none_or(|id| id.trim().is_empty()))
            {
                return Err(format!(
                    "`{VECTOR_STORES_KEY}` entries must be vector store ids"
                ));
            }
        }
        if let Some(max) = obj.get(MAX_RESULTS_KEY)
            && !max.as_u64().is_some_and(|max| (1..=50).contains(&max))
        {
            return Err(format!(
                "`{MAX_RESULTS_KEY}` must be a whole number from 1 to 50"
            ));
        }
        let file_search_on = obj
            .get(TOOLS_KEY)
            .and_then(Value::as_array)
            .is_some_and(|tools| tools.iter().any(|tool| tool.as_str() == Some(FILE_SEARCH)));
        if file_search_on && string_list(obj.get(VECTOR_STORES_KEY)).is_empty() {
            return Err(format!(
                "file search needs at least one vector store id in `{VECTOR_STORES_KEY}`"
            ));
        }
        if let Some(servers) = obj.get(MCP_SERVERS_KEY) {
            validate_mcp_servers(servers)?;
        }
        let mcp_on = obj
            .get(TOOLS_KEY)
            .and_then(Value::as_array)
            .is_some_and(|tools| tools.iter().any(|tool| tool.as_str() == Some(MCP)));
        if mcp_on
            && obj
                .get(MCP_SERVERS_KEY)
                .and_then(Value::as_array)
                .is_none_or(Vec::is_empty)
        {
            return Err(format!(
                "remote MCP needs at least one server in `{MCP_SERVERS_KEY}`"
            ));
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
    use everruns_contracts::openai_hosted_tools::OPENAI_HOSTED_TOOLS_OPTION;

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
        // File search without a vector store would be rejected by OpenAI.
        let tools = hosted_tools_from_config(&json!({ "tools": ["file_search"] }));
        assert!(tools.is_empty());
    }

    #[test]
    fn mcp_servers_default_to_approval() {
        let tools = hosted_tools_from_config(&json!({
            "tools": ["mcp"],
            MCP_SERVERS_KEY: [
                { "server_label": "deepwiki", "server_url": "https://mcp.deepwiki.com/mcp" },
                { "server_label": "docs", "server_url": "https://docs.example/mcp",
                  "allowed_tools": ["search"], "require_approval": "never" },
                // Read path: `never` without an allow-list still asks.
                { "server_label": "loose", "server_url": "https://loose.example/mcp",
                  "require_approval": "never" },
                { "server_label": "plain", "server_url": "http://plain.example/mcp" },
            ],
        }));
        let approvals: Vec<_> = tools
            .mcp_servers
            .iter()
            .map(|s| (s.server_label.as_str(), s.require_approval))
            .collect();
        assert_eq!(
            approvals,
            [
                ("deepwiki", McpApproval::Always),
                ("docs", McpApproval::Never),
                ("loose", McpApproval::Always),
            ]
        );
        // Listing servers without enabling the tool contributes nothing.
        let off = hosted_tools_from_config(&json!({
            MCP_SERVERS_KEY: [{ "server_label": "a", "server_url": "https://a.example" }],
        }));
        assert!(off.is_empty());
    }

    #[test]
    fn registered_mcp_server_carries_no_url_or_credentials() {
        let config = json!({ "tools": ["mcp"], MCP_SERVERS_KEY: [
            { "server_label": "gh", "mcp_server": "github" },
        ]});
        assert!(OpenAiServerToolsCapability.validate_config(&config).is_ok());
        let server = hosted_tools_from_config(&config).mcp_servers.remove(0);
        assert_eq!(server.mcp_server.as_deref(), Some("github"));
        assert!(server.server_url.is_empty() && server.headers.is_empty());
        for bad in [
            json!({ "server_label": "gh", "mcp_server": "github", "server_url": "https://a.example" }),
            json!({ "server_label": "gh", "mcp_server": "" }),
            json!({ "server_label": "gh", "mcp_server": "bad__name" }),
        ] {
            let config = json!({ "tools": ["mcp"], MCP_SERVERS_KEY: [bad.clone()] });
            assert!(
                OpenAiServerToolsCapability
                    .validate_config(&config)
                    .is_err(),
                "accepted {bad}"
            );
        }
    }

    #[test]
    fn mcp_validation_rejects_unsafe_servers() {
        let cap = OpenAiServerToolsCapability;
        let with = |server: Value| json!({ "tools": ["mcp"], MCP_SERVERS_KEY: [server] });
        assert!(
            cap.validate_config(&with(json!({
                "server_label": "docs", "server_url": "https://docs.example/mcp",
                "allowed_tools": ["search"], "require_approval": "never",
            })))
            .is_ok()
        );
        for bad in [
            json!({ "server_label": "docs", "server_url": "http://docs.example/mcp" }),
            json!({ "server_label": "docs", "server_url": "https://user:pw@docs.example/mcp" }),
            json!({ "server_label": "has space", "server_url": "https://docs.example" }),
            json!({ "server_label": "docs", "server_url": "https://docs.example", "require_approval": "never" }),
            json!({ "server_label": "docs", "server_url": "https://docs.example", "require_approval": "sometimes" }),
            json!({ "server_label": "docs", "server_url": "https://docs.example", "headers": {} }),
        ] {
            assert!(
                cap.validate_config(&with(bad.clone())).is_err(),
                "accepted {bad}"
            );
        }
        let duplicate = json!({ "tools": ["mcp"], MCP_SERVERS_KEY: [
            { "server_label": "a", "server_url": "https://a.example" },
            { "server_label": "a", "server_url": "https://b.example" },
        ]});
        assert!(cap.validate_config(&duplicate).is_err());
    }

    #[test]
    fn container_and_file_tools_contribute_their_options() {
        let tools = hosted_tools_from_config(&json!({
            "tools": ["code_interpreter", "shell", "file_search"],
            MEMORY_KEY: "4g",
            VECTOR_STORES_KEY: ["vs_1"],
            MAX_RESULTS_KEY: 8,
        }));
        let container = Some(ContainerTool {
            memory_limit: Some(ContainerMemory::FourGb),
        });
        assert_eq!(tools.code_interpreter, container);
        assert_eq!(tools.shell, container);
        assert_eq!(
            tools.file_search,
            Some(FileSearchTool {
                vector_store_ids: vec!["vs_1".into()],
                max_num_results: Some(8),
            })
        );
        assert_eq!(tools.web_search, None);
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
        assert!(
            cap.validate_config(&json!({
                "tools": ["code_interpreter", "shell", "file_search"],
                MEMORY_KEY: "16g",
                VECTOR_STORES_KEY: ["vs_1"],
                MAX_RESULTS_KEY: 50,
            }))
            .is_ok()
        );
        for bad in [
            json!({ "tools": ["image_generation"] }),
            json!({ "tools": ["file_search"] }),
            json!({ "tools": ["mcp"] }),
            json!({ "tools": ["file_search"], VECTOR_STORES_KEY: [" "] }),
            json!({ MEMORY_KEY: "2g" }),
            json!({ MAX_RESULTS_KEY: 0 }),
            json!({ MAX_RESULTS_KEY: 51 }),
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
