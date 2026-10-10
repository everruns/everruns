//! Portable MCP transport, authentication, and scoped configuration values.
//!
//! HTTP transport is available to hosted runtimes; local hosts may also use
//! stdio. Persisted MCP server records and their lifecycle live in the server.

use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::collections::{BTreeMap, HashMap};
use std::str::FromStr;

#[cfg(feature = "openapi")]
use utoipa::ToSchema;

/// MCP Server transport type.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[cfg_attr(feature = "openapi", derive(ToSchema))]
#[cfg_attr(feature = "openapi", schema(example = "http"))]
#[serde(rename_all = "lowercase")]
pub enum McpServerTransportType {
    /// HTTP (Streamable HTTP) transport.
    Http,
    /// Local-process transport over stdio. Only usable by single-tenant
    /// runtime/CLI hosts (e.g. the example coding CLI); the hosted product
    /// rejects it during scoped-config validation.
    Stdio,
}

impl McpServerTransportType {
    /// Whether this transport spawns/contacts a local process rather than a
    /// remote endpoint.
    pub fn is_local(&self) -> bool {
        matches!(self, McpServerTransportType::Stdio)
    }
}

/// MCP server authentication mode.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq, Default)]
#[cfg_attr(feature = "openapi", derive(ToSchema))]
#[cfg_attr(feature = "openapi", schema(example = "api_key"))]
#[serde(rename_all = "snake_case")]
pub enum McpServerAuthMode {
    /// No authentication required.
    #[default]
    None,
    /// Organization-scoped API key stored on the MCP server config.
    ApiKey,
    /// User-scoped OAuth token resolved at runtime.
    #[serde(rename = "oauth", alias = "o_auth")]
    OAuth,
}

impl std::fmt::Display for McpServerAuthMode {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            McpServerAuthMode::None => write!(f, "none"),
            McpServerAuthMode::ApiKey => write!(f, "api_key"),
            McpServerAuthMode::OAuth => write!(f, "oauth"),
        }
    }
}

impl From<&str> for McpServerAuthMode {
    fn from(s: &str) -> Self {
        match s {
            "api_key" => McpServerAuthMode::ApiKey,
            "oauth" => McpServerAuthMode::OAuth,
            _ => McpServerAuthMode::None,
        }
    }
}

impl McpServerAuthMode {
    pub fn is_none(&self) -> bool {
        matches!(self, McpServerAuthMode::None)
    }
}
/// Identity whose OAuth grant a scoped MCP attachment requests.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
#[cfg_attr(feature = "openapi", derive(ToSchema))]
#[cfg_attr(feature = "openapi", schema(example = "service"))]
#[serde(rename_all = "lowercase")]
pub enum McpServerActsAs {
    /// Do not resolve a credential for this attachment.
    #[default]
    None,
    /// Resolve the agent service identity's grant.
    Service,
    /// Resolve the invoking user's grant.
    User,
    /// Resolve the invoking user's grant when they have one, otherwise the
    /// agent service identity's. Unattended runs always use the service grant.
    /// Only ever chosen explicitly; each tool call records which one it used.
    #[serde(rename = "user_or_service")]
    UserOrService,
}

impl McpServerActsAs {
    /// Return whether the attachment requests no acting identity.
    pub fn is_none(&self) -> bool {
        matches!(self, Self::None)
    }

    /// Whether the invoking user's own grant can serve this attachment.
    pub fn uses_user_grant(&self) -> bool {
        matches!(self, Self::User | Self::UserOrService)
    }

    /// Whether the agent service identity's grant can serve this attachment.
    pub fn uses_service_grant(&self) -> bool {
        matches!(self, Self::Service | Self::UserOrService)
    }

    /// The concrete identities to try, in order. `user_or_service` tries the
    /// invoking user first, then the agent.
    pub fn resolution_order(&self) -> &'static [McpServerActsAs] {
        match self {
            Self::None => &[],
            Self::Service => &[Self::Service],
            Self::User => &[Self::User],
            Self::UserOrService => &[Self::User, Self::Service],
        }
    }
}

impl std::fmt::Display for McpServerActsAs {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::None => write!(f, "none"),
            Self::Service => write!(f, "service"),
            Self::User => write!(f, "user"),
            Self::UserOrService => write!(f, "user_or_service"),
        }
    }
}

impl From<&str> for McpServerActsAs {
    fn from(value: &str) -> Self {
        match value {
            "service" => Self::Service,
            "user" => Self::User,
            "user_or_service" => Self::UserOrService,
            _ => Self::None,
        }
    }
}

/// Whether a missing sign-in for an agent MCP attachment may pause the turn
/// with an in-chat Connect card.
///
/// Decision: `never` exists for agents behind channels that cannot render a
/// card (user MCP servers D5). The call then fails like any other tool error,
/// naming the server and where to connect it, and the turn keeps going. It
/// changes only how a missing grant is reported, never which grant is used.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
#[cfg_attr(feature = "openapi", derive(ToSchema))]
#[cfg_attr(feature = "openapi", schema(example = "ask"))]
#[serde(rename_all = "lowercase")]
pub enum McpConnectInChat {
    /// Pause the turn with a Connect card (or an Authorize / Ask admin card
    /// for the agent's own login).
    #[default]
    Ask,
    /// Return a tool error carrying the settings link instead of a card.
    Never,
}

impl McpConnectInChat {
    /// Whether this is the default, `ask`.
    pub fn is_ask(&self) -> bool {
        matches!(self, Self::Ask)
    }

    /// Whether a missing sign-in may be offered as an in-chat card.
    pub fn allows_card(&self) -> bool {
        self.is_ask()
    }
}

impl std::fmt::Display for McpConnectInChat {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Ask => write!(f, "ask"),
            Self::Never => write!(f, "never"),
        }
    }
}

impl From<&str> for McpConnectInChat {
    /// Anything but `never` (including an empty string from an older peer)
    /// reads as the default.
    fn from(value: &str) -> Self {
        match value {
            "never" => Self::Never,
            _ => Self::Ask,
        }
    }
}

/// Reference to an organization MCP server catalog entry.
#[derive(Debug, Clone, PartialEq, Eq)]
#[cfg_attr(feature = "openapi", derive(ToSchema))]
#[cfg_attr(feature = "openapi", schema(value_type = String, example = "catalog:linear"))]
pub struct McpServerPresetRef(String);

impl McpServerPresetRef {
    /// Return the organization catalog entry name without the `catalog:` prefix.
    pub fn catalog_name(&self) -> &str {
        // Construction only admits `catalog:` references, so the fallback never runs.
        self.0.strip_prefix("catalog:").unwrap_or(&self.0)
    }
}

impl std::fmt::Display for McpServerPresetRef {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(&self.0)
    }
}

impl FromStr for McpServerPresetRef {
    type Err = String;

    fn from_str(value: &str) -> Result<Self, Self::Err> {
        let Some(name) = value.strip_prefix("catalog:") else {
            return Err("MCP server preset reference must start with 'catalog:'".to_string());
        };
        if name.trim().is_empty() {
            return Err("MCP server catalog preset name cannot be empty".to_string());
        }
        Ok(Self(value.to_string()))
    }
}

impl Serialize for McpServerPresetRef {
    fn serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: serde::Serializer,
    {
        serializer.serialize_str(&self.0)
    }
}

impl<'de> Deserialize<'de> for McpServerPresetRef {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: serde::Deserializer<'de>,
    {
        String::deserialize(deserializer)?
            .parse()
            .map_err(serde::de::Error::custom)
    }
}

// Protocol-era and elicitation policies live in `mcp_server/policy.rs`.
mod policy;
pub use policy::{
    MCP_PROTOCOL_VERSION_2025_03, MCP_PROTOCOL_VERSION_2025_06, MCP_PROTOCOL_VERSION_2026_07,
    McpElicitationPolicy, McpProtocolMode,
};

/// Normalize a JSON-RPC error code across MCP eras.
///
/// `2026-07-28` renumbered the older MCP-specific `-32002` ("invalid
/// params"-class failure) onto the standard JSON-RPC `-32602` ("Invalid
/// params"). Callers that branch on the code should normalize first so servers
/// on either side of that change are handled identically.
pub fn normalize_mcp_error_code(code: i64) -> i64 {
    match code {
        -32002 => -32602,
        other => other,
    }
}

impl std::fmt::Display for McpServerTransportType {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            McpServerTransportType::Http => write!(f, "http"),
            McpServerTransportType::Stdio => write!(f, "stdio"),
        }
    }
}

impl From<&str> for McpServerTransportType {
    fn from(s: &str) -> Self {
        match s {
            "stdio" => McpServerTransportType::Stdio,
            // Default to HTTP for "http" and any unknown value.
            _ => McpServerTransportType::Http,
        }
    }
}

/// Session-, agent-, or harness-scoped remote MCP server configuration.
///
/// This intentionally mirrors the `mcpServers` object shape used by common MCP
/// client config files while staying within Everruns' current remote-HTTP-only
/// support.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[cfg_attr(feature = "openapi", derive(ToSchema))]
#[serde(try_from = "ScopedMcpServerWire", into = "ScopedMcpServerWire")]
pub struct ScopedMcpServer {
    /// MCP transport type. Only remote HTTP is supported today.
    #[serde(
        default = "default_scoped_transport_type",
        rename = "type",
        alias = "transport_type"
    )]
    #[cfg_attr(feature = "openapi", schema(rename = "type"))]
    pub transport_type: McpServerTransportType,
    /// URL of the remote MCP server endpoint. Required for HTTP transport;
    /// empty/ignored for stdio.
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub url: String,
    /// Additional HTTP headers sent on MCP requests (HTTP transport only).
    #[serde(default, skip_serializing_if = "HashMap::is_empty")]
    pub headers: HashMap<String, String>,
    /// Executable to spawn for a stdio transport server.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub command: Option<String>,
    /// Arguments passed to the stdio `command`.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub args: Vec<String>,
    /// Environment variables set for the stdio `command`.
    #[serde(default, skip_serializing_if = "HashMap::is_empty")]
    pub env: HashMap<String, String>,
    /// Authentication mode used when executing tools from this scoped server.
    #[serde(default, skip_serializing_if = "McpServerAuthMode::is_none")]
    pub auth_mode: McpServerAuthMode,
    /// Protocol-era adoption policy for the MCP client (`auto` negotiates).
    #[serde(default, skip_serializing_if = "McpProtocolMode::is_auto")]
    pub protocol_mode: McpProtocolMode,
    /// Which elicitation modes this server may use (`url` by default).
    #[serde(default, skip_serializing_if = "McpElicitationPolicy::is_default")]
    pub elicitation_policy: McpElicitationPolicy,
    /// Provider id used to resolve a user-scoped bearer token.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub oauth_provider_id: Option<String>,
    /// Whether to discover tool definitions live from this server.
    #[serde(
        default = "default_scoped_tool_discovery",
        skip_serializing_if = "is_true"
    )]
    pub tool_discovery: bool,
    /// Organization catalog preset that supplies transport and authentication policy.
    #[serde(rename = "use", skip_serializing_if = "Option::is_none")]
    #[cfg_attr(feature = "openapi", schema(rename = "use"))]
    pub preset: Option<McpServerPresetRef>,
    /// Identity whose grant this attachment requests.
    #[serde(
        default,
        rename = "actsAs",
        alias = "acts_as",
        skip_serializing_if = "McpServerActsAs::is_none"
    )]
    #[cfg_attr(feature = "openapi", schema(rename = "actsAs"))]
    pub acts_as: McpServerActsAs,
    /// Whether a missing sign-in may pause the turn with an in-chat Connect
    /// card (`ask`, the default) or fails the call with a settings link
    /// (`never`).
    #[serde(
        default,
        rename = "connectInChat",
        alias = "connect_in_chat",
        skip_serializing_if = "McpConnectInChat::is_ask"
    )]
    #[cfg_attr(feature = "openapi", schema(rename = "connectInChat"))]
    pub connect_in_chat: McpConnectInChat,
    /// Whether the server's tools are listed only when the model asks for
    /// them through tool search (`false`, the default, lists them at turn
    /// start).
    #[serde(default, skip_serializing_if = "is_false")]
    pub deferred: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
struct ScopedMcpServerWire {
    #[serde(
        rename = "type",
        alias = "transport_type",
        skip_serializing_if = "Option::is_none"
    )]
    transport_type: Option<McpServerTransportType>,
    #[serde(default, skip_serializing_if = "String::is_empty")]
    url: String,
    #[serde(default, skip_serializing_if = "HashMap::is_empty")]
    headers: HashMap<String, String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    command: Option<String>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    args: Vec<String>,
    #[serde(default, skip_serializing_if = "HashMap::is_empty")]
    env: HashMap<String, String>,
    #[serde(default, skip_serializing_if = "McpServerAuthMode::is_none")]
    auth_mode: McpServerAuthMode,
    #[serde(default, skip_serializing_if = "McpProtocolMode::is_auto")]
    protocol_mode: McpProtocolMode,
    #[serde(default, skip_serializing_if = "McpElicitationPolicy::is_default")]
    elicitation_policy: McpElicitationPolicy,
    #[serde(skip_serializing_if = "Option::is_none")]
    oauth_provider_id: Option<String>,
    #[serde(
        default = "default_scoped_tool_discovery",
        skip_serializing_if = "is_true"
    )]
    tool_discovery: bool,
    #[serde(rename = "use", skip_serializing_if = "Option::is_none")]
    preset: Option<McpServerPresetRef>,
    #[serde(
        default,
        rename = "actsAs",
        alias = "acts_as",
        skip_serializing_if = "McpServerActsAs::is_none"
    )]
    acts_as: McpServerActsAs,
    #[serde(
        default,
        rename = "connectInChat",
        alias = "connect_in_chat",
        skip_serializing_if = "McpConnectInChat::is_ask"
    )]
    connect_in_chat: McpConnectInChat,
    #[serde(default, skip_serializing_if = "is_false")]
    deferred: bool,
}

impl TryFrom<ScopedMcpServerWire> for ScopedMcpServer {
    type Error = String;

    fn try_from(wire: ScopedMcpServerWire) -> Result<Self, Self::Error> {
        if wire.preset.is_some() && wire.transport_type.is_some() {
            return Err(
                "MCP server preset reference cannot be combined with inline field 'type'"
                    .to_string(),
            );
        }
        Ok(Self {
            transport_type: wire
                .transport_type
                .unwrap_or_else(default_scoped_transport_type),
            url: wire.url,
            headers: wire.headers,
            command: wire.command,
            args: wire.args,
            env: wire.env,
            auth_mode: wire.auth_mode,
            protocol_mode: wire.protocol_mode,
            elicitation_policy: wire.elicitation_policy,
            oauth_provider_id: wire.oauth_provider_id,
            tool_discovery: wire.tool_discovery,
            preset: wire.preset,
            acts_as: wire.acts_as,
            connect_in_chat: wire.connect_in_chat,
            deferred: wire.deferred,
        })
    }
}

impl From<ScopedMcpServer> for ScopedMcpServerWire {
    fn from(server: ScopedMcpServer) -> Self {
        Self {
            transport_type: server.preset.is_none().then_some(server.transport_type),
            url: server.url,
            headers: server.headers,
            command: server.command,
            args: server.args,
            env: server.env,
            auth_mode: server.auth_mode,
            protocol_mode: server.protocol_mode,
            elicitation_policy: server.elicitation_policy,
            oauth_provider_id: server.oauth_provider_id,
            tool_discovery: server.tool_discovery,
            preset: server.preset,
            acts_as: server.acts_as,
            connect_in_chat: server.connect_in_chat,
            deferred: server.deferred,
        }
    }
}

impl Default for ScopedMcpServer {
    fn default() -> Self {
        Self {
            transport_type: McpServerTransportType::Http,
            url: String::new(),
            headers: HashMap::new(),
            auth_mode: McpServerAuthMode::None,
            protocol_mode: McpProtocolMode::Auto,
            elicitation_policy: McpElicitationPolicy::Url,
            oauth_provider_id: None,
            tool_discovery: true,
            command: None,
            args: Vec::new(),
            env: HashMap::new(),
            preset: None,
            acts_as: McpServerActsAs::None,
            connect_in_chat: McpConnectInChat::Ask,
            deferred: false,
        }
    }
}

/// Id of the `user_mcp` capability, which owns the manage tools and
/// `connect_mcp_server`.
pub const USER_MCP_CAPABILITY_ID: &str = "user_mcp";

/// `user_mcp` setting a host derives (never authored) when the agent has an
/// MCP server acting as `user` or `user_or_service`: it gives the agent
/// `connect_mcp_server` alone, so it can offer the Connect card before a call
/// fails (user MCP servers D5).
pub const USER_MCP_CONNECT_SETTING: &str = "connect";

pub type ScopedMcpServers = BTreeMap<String, ScopedMcpServer>;
#[derive(Debug, Clone)]
pub struct McpSecretBindingMetadata {
    pub server_name: String,
    pub tool_name: String,
    pub parameter_name: String,
    pub configured: bool,
    pub setup_url: String,
}

/// Hide server-injected credential parameters from the model-visible schema.
/// The original call arguments are persisted before the MCP executor injects
/// plaintext, so this rewrite and the executor's override rejection form the
/// no-model-plaintext boundary.
pub fn apply_mcp_secret_binding_schemas(
    definitions: &mut [crate::runtime::ToolDefinition],
    bindings: &[McpSecretBindingMetadata],
) {
    for binding in bindings {
        if !is_valid_mcp_server_name(&binding.server_name) {
            continue;
        }
        let tool_name = crate::runtime::mcp_tool_name(&binding.server_name, &binding.tool_name);
        let Some(crate::runtime::ToolDefinition::Builtin(definition)) = definitions
            .iter_mut()
            .find(|definition| definition.name() == tool_name)
        else {
            continue;
        };
        remove_bound_parameter(&mut definition.parameters, &binding.parameter_name);
        if let Some(full) = definition.full_parameters.as_mut() {
            remove_bound_parameter(full, &binding.parameter_name);
        }
        let status = if binding.configured {
            "configured"
        } else {
            "setup required"
        };
        definition.description.push_str(&format!(
            "\n\nCredential '{}' is securely bound ({status}); do not request or supply it. Setup: {}",
            binding.parameter_name, binding.setup_url
        ));
    }
}

fn remove_bound_parameter(schema: &mut Value, parameter_name: &str) {
    let Some(object) = schema.as_object_mut() else {
        return;
    };
    if let Some(properties) = object.get_mut("properties").and_then(Value::as_object_mut) {
        properties.remove(parameter_name);
    }
    if let Some(required) = object.get_mut("required").and_then(Value::as_array_mut) {
        required.retain(|value| value.as_str() != Some(parameter_name));
    }
}

fn default_scoped_transport_type() -> McpServerTransportType {
    McpServerTransportType::Http
}

fn default_scoped_tool_discovery() -> bool {
    true
}

fn is_true(value: &bool) -> bool {
    *value
}

fn is_false(value: &bool) -> bool {
    !*value
}

pub fn scoped_mcp_servers_is_empty(servers: &ScopedMcpServers) -> bool {
    servers.is_empty()
}

/// Merge scoped MCP servers by logical server name. Later layers override earlier ones.
pub fn merge_scoped_mcp_servers(
    base: &ScopedMcpServers,
    overlay: &ScopedMcpServers,
) -> ScopedMcpServers {
    let mut merged = base.clone();
    merged.extend(overlay.clone());
    merged
}

// ============================================================================
// MCP Tool Types (following MCP specification)
// ============================================================================

/// MCP Tool definition as returned by tools/list.
/// Follows the MCP specification for tool discovery.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[cfg_attr(feature = "openapi", derive(ToSchema))]
pub struct McpToolDefinition {
    /// Unique name of the tool within the MCP server.
    pub name: String,
    /// Human-readable tool name supplied by the MCP server.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub title: Option<String>,
    /// Human-readable description of what the tool does.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub description: Option<String>,
    /// JSON Schema describing the tool's input parameters.
    #[serde(rename = "inputSchema")]
    pub input_schema: Value,
    /// MCP tool annotations (behavioral hints).
    /// See: <https://spec.modelcontextprotocol.io>
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub annotations: Option<McpToolAnnotations>,
}

/// MCP tool annotations as defined by the MCP specification.
/// All fields are optional booleans following the MCP convention.
#[derive(Debug, Clone, Serialize, Deserialize, Default)]
#[cfg_attr(feature = "openapi", derive(ToSchema))]
pub struct McpToolAnnotations {
    #[serde(
        default,
        skip_serializing_if = "Option::is_none",
        rename = "readOnlyHint"
    )]
    pub read_only_hint: Option<bool>,
    #[serde(
        default,
        skip_serializing_if = "Option::is_none",
        rename = "destructiveHint"
    )]
    pub destructive_hint: Option<bool>,
    #[serde(
        default,
        skip_serializing_if = "Option::is_none",
        rename = "idempotentHint"
    )]
    pub idempotent_hint: Option<bool>,
    #[serde(
        default,
        skip_serializing_if = "Option::is_none",
        rename = "openWorldHint"
    )]
    pub open_world_hint: Option<bool>,
}

/// A person's saved risk label for one MCP tool.
///
/// Decision (2026-10-10): a person's label always wins over the tool's own
/// annotations, because a remote server describes itself and a person who
/// owns the integration knows better. `read_only` also clears `open_world`,
/// since tool approval treats an outward-reaching tool like a destructive one;
/// without that a read-only MCP tool would still ask every time.
#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq, Hash)]
#[cfg_attr(feature = "openapi", derive(ToSchema))]
#[serde(rename_all = "snake_case")]
pub enum McpToolLabel {
    /// Only reads: never asks for approval in the normal approval mode.
    ReadOnly,
    /// Changes something: always asks for approval in the normal approval mode.
    Changes,
}

impl McpToolLabel {
    /// Stored and wire form.
    pub fn as_str(self) -> &'static str {
        match self {
            Self::ReadOnly => "read_only",
            Self::Changes => "changes",
        }
    }

    /// Parse the stored form; unknown values are `None`.
    pub fn parse(value: &str) -> Option<Self> {
        match value {
            "read_only" => Some(Self::ReadOnly),
            "changes" => Some(Self::Changes),
            _ => None,
        }
    }

    /// Override the hints a tool declared about itself.
    pub fn apply(self, hints: &mut crate::tool_types::ToolHints) {
        match self {
            Self::ReadOnly => {
                hints.readonly = Some(true);
                hints.destructive = Some(false);
                hints.open_world = Some(false);
            }
            Self::Changes => {
                hints.readonly = Some(false);
                hints.destructive = Some(true);
            }
        }
    }
}

/// Request for MCP tools/list endpoint.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct McpToolsListRequest {
    pub jsonrpc: String,
    pub id: i64,
    pub method: String,
}

impl Default for McpToolsListRequest {
    fn default() -> Self {
        Self {
            jsonrpc: "2.0".to_string(),
            id: 1,
            method: "tools/list".to_string(),
        }
    }
}

/// Response from MCP tools/list endpoint.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct McpToolsListResponse {
    pub jsonrpc: String,
    pub id: i64,
    #[serde(default)]
    pub result: Option<McpToolsListResult>,
    #[serde(default)]
    pub error: Option<McpError>,
}

/// Result of tools/list containing the list of tools.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct McpToolsListResult {
    pub tools: Vec<McpToolDefinition>,
    #[serde(rename = "nextCursor", skip_serializing_if = "Option::is_none")]
    pub next_cursor: Option<String>,
}

/// MCP error response.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct McpError {
    pub code: i64,
    pub message: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub data: Option<Value>,
}

/// Request for MCP tools/call endpoint.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct McpToolCallRequest {
    pub jsonrpc: String,
    pub id: i64,
    pub method: String,
    pub params: McpToolCallParams,
}

/// Parameters for tools/call request.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct McpToolCallParams {
    pub name: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub arguments: Option<Value>,
}

impl McpToolCallRequest {
    pub fn new(id: i64, name: String, arguments: Option<Value>) -> Self {
        Self {
            jsonrpc: "2.0".to_string(),
            id,
            method: "tools/call".to_string(),
            params: McpToolCallParams { name, arguments },
        }
    }
}

/// Response from MCP tools/call endpoint.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct McpToolCallResponse {
    pub jsonrpc: String,
    pub id: i64,
    #[serde(default)]
    pub result: Option<McpToolCallResult>,
    #[serde(default)]
    pub error: Option<McpError>,
}

/// Result of tools/call containing content.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct McpToolCallResult {
    pub content: Vec<McpContent>,
    #[serde(rename = "isError", default)]
    pub is_error: bool,
}

/// MCP content type (text, image, etc.).
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "type")]
pub enum McpContent {
    #[serde(rename = "text")]
    Text { text: String },
    #[serde(rename = "image")]
    Image { data: String, mime_type: String },
    #[serde(rename = "resource")]
    Resource {
        uri: String,
        mime_type: Option<String>,
        text: Option<String>,
    },
}

/// Helper to generate prefixed tool name for MCP tools.
/// Format: mcp_{server_name}__{tool_name} (double underscore separator)
/// The double underscore allows unambiguous parsing when server names contain underscores.
pub fn mcp_tool_name(server_name: &str, tool_name: &str) -> String {
    format!(
        "mcp_{}__{}",
        sanitize_mcp_server_name(server_name),
        tool_name
    )
}

/// Sanitize an MCP server name into a stable tool-name prefix.
pub fn sanitize_mcp_server_name(server_name: &str) -> String {
    server_name
        .to_lowercase()
        .chars()
        .map(|c| if c.is_alphanumeric() { c } else { '_' })
        .collect::<String>()
}

/// Whether a server name produces an unambiguous MCP tool prefix.
///
/// Repeated underscores collide with the separator inside a prefix; a trailing
/// underscore collides with it at the server/tool boundary. A single leading
/// underscore and underscores inside tool names remain valid.
pub fn is_valid_mcp_server_name(server_name: &str) -> bool {
    let prefix = sanitize_mcp_server_name(server_name);
    !prefix.is_empty() && !prefix.contains("__") && !prefix.ends_with('_')
}

/// Check if a tool name is an MCP tool (starts with "mcp_").
pub fn is_mcp_tool(tool_name: &str) -> bool {
    tool_name.starts_with("mcp_")
}

/// Parse MCP tool name to extract server name prefix and original tool name.
/// Returns (server_name_prefix, original_tool_name) if valid MCP tool.
/// Expected format: mcp_{server_name}__{tool_name} (double underscore separator)
pub fn parse_mcp_tool_name(tool_name: &str) -> Option<(String, String)> {
    if !tool_name.starts_with("mcp_") {
        return None;
    }
    let rest = &tool_name[4..]; // Skip "mcp_"
    // Find the double underscore separator between server name and tool name
    if let Some(pos) = rest.find("__") {
        let server_prefix = rest[..pos].to_string();
        let original_name = rest[pos + 2..].to_string(); // Skip "__"
        if !server_prefix.is_empty() && !original_name.is_empty() {
            return Some((server_prefix, original_name));
        }
    }
    None
}

/// Stable connection-provider id for an OAuth-enabled MCP server.
pub fn mcp_oauth_provider_id_for_uuid(server_id: uuid::Uuid) -> String {
    format!("mcp_oauth_{}", server_id)
}

/// Secret name for a session-scoped MCP OAuth token field.
pub fn mcp_oauth_session_secret_name(server_id: uuid::Uuid, field: &str) -> String {
    format!("mcp_oauth:{}:{}", server_id, field)
}

// ============================================================================
// Structured execute errors (EVE-492)
// ============================================================================

/// Closed vocabulary of error codes for Everruns' own MCP `tools/call`
/// execute path. Surfaces in [`McpExecuteError::code`] so LLM toolcallers
/// can branch on a machine-readable value instead of regexing prose.
///
/// New variants are a spec change. SDKs should treat any value they don't
/// recognise as `unknown` (forward-compat) — serde's `#[serde(other)]`
/// catch-all enables that on the deserialize side.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "openapi", derive(ToSchema))]
#[serde(rename_all = "snake_case")]
pub enum McpErrorCode {
    /// Tool name doesn't match any registered tool.
    ToolNotFound,
    /// Tool timed out (server-imposed budget exceeded).
    ToolTimeout,
    /// Tool panicked or hit an unrecoverable internal error.
    ToolPanicked,
    /// Required argument missing or argument failed validation.
    InvalidArguments,
    /// Caller is authenticated but not authorized for the requested action
    /// or org scope.
    PermissionDenied,
    /// Org/user quota or rate limit hit.
    QuotaExceeded,
    /// Outbound network call blocked by egress policy.
    NetworkBlocked,
    /// Upstream MCP server unreachable or returned an error we couldn't
    /// classify.
    McpServerUnreachable,
    /// Catch-all for unclassified internal failures. Treat as transient
    /// only if `retryable` is also true.
    Internal,
    /// Forward-compat sentinel — SDKs see this when the server returns a
    /// code they don't know yet.
    #[serde(other)]
    Unknown,
}

impl McpErrorCode {
    /// Stable wire string for this variant. Mirrors what `serde` emits so
    /// non-Rust SDKs and tests can match on the same value.
    pub fn as_str(&self) -> &'static str {
        match self {
            McpErrorCode::ToolNotFound => "tool_not_found",
            McpErrorCode::ToolTimeout => "tool_timeout",
            McpErrorCode::ToolPanicked => "tool_panicked",
            McpErrorCode::InvalidArguments => "invalid_arguments",
            McpErrorCode::PermissionDenied => "permission_denied",
            McpErrorCode::QuotaExceeded => "quota_exceeded",
            McpErrorCode::NetworkBlocked => "network_blocked",
            McpErrorCode::McpServerUnreachable => "mcp_server_unreachable",
            McpErrorCode::Internal => "internal",
            McpErrorCode::Unknown => "unknown",
        }
    }

    /// Default category for this code. Callers may override per-occurrence
    /// when context narrows the classification (e.g. an `Internal` with a
    /// known-transient root cause).
    pub fn default_category(&self) -> McpErrorCategory {
        match self {
            McpErrorCode::ToolTimeout
            | McpErrorCode::McpServerUnreachable
            | McpErrorCode::QuotaExceeded => McpErrorCategory::Transient,
            McpErrorCode::InvalidArguments => McpErrorCategory::Validation,
            McpErrorCode::PermissionDenied => McpErrorCategory::Auth,
            McpErrorCode::ToolNotFound
            | McpErrorCode::ToolPanicked
            | McpErrorCode::NetworkBlocked => McpErrorCategory::Permanent,
            McpErrorCode::Internal | McpErrorCode::Unknown => McpErrorCategory::Permanent,
        }
    }

    /// Default retryability for this code. Same override caveat as
    /// `default_category`.
    pub fn default_retryable(&self) -> bool {
        matches!(
            self,
            McpErrorCode::ToolTimeout
                | McpErrorCode::McpServerUnreachable
                | McpErrorCode::QuotaExceeded
        )
    }
}

/// Broad-strokes routing hint sitting alongside the precise [`McpErrorCode`].
/// The categories are stable enough that an LLM can pick a recovery
/// strategy from this field alone (e.g. retry transients with backoff,
/// surface validation errors to the user, escalate auth failures).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "openapi", derive(ToSchema))]
#[serde(rename_all = "snake_case")]
pub enum McpErrorCategory {
    /// Worth retrying — same call, possibly after `retry_after_seconds`.
    Transient,
    /// Repeating the same call will fail the same way.
    Permanent,
    /// Caller-side problem (bad arguments, schema mismatch).
    Validation,
    /// Authentication/authorization issue.
    Auth,
    /// Forward-compat sentinel.
    #[serde(other)]
    Unknown,
}

/// Typed structured-error envelope returned by Everruns' MCP `tools/call`
/// execute path. Serialized into the MCP `structuredContent` field on
/// error responses so the legacy `content[0].text` channel stays
/// backward-compatible; new SDKs prefer the typed envelope.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[cfg_attr(feature = "openapi", derive(ToSchema))]
pub struct McpExecuteError {
    /// Machine-readable error code. Closed vocabulary; SDKs that see an
    /// unrecognised value should map it to `unknown`.
    pub code: McpErrorCode,
    /// Human-readable error message. Mirrors the legacy
    /// `content[0].text` string for backward compat.
    pub message: String,
    /// Broad-strokes recovery category.
    pub category: McpErrorCategory,
    /// `true` when the same call is worth retrying. Distinct from
    /// `category == "transient"` because a server may know about a
    /// non-transient retry path (e.g. a transient `Internal`).
    pub retryable: bool,
    /// Seconds the caller should wait before retrying. Set on
    /// `tool_timeout`, `quota_exceeded`, and upstream-unreachable cases
    /// when the server has a concrete back-off hint.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub retry_after_seconds: Option<u32>,
    /// Short, agent-readable recovery hint. Free-form; one or two sentences.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub hint: Option<String>,
    /// Chain of upstream error messages, oldest cause first. Useful for
    /// debugging; SDKs should not treat this as machine-readable.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub cause_chain: Vec<String>,
}

impl McpExecuteError {
    /// Construct an error using the code's default category and
    /// retryability. Callers can chain `.with_*` to override.
    pub fn new(code: McpErrorCode, message: impl Into<String>) -> Self {
        Self {
            category: code.default_category(),
            retryable: code.default_retryable(),
            code,
            message: message.into(),
            retry_after_seconds: None,
            hint: None,
            cause_chain: Vec::new(),
        }
    }

    pub fn with_category(mut self, category: McpErrorCategory) -> Self {
        self.category = category;
        self
    }

    pub fn with_retryable(mut self, retryable: bool) -> Self {
        self.retryable = retryable;
        self
    }

    pub fn with_retry_after_seconds(mut self, seconds: u32) -> Self {
        self.retry_after_seconds = Some(seconds);
        self
    }

    pub fn with_hint(mut self, hint: impl Into<String>) -> Self {
        self.hint = Some(hint.into());
        self
    }

    pub fn with_cause(mut self, cause: impl Into<String>) -> Self {
        self.cause_chain.push(cause.into());
        self
    }
}

/// Classify a free-form error string raised by an internal MCP tool
/// implementation into a structured envelope. The implementations
/// currently return `Result<String, String>`; this is the boundary
/// where we recover the structure from prose. Pattern matches are
/// intentionally narrow (substrings, not regexes) so the classifier
/// fails open to `Internal` rather than mis-categorising.
///
/// **Convention for new error messages**: prefer constructing the
/// `McpExecuteError` directly (via a future `McpExecuteError`-typed
/// `Result`) instead of relying on this classifier. The classifier
/// exists to give the legacy `String` error path structure without
/// rewriting every tool first.
pub fn classify_mcp_execute_error(message: &str) -> McpExecuteError {
    let lower = message.to_ascii_lowercase();
    // Catalog-backed query/execute tools format their dispatch errors as
    // `<kind>: <message>` (see `crates/server/src/api/mcp_endpoint/catalog.rs::format_dispatch_error`
    // and the public contract in `knowledge/foundations/domains.md`). Map those prefixes
    // first so the most common real-world MCP failures get a precise code
    // rather than landing in the `Internal` catch-all.
    let code = if lower.starts_with("bad_request:") || lower.starts_with("unprocessable:") {
        McpErrorCode::InvalidArguments
    } else if lower.starts_with("not_found:") {
        McpErrorCode::ToolNotFound
    } else if lower.starts_with("conflict:") {
        // No dedicated `conflict` code today; surface as a validation
        // failure since the caller's input is the proximate cause and
        // a retry without changes won't succeed.
        McpErrorCode::InvalidArguments
    } else if lower.starts_with("forbidden:") {
        McpErrorCode::PermissionDenied
    } else if lower.starts_with("internal:") {
        McpErrorCode::Internal
    // Order matters: more specific patterns first.
    } else if lower.contains("timed out") || lower.contains("timeout") {
        McpErrorCode::ToolTimeout
    } else if lower.starts_with("unknown tool") {
        McpErrorCode::ToolNotFound
    } else if lower.starts_with("missing required parameter") || lower.contains("invalid argument")
    {
        McpErrorCode::InvalidArguments
    } else if lower.contains("permission denied")
        || lower.contains("forbidden")
        || lower.contains("not authorized")
        || lower.contains("unauthorized")
    {
        McpErrorCode::PermissionDenied
    } else if lower.contains("quota") || lower.contains("rate limit") {
        McpErrorCode::QuotaExceeded
    } else if lower.contains("network blocked") || lower.contains("egress") {
        McpErrorCode::NetworkBlocked
    } else if lower.contains("mcp server") && lower.contains("unreachable") {
        McpErrorCode::McpServerUnreachable
    } else if lower.contains("panicked") {
        McpErrorCode::ToolPanicked
    } else {
        McpErrorCode::Internal
    };
    McpExecuteError::new(code, message)
}

#[cfg(test)]
#[path = "mcp_server_tests.rs"]
mod tests;
