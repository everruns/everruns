//! OpenAI Agents API as an opt-in runtime backend (EVE-1120, EVE-1123).
//!
//! OpenAI owns the agent loop; Everruns owns the event ledger. This module
//! holds the wire protocol: the session configuration mapped from a resolved
//! Everruns agent, the portable import back, the HTTP client, and helpers
//! over provider items. [`durable`] drives one Everruns turn through a
//! provider session with a write-ahead checkpoint so a worker restart or a
//! stream disconnect reconciles instead of repeating work. [`backend`] wires
//! that driver into the host's Reason activity for sessions that selected the
//! backend. Compiled only with the `openai-agents-api` Cargo feature; selected
//! only by the `openai_agents_api_runtime` capability, which the platform
//! strips unless the org has the `openai_agents_api` flag. Design, gaps, and
//! the recommendation live in `knowledge/execution/openai-agents-api-runtime.md`.

pub mod backend;
pub mod durable;
pub mod lifecycle;
pub mod seed;

use std::collections::HashMap;

use eventsource_stream::Eventsource;
use everruns_core::events::TokenUsage;
use everruns_core::{McpServerTransportType, RuntimeAgent, ScopedMcpServer, ScopedMcpServers};
use everruns_provider::BearerAuth;
use everruns_provider::runtime_provider::ProviderEndpoint;
use everruns_provider::tool_types::ToolDefinition;
use futures::StreamExt;
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use thiserror::Error;

/// Cargo feature that compiles the backend.
pub const CARGO_FEATURE: &str = "openai-agents-api";
/// Product feature flag that must be enabled before an org can select this backend.
pub const PRODUCT_FEATURE_FLAG: &str = "openai_agents_api";
/// Capability that selects this backend for an agent or session.
pub const RUNTIME_CAPABILITY_ID: &str = everruns_core::capabilities::OPENAI_AGENTS_API_RUNTIME_ID;

#[derive(Debug, Error)]
pub enum AgentsApiError {
    #[error("Agents API configuration is missing agent.model")]
    MissingModel,
    #[error("Agents API configuration is missing agent.instructions")]
    MissingInstructions,
    #[error("Everruns policy cannot be enforced on this Agents API configuration: {0}")]
    PolicyViolation(String),
    #[error("Agents API event is missing type")]
    MissingEventType,
    #[error("Agents API terminal event '{0}' is not supported")]
    UnsupportedTerminalEvent(String),
    #[error("Agents API stream closed before the root turn ended")]
    StreamClosedBeforeTurnEnded,
    #[error("Agents API request failed: {0}")]
    Http(String),
    #[error("Agents API returned HTTP {status}: {body}")]
    Api { status: u16, body: String },
    #[error("Agents API event '{event_type}' is missing {field}")]
    MissingEventField {
        event_type: String,
        field: &'static str,
    },
    #[error("Agents API reconciliation failed: {0}")]
    Reconcile(String),
    #[error("Agents API durable state: {0}")]
    Store(String),
    #[error("Agents API turn cancelled")]
    Cancelled,
}

impl AgentsApiError {
    fn is_conflict(&self) -> bool {
        matches!(self, Self::Api { status: 409, .. })
    }
}

/// One pending client function call from `session.required_actions`.
#[derive(Clone, Debug, PartialEq)]
pub struct FunctionCallAction {
    pub turn_id: String,
    pub call_id: String,
    pub name: String,
    pub arguments: Value,
}

impl FunctionCallAction {
    /// Pending function calls in a session resource or an
    /// `agent.session.requires_action` event. Other action kinds (environment
    /// connection, browser sign-in) are not function results and are skipped.
    pub fn from_required_actions(provider_value: &Value) -> Vec<Self> {
        provider_value
            .pointer("/session/required_actions")
            .or_else(|| provider_value.get("required_actions"))
            .and_then(Value::as_array)
            .into_iter()
            .flatten()
            .filter(|action| action.get("type").and_then(Value::as_str) == Some("function_call"))
            .filter_map(|action| {
                Some(Self {
                    turn_id: action.get("turn_id")?.as_str()?.to_string(),
                    call_id: action.get("call_id")?.as_str()?.to_string(),
                    name: action.get("name")?.as_str()?.to_string(),
                    arguments: arguments_value(action),
                })
            })
            .collect()
    }
}

/// Function arguments arrive as an object; tolerate a JSON-encoded string.
fn arguments_value(value: &Value) -> Value {
    match value.get("arguments") {
        Some(Value::String(raw)) => serde_json::from_str(raw).unwrap_or(Value::String(raw.clone())),
        Some(other) => other.clone(),
        None => json!({}),
    }
}

/// Build the documented continuation event for one client function result.
///
/// OpenAI takes the output as a string; an `Err` reports a failed call the
/// agent can react to instead of retrying blind.
pub fn build_tool_result_input(turn_id: &str, call_id: &str, result: Result<&str, &str>) -> Value {
    match result {
        Ok(output) => json!({
            "type": "agent.session.input.tool_result",
            "turn_id": turn_id,
            "call_id": call_id,
            "success": true,
            "output": output,
        }),
        Err(error) => json!({
            "type": "agent.session.input.tool_result",
            "turn_id": turn_id,
            "call_id": call_id,
            "success": false,
            "error": error,
        }),
    }
}

/// Build the documented follow-up user input event.
pub fn build_message_input(text: &str) -> Value {
    json!({
        "type": "agent.session.input.message",
        "input": [{"role": "user", "content": [{"type": "input_text", "text": text}]}],
    })
}

/// The session-create `input`: the turn's text alone, or, for a session
/// seeded from the Everruns record ([`seed`]), the transcript and the turn's
/// text as two user-role messages. Verified live on 2026-10-02: create input
/// takes user-role messages only, and the provider folds them into one user
/// item with one `input_text` part each.
pub fn build_create_input(seed: Option<&str>, text: &str) -> Value {
    let message = |text: &str| json!({"type": "message", "role": "user", "content": [{"type": "input_text", "text": text}]});
    match seed {
        Some(seed) => json!([message(seed), message(text)]),
        None => Value::String(text.to_string()),
    }
}

/// Session-create configuration for `POST /v1/agents/sessions`.
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
pub struct AgentsApiSessionConfig {
    pub agent: AgentsApiAgentConfig,
    pub environment: AgentsApiEnvironment,
    pub input: Value,
    pub stream: bool,
    /// Correlation metadata. Everruns stores its session id and create
    /// attempt here so an uncertain create can be adopted, not repeated.
    #[serde(default, skip_serializing_if = "HashMap::is_empty")]
    pub metadata: HashMap<String, String>,
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
pub struct AgentsApiAgentConfig {
    pub model: String,
    pub instructions: String,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub tools: Vec<AgentsApiTool>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub multi_agent: Option<AgentsApiMultiAgent>,
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
pub struct AgentsApiMultiAgent {
    pub enabled: bool,
    pub max_concurrent_subagents: u32,
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum AgentsApiEnvironment {
    None,
    OpenaiHosted,
    SelfHosted {
        workspace_directory: String,
        #[serde(default, skip_serializing_if = "Vec::is_empty")]
        capability_directories: Vec<String>,
    },
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum AgentsApiTool {
    Function {
        name: String,
        description: String,
        parameters: Value,
        #[serde(default, skip_serializing_if = "is_false")]
        defer_loading: bool,
    },
    Mcp {
        server_label: String,
        transport: AgentsApiMcpTransport,
        connection_origin: String,
        required: bool,
        /// Tool names the provider may call on this server. `None` allows all.
        #[serde(default, skip_serializing_if = "Option::is_none")]
        allowed_tools: Option<Vec<String>>,
    },
    #[serde(other)]
    Unsupported,
}

#[derive(Clone, Serialize, Deserialize, PartialEq)]
pub struct AgentsApiMcpTransport {
    #[serde(rename = "type")]
    pub transport_type: String,
    pub server_url: String,
    /// Only ever read from an imported configuration. Everruns never sends
    /// MCP credentials to the provider: [`AgentsApiSessionConfig::with_direct_mcp`]
    /// takes none, and [`AgentsApiSessionConfig::validate_direct_mcp`] refuses
    /// a configuration that carries any.
    #[serde(default, skip_serializing_if = "HashMap::is_empty")]
    pub headers: HashMap<String, String>,
}

// THREAT[TM-LLM-043]: MCP credentials never reach logs through Debug.
impl std::fmt::Debug for AgentsApiMcpTransport {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let mut header_names: Vec<&str> = self.headers.keys().map(String::as_str).collect();
        header_names.sort_unstable();
        f.debug_struct("AgentsApiMcpTransport")
            .field("transport_type", &self.transport_type)
            .field("server_url", &self.server_url)
            .field("headers", &format_args!("<redacted {header_names:?}>"))
            .finish()
    }
}

fn is_false(value: &bool) -> bool {
    !value
}

impl AgentsApiSessionConfig {
    /// Add a direct MCP server the provider calls itself, restricted to an
    /// explicit, non-empty tool allowlist.
    ///
    /// Direct MCP runs outside Everruns' tool pipeline (no approval gate, no
    /// pre-tool guardrails, no network policy), so the production backend
    /// never uses it; it exists for custom hosts and the live conformance
    /// test. It takes no credentials: an authenticated server stays behind
    /// Everruns' session-scoped MCP client and reaches the provider only as
    /// client functions.
    pub fn with_direct_mcp(
        mut self,
        server_label: &str,
        server_url: &str,
        allowed_tools: &[&str],
    ) -> Result<Self, AgentsApiError> {
        self.agent.tools.push(AgentsApiTool::Mcp {
            server_label: server_label.to_string(),
            transport: AgentsApiMcpTransport {
                transport_type: "http".to_string(),
                server_url: server_url.to_string(),
                headers: HashMap::new(),
            },
            connection_origin: "service".to_string(),
            required: true,
            allowed_tools: Some(allowed_tools.iter().map(|tool| tool.to_string()).collect()),
        });
        self.validate_direct_mcp()?;
        Ok(self)
    }

    /// Protocol-level invariants on direct MCP servers, checked before every
    /// session create: HTTPS only, an explicit non-empty tool allowlist, and
    /// no credentials.
    // THREAT[TM-LLM-043]: a provider-run MCP server can never carry Everruns
    // credentials or call tools nobody listed.
    pub fn validate_direct_mcp(&self) -> Result<(), AgentsApiError> {
        for tool in &self.agent.tools {
            let AgentsApiTool::Mcp {
                server_label,
                transport,
                allowed_tools,
                ..
            } = tool
            else {
                continue;
            };
            if transport.transport_type != "http"
                || !transport
                    .server_url
                    .to_ascii_lowercase()
                    .starts_with("https://")
            {
                return Err(AgentsApiError::PolicyViolation(format!(
                    "direct MCP server '{server_label}' must use an HTTPS URL"
                )));
            }
            if !transport.headers.is_empty() {
                return Err(AgentsApiError::PolicyViolation(format!(
                    "direct MCP server '{server_label}' carries credentials; authenticated MCP servers run through Everruns' session-scoped MCP client"
                )));
            }
            if allowed_tools
                .as_ref()
                .is_none_or(|tools| tools.is_empty() || tools.iter().any(|t| t.trim().is_empty()))
            {
                return Err(AgentsApiError::PolicyViolation(format!(
                    "direct MCP server '{server_label}' needs an explicit allowed-tool set"
                )));
            }
        }
        Ok(())
    }

    /// The production backend's boundary: every tool is a client function
    /// that crosses Everruns' tool pipeline. OpenAI built-ins, direct MCP,
    /// multi-agent delegation, and hosted environments run tools or models
    /// where Everruns cannot apply approval, guardrail, budget, or network
    /// policy, so a configuration that asks for any of them is refused.
    pub fn ensure_enforceable(&self) -> Result<(), AgentsApiError> {
        for tool in &self.agent.tools {
            match tool {
                AgentsApiTool::Function { .. } => {}
                AgentsApiTool::Mcp { server_label, .. } => {
                    return Err(AgentsApiError::PolicyViolation(format!(
                        "direct MCP server '{server_label}' would run tools outside Everruns' tool pipeline"
                    )));
                }
                AgentsApiTool::Unsupported => {
                    return Err(AgentsApiError::PolicyViolation(
                        "OpenAI built-in tools run outside Everruns' tool pipeline".to_string(),
                    ));
                }
            }
        }
        if self.agent.multi_agent.is_some() {
            return Err(AgentsApiError::PolicyViolation(
                "provider-managed subagents bypass Everruns delegation policy".to_string(),
            ));
        }
        if !matches!(self.environment, AgentsApiEnvironment::None) {
            return Err(AgentsApiError::PolicyViolation(
                "a provider environment runs commands outside Everruns' tool pipeline".to_string(),
            ));
        }
        Ok(())
    }

    /// Stable digest of the agent definition. A provider session keeps the
    /// agent it was created with, so a changed definition needs a new one.
    pub fn agent_fingerprint(&self) -> String {
        use sha2::{Digest, Sha256};
        let bytes = serde_json::to_vec(&(&self.agent, &self.environment)).unwrap_or_default();
        Sha256::digest(bytes)
            .iter()
            .map(|byte| format!("{byte:02x}"))
            .collect()
    }
}

/// Convert the resolved Everruns runtime configuration to an Agents API session request.
///
/// Every Everruns tool, including scoped MCP tools, becomes a client
/// function, so its execution crosses Everruns' tool pipeline and MCP
/// credentials stay with Everruns' session-scoped MCP client. Direct MCP is
/// opt-in through [`AgentsApiSessionConfig::with_direct_mcp`].
pub fn build_session_config(
    runtime_agent: &RuntimeAgent,
    input: impl Into<String>,
    max_concurrent_subagents: Option<u32>,
) -> Result<AgentsApiSessionConfig, AgentsApiError> {
    let tools = runtime_agent
        .tools
        .iter()
        .map(|tool| AgentsApiTool::Function {
            name: tool.name().to_string(),
            description: tool.description().to_string(),
            parameters: tool.full_parameters().clone(),
            defer_loading: false,
        })
        .collect::<Vec<_>>();

    Ok(AgentsApiSessionConfig {
        agent: AgentsApiAgentConfig {
            model: runtime_agent.model.clone(),
            instructions: runtime_agent.system_prompt.clone(),
            tools,
            multi_agent: max_concurrent_subagents.map(|limit| AgentsApiMultiAgent {
                enabled: true,
                max_concurrent_subagents: limit,
            }),
        },
        environment: AgentsApiEnvironment::None,
        input: Value::String(input.into()),
        stream: true,
        metadata: HashMap::new(),
    })
}

/// Result of importing the portable subset of an Agents API session configuration.
#[derive(Clone)]
pub struct ImportedAgentsApiConfig {
    pub runtime_agent: RuntimeAgent,
    pub mcp_servers: ScopedMcpServers,
    pub warnings: Vec<String>,
}

/// Import function tools and HTTP MCP servers into native Everruns configuration.
pub fn import_session_config(
    config: AgentsApiSessionConfig,
) -> Result<ImportedAgentsApiConfig, AgentsApiError> {
    if config.agent.model.trim().is_empty() {
        return Err(AgentsApiError::MissingModel);
    }
    if config.agent.instructions.trim().is_empty() {
        return Err(AgentsApiError::MissingInstructions);
    }

    let mut runtime_agent = RuntimeAgent::new(
        config.agent.instructions.clone(),
        config.agent.model.clone(),
    );
    let mut mcp_servers = ScopedMcpServers::default();
    let mut warnings = Vec::new();

    for tool in config.agent.tools {
        match tool {
            AgentsApiTool::Function {
                name,
                description,
                parameters,
                ..
            } => runtime_agent
                .tools
                .push(ToolDefinition::function(name, description, parameters)),
            AgentsApiTool::Mcp {
                server_label,
                transport,
                allowed_tools,
                ..
            } if transport.transport_type == "http" => {
                if allowed_tools.is_some() {
                    warnings.push(format!(
                        "MCP server '{server_label}' tool allowlist is not imported"
                    ));
                }
                // Credentials are never copied into an agent definition; the
                // server is reconnected through a session-scoped connection.
                if !transport.headers.is_empty() {
                    warnings.push(format!(
                        "MCP server '{server_label}' credentials are not imported; connect it again"
                    ));
                }
                mcp_servers.insert(
                    server_label,
                    ScopedMcpServer {
                        transport_type: McpServerTransportType::Http,
                        url: transport.server_url,
                        ..ScopedMcpServer::default()
                    },
                );
            }
            AgentsApiTool::Mcp { server_label, .. } => warnings.push(format!(
                "MCP server '{server_label}' uses an unsupported transport"
            )),
            AgentsApiTool::Unsupported => warnings
                .push("OpenAI built-in tool has no native Everruns import mapping".to_string()),
        }
    }
    if config.agent.multi_agent.is_some() {
        warnings.push(
            "Agents API multi-agent policy requires an explicit Everruns delegation mapping"
                .to_string(),
        );
    }
    if !matches!(config.environment, AgentsApiEnvironment::None) {
        warnings.push(
            "Agents API environment state and files are not imported into the agent definition"
                .to_string(),
        );
    }

    Ok(ImportedAgentsApiConfig {
        runtime_agent,
        mcp_servers,
        warnings,
    })
}

/// The text of a provider tool output: MCP `{content: [{text}]}`, a function
/// output `[{text}]`, or anything else serialized.
pub(crate) fn provider_output_text(item: &Value) -> String {
    let output = item.get("output").unwrap_or(&Value::Null);
    let parts = output
        .get("content")
        .or(Some(output))
        .and_then(Value::as_array)
        .map(|parts| {
            parts
                .iter()
                .filter_map(|part| part.get("text").and_then(Value::as_str))
                .collect::<Vec<_>>()
                .join("\n")
        })
        .filter(|text| !text.is_empty());
    parts.unwrap_or_else(|| match output {
        Value::String(text) => text.clone(),
        Value::Null => String::new(),
        other => other.to_string(),
    })
}

/// The assistant text of a saved `message` item.
pub(crate) fn message_item_text(item: &Value) -> String {
    item.get("content")
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
        .filter_map(|part| part.get("text").and_then(Value::as_str))
        .collect::<Vec<_>>()
        .join("")
}

pub(crate) fn is_subagent_event(provider_event: &Value) -> bool {
    provider_event
        .pointer("/turn/subagent_id")
        .or_else(|| provider_event.get("subagent_id"))
        .is_some_and(|id| !id.is_null())
}

/// OpenAI reports cached tokens inside `input_tokens`; Everruns keeps disjoint
/// buckets (see [`TokenUsage`]), so the cached subset is subtracted here. The
/// provider usage is best-effort and may be null, which is not zero. Public
/// so the server's late-usage reconciler (EVE-1145) reads a turn resource
/// exactly as the driver does.
pub fn usage_from(turn: &Value) -> Option<TokenUsage> {
    let usage = turn.get("usage").filter(|usage| !usage.is_null())?;
    let count = |pointer: &str| {
        usage
            .pointer(pointer)
            .and_then(Value::as_u64)
            .map(|value| u32::try_from(value).unwrap_or(u32::MAX))
    };
    let input = count("/input_tokens").unwrap_or_default();
    let cached = count("/input_tokens_details/cached_tokens");
    Some(TokenUsage::with_cache(
        input.saturating_sub(cached.unwrap_or_default()),
        count("/output_tokens").unwrap_or_default(),
        cached,
        None,
    ))
}

/// Default Agents API base URL.
pub const DEFAULT_BASE_URL: &str = "https://api.openai.com/v1";

/// Metadata key on an Agents API `llm.generation` naming the Everruns
/// provider whose credentials ran the turn, so usage the provider fills late
/// is read back with those same credentials (EVE-1145).
pub const GENERATION_PROVIDER_ID: &str = "everruns_provider_id";

/// Beta header value the Agents API requires on every request.
pub const BETA_HEADER: &str = "agents=v1";
/// Page size for item, turn, and session listings.
const PAGE_LIMIT: usize = 100;
/// Bound on listing pages, so a misbehaving cursor cannot loop forever.
const MAX_PAGES: usize = 50;

/// One page of a provider list response.
#[derive(Debug, Deserialize)]
struct ListPage {
    #[serde(default)]
    data: Vec<Value>,
    #[serde(default)]
    has_more: bool,
    #[serde(default)]
    last_id: Option<String>,
}

/// Provider event stream: each item is one SSE `data` payload.
pub type AgentsApiEventStream =
    std::pin::Pin<Box<dyn futures::Stream<Item = Result<Value, AgentsApiError>> + Send>>;

/// HTTP client for the Agents API. Credentials stay inside the
/// [`ProviderEndpoint`] the provider registry resolved; this type never
/// exposes them.
#[derive(Clone)]
pub struct AgentsApiClient {
    http: reqwest::Client,
    endpoint: ProviderEndpoint,
}

impl AgentsApiClient {
    /// Client for the official API with a bearer key (tests and tools).
    pub fn new(api_key: impl Into<String>) -> Self {
        Self::from_endpoint(ProviderEndpoint::from_parts(
            DEFAULT_BASE_URL,
            BearerAuth::new(api_key),
        ))
    }

    /// Client over an already-resolved provider endpoint.
    pub fn from_endpoint(endpoint: ProviderEndpoint) -> Self {
        Self {
            http: reqwest::Client::new(),
            endpoint,
        }
    }

    /// Same credentials, different base URL (tests against a fake server).
    pub fn with_base_url(self, base_url: impl Into<String>) -> Self {
        Self::from_endpoint(ProviderEndpoint::from_parts(
            base_url,
            EndpointAuth(self.endpoint),
        ))
    }

    async fn request(
        &self,
        method: reqwest::Method,
        path: &str,
        body: Option<&Value>,
    ) -> Result<reqwest::RequestBuilder, AgentsApiError> {
        let url = self
            .endpoint
            .url(path)
            .ok_or_else(|| AgentsApiError::Http("Agents API endpoint has no base URL".into()))?;
        let bytes = body
            .map(serde_json::to_vec)
            .transpose()
            .map_err(|error| AgentsApiError::Http(error.to_string()))?
            .unwrap_or_default();
        let resolved = self
            .endpoint
            .resolve(method.as_str(), url, &bytes)
            .await
            .map_err(|error| AgentsApiError::Http(error.to_string()))?;
        let mut request = self
            .http
            .request(method, &resolved.url)
            .header("OpenAI-Beta", BETA_HEADER);
        for (name, value) in &resolved.headers {
            request = request.header(name, value);
        }
        if body.is_some() {
            request = request
                .header("content-type", "application/json")
                .body(bytes);
        }
        Ok(request)
    }

    async fn send(request: reqwest::RequestBuilder) -> Result<reqwest::Response, AgentsApiError> {
        let response = request
            .send()
            .await
            .map_err(|error| AgentsApiError::Http(error.to_string()))?;
        let status = response.status();
        if status.is_success() {
            return Ok(response);
        }
        let body = response.text().await.unwrap_or_default();
        Err(AgentsApiError::Api {
            status: status.as_u16(),
            body: body.chars().take(2000).collect(),
        })
    }

    async fn get_json(&self, path: &str) -> Result<Value, AgentsApiError> {
        Self::send(self.request(reqwest::Method::GET, path, None).await?)
            .await?
            .json()
            .await
            .map_err(|error| AgentsApiError::Http(error.to_string()))
    }

    fn event_stream(response: reqwest::Response) -> AgentsApiEventStream {
        Box::pin(response.bytes_stream().eventsource().filter_map(|event| {
            futures::future::ready(match event {
                Ok(event) if event.data.trim().is_empty() || event.data == "[DONE]" => None,
                Ok(event) => Some(serde_json::from_str(&event.data).map_err(|error| {
                    AgentsApiError::Http(format!("invalid SSE payload: {error}"))
                })),
                Err(error) => Some(Err(AgentsApiError::Http(error.to_string()))),
            })
        }))
    }

    /// `POST /agents/sessions` with `stream: true`; yields each SSE payload.
    /// The first event carries the new session id.
    pub async fn create_session_stream(
        &self,
        config: &AgentsApiSessionConfig,
    ) -> Result<AgentsApiEventStream, AgentsApiError> {
        let mut body = config.clone();
        body.stream = true;
        let body =
            serde_json::to_value(&body).map_err(|error| AgentsApiError::Http(error.to_string()))?;
        let response = Self::send(
            self.request(reqwest::Method::POST, "agents/sessions", Some(&body))
                .await?
                .header("Accept", "text/event-stream"),
        )
        .await?;
        Ok(Self::event_stream(response))
    }

    /// `GET /agents/sessions/{id}/events?stream=true`: live events from now
    /// on. The provider does not replay missed events; callers reconcile from
    /// saved items after opening it.
    pub async fn stream_session_events(
        &self,
        session_id: &str,
    ) -> Result<AgentsApiEventStream, AgentsApiError> {
        let response = Self::send(
            self.request(
                reqwest::Method::GET,
                &format!(
                    "agents/sessions/{}/events?stream=true",
                    path_segment(session_id)?
                ),
                None,
            )
            .await?
            .header("Accept", "text/event-stream"),
        )
        .await?;
        Ok(Self::event_stream(response))
    }

    /// `POST /agents/sessions/{id}/events`: submit input, tool results, or
    /// cancel. `idempotency_key` makes a retried submission a no-op at the
    /// provider (verified live for input events on 2026-10-01).
    pub async fn send_events(
        &self,
        session_id: &str,
        events: Vec<Value>,
        idempotency_key: Option<&str>,
    ) -> Result<(), AgentsApiError> {
        let mut request = self
            .request(
                reqwest::Method::POST,
                &format!("agents/sessions/{}/events", path_segment(session_id)?),
                Some(&json!({ "events": events })),
            )
            .await?;
        if let Some(key) = idempotency_key {
            request = request.header("Idempotency-Key", key);
        }
        Self::send(request).await?;
        Ok(())
    }

    /// Where the provider's own trace of a session can be exported
    /// (`GET /agents/sessions/{id}/traces`, OTLP JSON). Recorded as a link on
    /// projected events; the driver never fetches or imports it.
    pub fn trace_url(&self, session_id: &str) -> Option<String> {
        let segment = path_segment(session_id).ok()?;
        self.endpoint
            .url(&format!("agents/sessions/{segment}/traces"))
    }

    /// `GET /agents/sessions/{id}`: status and pending required actions.
    pub async fn retrieve_session(&self, session_id: &str) -> Result<Value, AgentsApiError> {
        self.get_json(&format!("agents/sessions/{}", path_segment(session_id)?))
            .await
    }

    /// `DELETE /agents/sessions/{id}`: remove the provider session.
    pub async fn delete_session(&self, session_id: &str) -> Result<(), AgentsApiError> {
        Self::send(
            self.request(
                reqwest::Method::DELETE,
                &format!("agents/sessions/{}", path_segment(session_id)?),
                None,
            )
            .await?,
        )
        .await?;
        Ok(())
    }

    /// `GET /agents/sessions/{id}/turns/{turn}`: status, error, and usage.
    pub async fn retrieve_turn(
        &self,
        session_id: &str,
        turn_id: &str,
    ) -> Result<Value, AgentsApiError> {
        self.get_json(&format!(
            "agents/sessions/{}/turns/{}",
            path_segment(session_id)?,
            path_segment(turn_id)?
        ))
        .await
    }

    async fn list_all(&self, base: &str) -> Result<Vec<Value>, AgentsApiError> {
        let separator = if base.contains('?') { '&' } else { '?' };
        let mut all = Vec::new();
        let mut after: Option<String> = None;
        for _ in 0..MAX_PAGES {
            let mut path = format!("{base}{separator}limit={PAGE_LIMIT}");
            if let Some(cursor) = &after {
                path.push_str(&format!("&after={}", path_segment(cursor)?));
            }
            let page: ListPage = serde_json::from_value(self.get_json(&path).await?)
                .map_err(|error| AgentsApiError::Http(format!("invalid list page: {error}")))?;
            all.extend(page.data);
            match (page.has_more, page.last_id) {
                (true, Some(last)) => after = Some(last),
                _ => return Ok(all),
            }
        }
        Err(AgentsApiError::Reconcile(format!(
            "listing {base} exceeded {MAX_PAGES} pages"
        )))
    }

    /// Every turn of a session, root and subagent.
    pub async fn list_turns(&self, session_id: &str) -> Result<Vec<Value>, AgentsApiError> {
        self.list_all(&format!(
            "agents/sessions/{}/turns",
            path_segment(session_id)?
        ))
        .await
    }

    /// Saved root items of one turn, oldest first.
    pub async fn list_turn_items(
        &self,
        session_id: &str,
        turn_id: &str,
    ) -> Result<Vec<Value>, AgentsApiError> {
        self.list_all(&format!(
            "agents/sessions/{}/items?order=asc&turn_id={}",
            path_segment(session_id)?,
            path_segment(turn_id)?
        ))
        .await
    }

    /// The most recent sessions, newest first (one page).
    pub async fn list_recent_sessions(&self) -> Result<Vec<Value>, AgentsApiError> {
        let page: ListPage = serde_json::from_value(
            self.get_json(&format!("agents/sessions?limit={PAGE_LIMIT}"))
                .await?,
        )
        .map_err(|error| AgentsApiError::Http(format!("invalid list page: {error}")))?;
        Ok(page.data)
    }
}

/// Reuse a resolved endpoint's authentication under another base URL.
struct EndpointAuth(ProviderEndpoint);

#[async_trait::async_trait]
impl everruns_provider::ProviderAuth for EndpointAuth {
    async fn headers(
        &self,
        request: everruns_provider::ProviderAuthRequest<'_>,
    ) -> everruns_provider::error::Result<Vec<(String, String)>> {
        Ok(self
            .0
            .resolve(request.method, request.url, request.body)
            .await?
            .headers)
    }
    fn as_any(&self) -> &dyn std::any::Any {
        self
    }
}

/// Provider ids are opaque; refuse anything that could alter the request path.
fn path_segment(id: &str) -> Result<&str, AgentsApiError> {
    if !id.is_empty()
        && id
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || b == b'_' || b == b'-')
    {
        Ok(id)
    } else {
        Err(AgentsApiError::Http(format!("invalid provider id '{id}'")))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use everruns_provider::tool_types::ClientSideTool;

    fn prototype_config() -> AgentsApiSessionConfig {
        let mut agent = RuntimeAgent::new("Use both tools.", "gpt-6-astra");
        agent
            .tools
            .push(ToolDefinition::ClientSide(ClientSideTool::new(
                "lookup_customer",
                "Look up one customer",
                json!({
                    "type": "object",
                    "properties": {"customer_id": {"type": "string"}},
                    "required": ["customer_id"],
                    "additionalProperties": false
                }),
            )));
        build_session_config(&agent, "Use the function and MCP tools.", Some(2))
            .unwrap()
            .with_direct_mcp(
                "docs",
                "https://developers.openai.com/mcp",
                &["search_openai_docs"],
            )
            .unwrap()
    }

    #[test]
    fn session_config_maps_one_function_and_one_allowed_mcp_tool() {
        let config = prototype_config();
        assert_eq!(config.agent.model, "gpt-6-astra");
        assert_eq!(
            serde_json::to_value(&config.agent.tools).unwrap(),
            json!([
                {
                    "type": "function",
                    "name": "lookup_customer",
                    "description": "Look up one customer",
                    "parameters": {
                        "type": "object",
                        "properties": {"customer_id": {"type": "string"}},
                        "required": ["customer_id"],
                        "additionalProperties": false
                    }
                },
                {
                    "type": "mcp",
                    "server_label": "docs",
                    "transport": {
                        "type": "http",
                        "server_url": "https://developers.openai.com/mcp"
                    },
                    "connection_origin": "service",
                    "required": true,
                    "allowed_tools": ["search_openai_docs"]
                }
            ])
        );
    }

    #[test]
    fn agent_fingerprint_tracks_the_agent_not_the_input() {
        let first = prototype_config();
        let mut second = first.clone();
        second.input = json!("another turn");
        second.metadata.insert("k".into(), "v".into());
        assert_eq!(first.agent_fingerprint(), second.agent_fingerprint());
        second.agent.instructions.push_str(" Be brief.");
        assert_ne!(first.agent_fingerprint(), second.agent_fingerprint());
    }

    #[test]
    fn unknown_usage_is_not_reported_as_zero() {
        assert!(usage_from(&json!({"usage": null})).is_none());
        let usage = usage_from(&json!({"usage": {
            "input_tokens": 100, "input_tokens_details": {"cached_tokens": 40}, "output_tokens": 7
        }}))
        .unwrap();
        assert_eq!(usage.input_tokens, 60);
        assert_eq!(usage.output_tokens, 7);
    }

    #[test]
    fn function_result_and_message_inputs_match_the_documented_shapes() {
        let session = json!({
            "id": "sess_1",
            "required_actions": [
                {"type": "environment_connection"},
                {"type": "function_call", "turn_id": "turn_1", "call_id": "call_customer",
                 "name": "lookup_customer", "arguments": "{\"customer_id\":\"123\"}"}
            ]
        });
        let actions = FunctionCallAction::from_required_actions(&session);
        assert_eq!(actions.len(), 1);
        assert_eq!(actions[0].arguments, json!({"customer_id": "123"}));
        assert_eq!(
            build_tool_result_input("turn_1", "call_customer", Ok(r#"{"name":"Ada"}"#)),
            json!({
                "type": "agent.session.input.tool_result",
                "turn_id": "turn_1",
                "call_id": "call_customer",
                "success": true,
                "output": "{\"name\":\"Ada\"}"
            })
        );
        assert_eq!(
            build_tool_result_input("turn_1", "call_customer", Err("denied")),
            json!({
                "type": "agent.session.input.tool_result",
                "turn_id": "turn_1",
                "call_id": "call_customer",
                "success": false,
                "error": "denied"
            })
        );
        assert_eq!(
            build_message_input("hi"),
            json!({"type": "agent.session.input.message",
                   "input": [{"role": "user", "content": [{"type": "input_text", "text": "hi"}]}]})
        );
    }

    #[test]
    fn a_seeded_create_sends_the_transcript_then_the_input_as_user_messages() {
        assert_eq!(build_create_input(None, "hi"), json!("hi"));
        assert_eq!(
            build_create_input(Some("record"), "hi"),
            json!([
                {"type": "message", "role": "user", "content": [{"type": "input_text", "text": "record"}]},
                {"type": "message", "role": "user", "content": [{"type": "input_text", "text": "hi"}]}
            ])
        );
    }

    #[test]
    fn import_preserves_portable_tools_and_reports_nonportable_state() {
        let mut config = prototype_config();
        config.environment = AgentsApiEnvironment::OpenaiHosted;
        config.agent.tools.push(AgentsApiTool::Unsupported);
        let imported = import_session_config(config).unwrap();
        assert_eq!(imported.runtime_agent.tools.len(), 1);
        assert_eq!(
            imported.mcp_servers["docs"].url,
            "https://developers.openai.com/mcp"
        );
        // Built-in, allowlist, multi-agent, and environment warnings.
        assert_eq!(imported.warnings.len(), 4);
    }

    #[test]
    fn imported_mcp_credentials_are_dropped_and_never_debug_printed() {
        let mut config = prototype_config();
        if let AgentsApiTool::Mcp { transport, .. } = &mut config.agent.tools[1] {
            transport
                .headers
                .insert("Authorization".into(), "Bearer sk-mcp-secret".into());
        }
        assert!(
            !format!("{config:?}").contains("sk-mcp-secret"),
            "Debug output redacts MCP credentials"
        );
        assert!(matches!(
            config.validate_direct_mcp(),
            Err(AgentsApiError::PolicyViolation(_))
        ));
        let imported = import_session_config(config).unwrap();
        assert!(imported.mcp_servers["docs"].headers.is_empty());
        assert!(
            imported
                .warnings
                .iter()
                .any(|warning| warning.contains("credentials are not imported"))
        );
    }

    #[test]
    fn direct_mcp_needs_https_and_an_explicit_allowlist() {
        let base =
            build_session_config(&RuntimeAgent::new("Test.", "gpt-6-astra"), "", None).unwrap();
        for (url, tools) in [
            ("http://example.com/mcp", vec!["search"]),
            ("https://example.com/mcp", vec![]),
            ("https://example.com/mcp", vec![" "]),
        ] {
            assert!(
                matches!(
                    base.clone().with_direct_mcp("x", url, &tools),
                    Err(AgentsApiError::PolicyViolation(_))
                ),
                "{url} {tools:?}"
            );
        }
        let mut no_allowlist = base
            .clone()
            .with_direct_mcp("x", "https://example.com/mcp", &["search"])
            .unwrap();
        if let AgentsApiTool::Mcp { allowed_tools, .. } = &mut no_allowlist.agent.tools[0] {
            *allowed_tools = None;
        }
        assert!(no_allowlist.validate_direct_mcp().is_err());
    }

    #[test]
    fn the_production_boundary_refuses_tools_it_cannot_police() {
        let agent = RuntimeAgent::new("Test.", "gpt-6-astra");
        let functions_only = build_session_config(&agent, "", None).unwrap();
        assert!(functions_only.ensure_enforceable().is_ok());
        let refused = [
            prototype_config(),
            {
                let mut config = functions_only.clone();
                config.agent.tools.push(AgentsApiTool::Unsupported);
                config
            },
            build_session_config(&agent, "", Some(2)).unwrap(),
            {
                let mut config = functions_only.clone();
                config.environment = AgentsApiEnvironment::OpenaiHosted;
                config
            },
        ];
        for config in refused {
            assert!(matches!(
                config.ensure_enforceable(),
                Err(AgentsApiError::PolicyViolation(_))
            ));
        }
    }

    #[test]
    fn provider_ids_cannot_alter_request_paths() {
        assert!(path_segment("sess_abc-1").is_ok());
        assert!(path_segment("../x").is_err());
        assert!(path_segment("a?b").is_err());
        assert!(path_segment("").is_err());
    }
}
