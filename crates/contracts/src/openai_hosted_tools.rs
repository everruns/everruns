//! OpenAI hosted (provider-executed) tools for the Responses API.
//!
//! Hosted tools run inside OpenAI: the model calls them and OpenAI feeds the
//! result back within the same response, so the agent loop never dispatches
//! them. A capability contributes the selected tools as request intent under
//! [`OPENAI_HOSTED_TOOLS_OPTION`] in `LlmCallConfig::driver_options`, and the
//! Responses driver appends them to the request `tools` array.
//!
//! Only endpoints that implement OpenAI's hosted tools render them (the OpenAI
//! and Azure OpenAI drivers). Every other driver must reject the option rather
//! than drop it, so an agent never silently loses a tool it was configured
//! with. Web search, code interpreter, hosted shell, file search and remote
//! MCP are supported (EVE-1115).
//!
//! The native `computer` tool (EVE-1133) is rendered from here too, but it is
//! not provider-executed: every `computer_call` is answered by the client. The
//! driver sets [`OpenAiHostedTools::computer`] itself from the provider-neutral
//! [`crate::native_computer`] request, never from this option, and its calls
//! are neither hosted-call events nor priced here.
//!
//! Remote MCP approvals are the one hosted interaction that needs a person:
//! OpenAI stops the response at an `mcp_approval_request` and continues only
//! when the next request carries an `mcp_approval_response`. The driver
//! surfaces the request as a synthetic [`OPENAI_MCP_APPROVAL_TOOL`] call so the
//! engine pauses the turn through its client-side tool path, and turns the
//! answer back into the approval response on replay.
//!
//! Wire shapes: <https://platform.openai.com/docs/guides/tools>.

use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::collections::{BTreeMap, HashMap};

use crate::native_computer::NativeComputerUse;

/// Synthetic tool call name for a remote MCP approval request. Never offered
/// to the model; only the driver produces it.
pub const OPENAI_MCP_APPROVAL_TOOL: &str = "openai_mcp_approval";

/// `driver_options` key carrying [`OpenAiHostedTools`].
pub const OPENAI_HOSTED_TOOLS_OPTION: &str = "openai/hosted_tools";

/// Driver ids whose Responses endpoint executes OpenAI hosted tools.
pub const HOSTED_TOOLS_DRIVER_IDS: [&str; 2] = ["openai", "azure_openai"];

/// Hosted tools selected for one call.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct OpenAiHostedTools {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub web_search: Option<WebSearchTool>,
    /// Python in an OpenAI-managed container.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub code_interpreter: Option<ContainerTool>,
    /// Shell commands in an OpenAI-managed container.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub shell: Option<ContainerTool>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub file_search: Option<FileSearchTool>,
    /// Remote MCP servers OpenAI calls on the model's behalf.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub mcp_servers: Vec<McpServerTool>,
    /// The native `computer` tool. Client-executed: see the module docs.
    /// Never read from or written to the hosted-tools option, so it cannot
    /// make a call look provider-executed.
    #[serde(skip)]
    pub computer: Option<NativeComputerUse>,
}

impl OpenAiHostedTools {
    pub fn is_empty(&self) -> bool {
        self.web_search.is_none()
            && self.code_interpreter.is_none()
            && self.shell.is_none()
            && self.file_search.is_none()
            && self.mcp_servers.is_empty()
            && self.computer.is_none()
    }

    /// Read the option from a call's `driver_options`. A malformed payload is
    /// an error, not "no tools": the caller asked for hosted tools.
    pub fn from_driver_options(
        options: &HashMap<String, Value>,
    ) -> Result<Option<Self>, serde_json::Error> {
        options
            .get(OPENAI_HOSTED_TOOLS_OPTION)
            .map(|raw| serde_json::from_value::<Self>(raw.clone()))
            .transpose()
            .map(|tools| tools.filter(|tools| !tools.is_empty()))
    }

    /// The `(key, value)` pair a capability contributes, or `None` when empty.
    pub fn to_driver_option(&self) -> Option<(String, Value)> {
        if self.is_empty() {
            return None;
        }
        let value = serde_json::to_value(self).ok()?;
        Some((OPENAI_HOSTED_TOOLS_OPTION.to_string(), value))
    }

    /// Responses `tools` entries, in a stable order.
    pub fn wire_tools(&self) -> Vec<Value> {
        let mut tools: Vec<Value> = self.web_search.iter().map(WebSearchTool::wire).collect();
        tools.extend(self.code_interpreter.as_ref().map(|tool| {
            serde_json::json!({ "type": "code_interpreter", "container": tool.container("auto") })
        }));
        tools.extend(self.shell.as_ref().map(|tool| {
            serde_json::json!({ "type": "shell", "environment": tool.container("container_auto") })
        }));
        tools.extend(self.file_search.as_ref().map(FileSearchTool::wire));
        tools.extend(self.mcp_servers.iter().map(McpServerTool::wire));
        tools.extend(self.computer.map(|_| crate::openai_computer::wire_tool()));
        tools
    }
}

/// How much retrieved context the model may use per search.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum SearchContextSize {
    Low,
    Medium,
    High,
}

impl SearchContextSize {
    pub const ALL: [Self; 3] = [Self::Low, Self::Medium, Self::High];

    pub fn as_str(self) -> &'static str {
        match self {
            Self::Low => "low",
            Self::Medium => "medium",
            Self::High => "high",
        }
    }
}

/// Approximate location used to localize search results.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct WebSearchUserLocation {
    /// Two-letter ISO country code, e.g. `US`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub country: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub region: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub city: Option<String>,
    /// IANA time zone, e.g. `America/Chicago`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub timezone: Option<String>,
}

impl WebSearchUserLocation {
    fn is_empty(&self) -> bool {
        self.country.is_none()
            && self.region.is_none()
            && self.city.is_none()
            && self.timezone.is_none()
    }
}

/// OpenAI's hosted `web_search` tool.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct WebSearchTool {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub search_context_size: Option<SearchContextSize>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub user_location: Option<WebSearchUserLocation>,
    /// Restrict results to these domains (no scheme, e.g. `openai.com`).
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub allowed_domains: Vec<String>,
}

impl WebSearchTool {
    /// The Responses `tools` entry for this tool.
    pub fn wire(&self) -> Value {
        let mut tool = serde_json::json!({ "type": "web_search" });
        if let Some(size) = self.search_context_size {
            tool["search_context_size"] = Value::from(size.as_str());
        }
        if let Some(location) = self.user_location.as_ref().filter(|l| !l.is_empty()) {
            let mut wire = serde_json::to_value(location).unwrap_or_default();
            wire["type"] = Value::from("approximate");
            tool["user_location"] = wire;
        }
        if !self.allowed_domains.is_empty() {
            tool["filters"] = serde_json::json!({ "allowed_domains": self.allowed_domains });
        }
        tool
    }
}

/// Container memory for code interpreter and hosted shell. OpenAI bills per
/// container session by size, so the choice is a cost knob.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum ContainerMemory {
    #[serde(rename = "1g")]
    OneGb,
    #[serde(rename = "4g")]
    FourGb,
    #[serde(rename = "16g")]
    SixteenGb,
    #[serde(rename = "64g")]
    SixtyFourGb,
}

impl ContainerMemory {
    pub const ALL: [Self; 4] = [
        Self::OneGb,
        Self::FourGb,
        Self::SixteenGb,
        Self::SixtyFourGb,
    ];

    pub fn as_str(self) -> &'static str {
        match self {
            Self::OneGb => "1g",
            Self::FourGb => "4g",
            Self::SixteenGb => "16g",
            Self::SixtyFourGb => "64g",
        }
    }
}

/// A hosted tool that runs in an automatically created OpenAI container
/// (`code_interpreter`, `shell`). The container is OpenAI's sandbox, not an
/// Everruns one: nothing in it is visible to the session file system.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct ContainerTool {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub memory_limit: Option<ContainerMemory>,
}

impl ContainerTool {
    /// The auto-container spec; `kind` differs per tool (`auto` for code
    /// interpreter, `container_auto` for shell).
    fn container(&self, kind: &str) -> Value {
        let mut container = serde_json::json!({ "type": kind });
        if let Some(memory) = self.memory_limit {
            container["memory_limit"] = Value::from(memory.as_str());
        }
        container
    }
}

/// OpenAI's hosted `file_search` over the org's OpenAI vector stores.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct FileSearchTool {
    /// OpenAI vector store ids (`vs_...`). Required by OpenAI.
    pub vector_store_ids: Vec<String>,
    /// Cap on results per search (OpenAI allows 1 to 50).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub max_num_results: Option<u32>,
}

impl FileSearchTool {
    pub fn wire(&self) -> Value {
        let mut tool = serde_json::json!({
            "type": "file_search",
            "vector_store_ids": self.vector_store_ids,
        });
        if let Some(max) = self.max_num_results {
            tool["max_num_results"] = Value::from(max);
        }
        tool
    }
}

/// Whether OpenAI asks a person before each remote MCP tool call.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum McpApproval {
    /// Every call pauses the turn for approval (the default).
    #[default]
    Always,
    /// Calls run without asking. Only for an explicit `allowed_tools` list.
    Never,
}

/// A remote MCP server OpenAI connects to (`{"type": "mcp"}`).
#[derive(Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct McpServerTool {
    /// Label OpenAI reports on every call, unique within the request.
    pub server_label: String,
    /// `https` URL of the server. Empty until resolved when `mcp_server` is set.
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub server_url: String,
    /// A registered Everruns MCP server whose URL and credentials the host
    /// resolves per call ([`crate::hosted_mcp`]).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub mcp_server: Option<String>,
    /// Request headers from that resolution. Never configured by an agent.
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub headers: BTreeMap<String, String>,
    /// Tools the model may call; empty means every tool the server lists.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub allowed_tools: Vec<String>,
    #[serde(default)]
    pub require_approval: McpApproval,
}

impl std::fmt::Debug for McpServerTool {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("McpServerTool")
            .field("server_label", &self.server_label)
            .field("server_url", &self.server_url)
            .field("mcp_server", &self.mcp_server)
            .field("headers", &self.headers.keys().collect::<Vec<_>>())
            .field("allowed_tools", &self.allowed_tools)
            .field("require_approval", &self.require_approval)
            .finish()
    }
}

impl McpServerTool {
    pub fn wire(&self) -> Value {
        let mut tool = serde_json::json!({
            "type": "mcp",
            "server_label": self.server_label,
            "server_url": self.server_url,
            "require_approval": match self.require_approval {
                McpApproval::Always => "always",
                McpApproval::Never => "never",
            },
        });
        if !self.allowed_tools.is_empty() {
            tool["allowed_tools"] = serde_json::json!(self.allowed_tools);
        }
        if !self.headers.is_empty() {
            tool["headers"] = serde_json::json!(self.headers);
        }
        tool
    }
}

/// The synthetic call arguments for an `mcp_approval_request` output item, or
/// `None` for any other item. Returns `(approval_request_id, arguments)`.
pub fn mcp_approval_call(item: &Value) -> Option<(String, Value)> {
    if item.get("type").and_then(Value::as_str) != Some("mcp_approval_request") {
        return None;
    }
    let field = |key: &str| item.get(key).and_then(Value::as_str).unwrap_or_default();
    let id = field("id");
    if id.is_empty() {
        return None;
    }
    // The tool's own arguments stay a JSON string, exactly as OpenAI sent
    // them, so replay hands back the same request it approved.
    let arguments = serde_json::json!({
        "server_label": field("server_label"),
        "name": field("name"),
        "arguments": field("arguments"),
    });
    Some((id.to_string(), arguments))
}

/// Count hosted tool calls in a terminal Responses `output` array, keyed by
/// item type (`web_search_call`, ...). Hosted calls bill per call on top of
/// tokens, so this is what cost accounting prices.
pub fn count_hosted_tool_calls(output: &[Value]) -> BTreeMap<String, u32> {
    let mut counts = BTreeMap::new();
    for item in output {
        if let Some(kind) = item.get("type").and_then(Value::as_str)
            && hosted_call_tool(kind).is_some()
        {
            *counts.entry(kind.to_string()).or_insert(0) += 1;
        }
    }
    counts
}

/// The configured tool name for a hosted call output item type, or `None`
/// when the item is not a hosted call.
pub fn hosted_call_tool(item_type: &str) -> Option<&'static str> {
    match item_type {
        "web_search_call" => Some("web_search"),
        "code_interpreter_call" => Some("code_interpreter"),
        "shell_call" => Some("shell"),
        "file_search_call" => Some("file_search"),
        "mcp_call" => Some("mcp"),
        _ => None,
    }
}

/// Price-table estimate for one hosted call, in USD.
///
/// OpenAI prices web search per 1,000 calls: $10 for reasoning models
/// (GPT-5 and newer, o-series) and $25 for GPT-4o / GPT-4.1, whose rate also
/// covers the search content tokens. Other models' search content tokens are
/// already in the reported input usage. File search is $2.50 per 1,000 calls.
/// Code interpreter and shell bill per container session, not per call, and
/// one container serves many calls across turns, so they stay unpriced here.
/// Source: openai.com/api/pricing.
pub fn hosted_call_price_usd(item_type: &str, model: &str) -> Option<f64> {
    match item_type {
        "web_search_call" if model.starts_with("gpt-4o") || model.starts_with("gpt-4.1") => {
            Some(25.0 / 1000.0)
        }
        "web_search_call" => Some(10.0 / 1000.0),
        "file_search_call" => Some(2.5 / 1000.0),
        _ => None,
    }
}

/// Estimated USD for every hosted call in `counts`; `None` when there are none
/// or none is priced.
pub fn hosted_calls_cost_usd(counts: &BTreeMap<String, u32>, model: &str) -> Option<f64> {
    counts
        .iter()
        .filter_map(|(kind, count)| hosted_call_price_usd(kind, model).map(|p| p * *count as f64))
        .reduce(|a, b| a + b)
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn bare_web_search_renders_the_minimal_wire_tool() {
        let tools = OpenAiHostedTools {
            web_search: Some(WebSearchTool::default()),
            ..Default::default()
        };
        assert_eq!(tools.wire_tools(), vec![json!({ "type": "web_search" })]);
    }

    #[test]
    fn computer_renders_the_native_tool_and_is_never_a_hosted_call() {
        let tools = OpenAiHostedTools {
            computer: Some(NativeComputerUse {
                display_width: 1280,
                display_height: 800,
            }),
            ..Default::default()
        };
        assert!(!tools.is_empty());
        assert_eq!(tools.wire_tools(), vec![json!({ "type": "computer" })]);
        assert_eq!(hosted_call_tool("computer_call"), None);
        let output = vec![json!({ "type": "computer_call", "call_id": "c" })];
        assert!(count_hosted_tool_calls(&output).is_empty());
    }

    #[test]
    fn mcp_server_renders_in_openai_shape() {
        let open = McpServerTool {
            server_label: "deepwiki".into(),
            server_url: "https://mcp.deepwiki.com/mcp".into(),
            ..Default::default()
        };
        assert_eq!(
            open.wire(),
            json!({ "type": "mcp", "server_label": "deepwiki",
                    "server_url": "https://mcp.deepwiki.com/mcp", "require_approval": "always" })
        );
        let trusted = McpServerTool {
            allowed_tools: vec!["ask_question".into()],
            require_approval: McpApproval::Never,
            ..open
        };
        assert_eq!(trusted.wire()["require_approval"], "never");
        assert_eq!(trusted.wire()["allowed_tools"], json!(["ask_question"]));
    }

    #[test]
    fn approval_request_becomes_a_synthetic_call() {
        let (id, arguments) = mcp_approval_call(&json!({
            "type": "mcp_approval_request", "id": "mcpr_1", "server_label": "deepwiki",
            "name": "ask_question", "arguments": "{\"q\":\"x\"}"
        }))
        .unwrap();
        assert_eq!(id, "mcpr_1");
        assert_eq!(
            arguments,
            json!({ "server_label": "deepwiki", "name": "ask_question", "arguments": "{\"q\":\"x\"}" })
        );
        assert!(mcp_approval_call(&json!({ "type": "mcp_call", "id": "mcp_1" })).is_none());
    }

    #[test]
    fn container_and_file_tools_render_in_openai_shape() {
        let tools = OpenAiHostedTools {
            code_interpreter: Some(ContainerTool {
                memory_limit: Some(ContainerMemory::FourGb),
            }),
            shell: Some(ContainerTool::default()),
            file_search: Some(FileSearchTool {
                vector_store_ids: vec!["vs_1".into()],
                max_num_results: Some(5),
            }),
            ..Default::default()
        };
        assert_eq!(
            tools.wire_tools(),
            vec![
                json!({ "type": "code_interpreter", "container": { "type": "auto", "memory_limit": "4g" } }),
                json!({ "type": "shell", "environment": { "type": "container_auto" } }),
                json!({ "type": "file_search", "vector_store_ids": ["vs_1"], "max_num_results": 5 }),
            ]
        );
    }

    #[test]
    fn web_search_options_render_in_openai_shape() {
        let tool = WebSearchTool {
            search_context_size: Some(SearchContextSize::High),
            user_location: Some(WebSearchUserLocation {
                country: Some("UA".into()),
                city: Some("Kyiv".into()),
                ..Default::default()
            }),
            allowed_domains: vec!["openai.com".into()],
        };
        assert_eq!(
            tool.wire(),
            json!({
                "type": "web_search",
                "search_context_size": "high",
                "user_location": { "type": "approximate", "country": "UA", "city": "Kyiv" },
                "filters": { "allowed_domains": ["openai.com"] },
            })
        );
    }

    #[test]
    fn empty_location_is_omitted() {
        let tool = WebSearchTool {
            user_location: Some(WebSearchUserLocation::default()),
            ..Default::default()
        };
        assert_eq!(tool.wire(), json!({ "type": "web_search" }));
    }

    #[test]
    fn driver_option_round_trips_and_empty_contributes_nothing() {
        assert!(OpenAiHostedTools::default().to_driver_option().is_none());

        let tools = OpenAiHostedTools {
            web_search: Some(WebSearchTool {
                search_context_size: Some(SearchContextSize::Low),
                ..Default::default()
            }),
            ..Default::default()
        };
        let (key, value) = tools.to_driver_option().expect("non-empty");
        assert_eq!(key, OPENAI_HOSTED_TOOLS_OPTION);
        let options = HashMap::from([(key, value)]);
        assert_eq!(
            OpenAiHostedTools::from_driver_options(&options).unwrap(),
            Some(tools)
        );
        assert_eq!(
            OpenAiHostedTools::from_driver_options(&HashMap::new()).unwrap(),
            None
        );
    }

    #[test]
    fn malformed_option_is_an_error_not_silence() {
        let options = HashMap::from([(
            OPENAI_HOSTED_TOOLS_OPTION.to_string(),
            json!({ "web_search": { "search_context_size": "huge" } }),
        )]);
        assert!(OpenAiHostedTools::from_driver_options(&options).is_err());
    }

    #[test]
    fn counts_only_hosted_call_items() {
        let output = vec![
            json!({ "type": "web_search_call", "id": "ws_1", "status": "completed" }),
            json!({ "type": "reasoning", "id": "rs_1" }),
            json!({ "type": "web_search_call", "id": "ws_2", "status": "completed" }),
            json!({ "type": "shell_call_output", "id": "sho_1" }),
            json!({ "type": "shell_call", "id": "sh_1" }),
            json!({ "type": "message", "id": "msg_1" }),
        ];
        let counts = count_hosted_tool_calls(&output);
        assert_eq!(counts.len(), 2);
        assert_eq!(counts["shell_call"], 1);
        assert_eq!(counts["web_search_call"], 2);
    }

    #[test]
    fn hosted_calls_are_priced_per_model_family() {
        let counts = BTreeMap::from([("web_search_call".to_string(), 3)]);
        let sol = hosted_calls_cost_usd(&counts, "gpt-6.1-sol").unwrap();
        assert!((sol - 0.03).abs() < 1e-9, "{sol}");
        let legacy = hosted_calls_cost_usd(&counts, "gpt-4.1-mini").unwrap();
        assert!((legacy - 0.075).abs() < 1e-9, "{legacy}");
        assert_eq!(hosted_calls_cost_usd(&BTreeMap::new(), "gpt-6.1-sol"), None);
        let files = BTreeMap::from([("file_search_call".to_string(), 4)]);
        let files = hosted_calls_cost_usd(&files, "gpt-6.1-sol").unwrap();
        assert!((files - 0.01).abs() < 1e-9, "{files}");
        let unpriced = BTreeMap::from([("code_interpreter_call".to_string(), 1)]);
        assert_eq!(hosted_calls_cost_usd(&unpriced, "gpt-6.1-sol"), None);
    }
}
