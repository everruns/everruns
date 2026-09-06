// MCP Server domain types
//
// Spec: knowledge/integrations/mcp.md (umbrella), knowledge/integrations/mcp-servers.md (detail)
//
// These types represent the MCP (Model Context Protocol) server configuration.
// Used by both API and worker crates.
//
// Currently supports only HTTP (Streamable HTTP) transport.
// MCP tool types follow the MCP specification for tool discovery and execution.

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::collections::{BTreeMap, HashMap};

use crate::typed_id::McpServerId;

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
    /// rejects it during scoped-config validation (see knowledge/integrations/runtime-mcp.md).
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

// ============================================================================
// MCP protocol versions and per-server adoption policy
// ============================================================================
//
// Everruns' MCP *client* speaks three protocol eras. They differ in how the
// connection is established and what metadata travels with each request:
//
// - `2025-03-26` / `2025-06-18`: *stateful*. The client must run the
//   `initialize` handshake, may receive an `Mcp-Session-Id` it has to echo on
//   every subsequent request, and sends `notifications/initialized`.
// - `2026-07-28`: *stateless*. No handshake and no session id; protocol version
//   + client info ride in `_meta` on every request, and routable headers
//   (`MCP-Protocol-Version`, `Mcp-Method`, `Mcp-Name`) let edge infrastructure
//   route without parsing the body.
//
// Eras are named by their version date, not by a moving label like "stable" or
// "rc" — `2026-07-28` shipped as a final spec on 2026-07-28, and the previous
// naming outlived its meaning within one release.
//
// See knowledge/integrations/mcp-servers.md (Multi-era protocol support) and the negotiation
// engine in `everruns-mcp` (`protocol.rs`).

/// MCP `2025-03-26` (stateful handshake). Oldest era the client speaks.
pub const MCP_PROTOCOL_VERSION_2025_03: &str = "2025-03-26";
/// MCP `2025-06-18` (stateful handshake).
pub const MCP_PROTOCOL_VERSION_2025_06: &str = "2025-06-18";
/// MCP `2026-07-28` (stateless). Current era.
pub const MCP_PROTOCOL_VERSION_2026_07: &str = "2026-07-28";

/// Per-server policy for which MCP protocol era the client uses.
///
/// `Auto` (the default) probes the server and adapts — it tries the stateless
/// `2026-07-28` path first and transparently falls back to the stateful
/// handshake when a server demands it, so a single configuration speaks to
/// every era without operator action. The pinned variants skip negotiation when
/// an operator knows a server's era (or to work around a server that
/// mis-signals it).
///
/// Wire values are the version dates. The pre-release names (`legacy`,
/// `stable`, `rc`) stay accepted as deserialization aliases so stored config
/// keeps loading, but they are no longer emitted.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
#[cfg_attr(feature = "openapi", derive(ToSchema))]
#[cfg_attr(feature = "openapi", schema(example = "auto"))]
#[serde(rename_all = "snake_case")]
pub enum McpProtocolMode {
    /// Probe once, detect the server's era, adapt, and cache the verdict.
    #[default]
    Auto,
    /// Pin to `2025-03-26` stateful behavior (handshake + session id).
    #[serde(rename = "2025-03-26", alias = "legacy")]
    V2025March,
    /// Pin to `2025-06-18` stateful behavior (handshake + session id).
    #[serde(rename = "2025-06-18", alias = "stable")]
    V2025June,
    /// Pin to `2026-07-28` stateless behavior (`_meta` per request, routable
    /// headers, no handshake).
    #[serde(rename = "2026-07-28", alias = "rc")]
    V2026July,
}

impl McpProtocolMode {
    /// Whether this is the default `Auto` policy. Used to keep the field out of
    /// serialized config when it carries no information.
    pub fn is_auto(&self) -> bool {
        matches!(self, McpProtocolMode::Auto)
    }

    /// The protocol version string a *pinned* mode advertises. `Auto` returns
    /// `None` because its version is decided by negotiation at runtime.
    pub fn pinned_version(&self) -> Option<&'static str> {
        match self {
            McpProtocolMode::Auto => None,
            McpProtocolMode::V2025March => Some(MCP_PROTOCOL_VERSION_2025_03),
            McpProtocolMode::V2025June => Some(MCP_PROTOCOL_VERSION_2025_06),
            McpProtocolMode::V2026July => Some(MCP_PROTOCOL_VERSION_2026_07),
        }
    }

    /// Whether a pinned mode requires the stateful `initialize` handshake.
    /// `Auto` returns `None` (decided by negotiation).
    pub fn pinned_stateful(&self) -> Option<bool> {
        match self {
            McpProtocolMode::Auto => None,
            McpProtocolMode::V2025March | McpProtocolMode::V2025June => Some(true),
            McpProtocolMode::V2026July => Some(false),
        }
    }
}

impl std::fmt::Display for McpProtocolMode {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            McpProtocolMode::Auto => write!(f, "auto"),
            McpProtocolMode::V2025March => write!(f, "{MCP_PROTOCOL_VERSION_2025_03}"),
            McpProtocolMode::V2025June => write!(f, "{MCP_PROTOCOL_VERSION_2025_06}"),
            McpProtocolMode::V2026July => write!(f, "{MCP_PROTOCOL_VERSION_2026_07}"),
        }
    }
}

impl From<&str> for McpProtocolMode {
    /// Parses the canonical version-date values and the pre-release aliases
    /// (`legacy`/`stable`/`rc`) that stored config and older workers still send.
    /// Anything unrecognized falls back to `Auto`, which negotiates anyway.
    fn from(s: &str) -> Self {
        match s {
            MCP_PROTOCOL_VERSION_2025_03 | "legacy" => McpProtocolMode::V2025March,
            MCP_PROTOCOL_VERSION_2025_06 | "stable" => McpProtocolMode::V2025June,
            MCP_PROTOCOL_VERSION_2026_07 | "rc" => McpProtocolMode::V2026July,
            _ => McpProtocolMode::Auto,
        }
    }
}

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

/// MCP Server lifecycle status.
/// - `active`: Server is available for use
/// - `disabled`: Server is disabled and not used
/// - `archived`: Server is hidden from listings and cannot be modified or assigned
/// - `deleted`: Server is a tombstone kept only for historical references
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[cfg_attr(feature = "openapi", derive(ToSchema))]
#[cfg_attr(feature = "openapi", schema(example = "active"))]
#[serde(rename_all = "lowercase")]
pub enum McpServerStatus {
    /// Server is available for use.
    Active,
    /// Server is disabled and not used.
    Disabled,
    /// Server is hidden from listings and cannot be modified or assigned.
    Archived,
    /// Server is deleted and should only survive as a tombstone for references.
    Deleted,
}

impl std::fmt::Display for McpServerStatus {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            McpServerStatus::Active => write!(f, "active"),
            McpServerStatus::Disabled => write!(f, "disabled"),
            McpServerStatus::Archived => write!(f, "archived"),
            McpServerStatus::Deleted => write!(f, "deleted"),
        }
    }
}

impl From<&str> for McpServerStatus {
    fn from(s: &str) -> Self {
        match s {
            "disabled" => McpServerStatus::Disabled,
            "archived" => McpServerStatus::Archived,
            "deleted" => McpServerStatus::Deleted,
            _ => McpServerStatus::Active,
        }
    }
}

/// MCP Server configuration.
/// Represents a remote MCP server that can provide tools and resources.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[cfg_attr(feature = "openapi", derive(ToSchema))]
pub struct McpServer {
    /// Unique identifier for the MCP server.
    #[cfg_attr(feature = "openapi", schema(value_type = String, example = "mcp_01933b5a00007000800000000000001"))]
    pub id: McpServerId,
    /// Display name of the MCP server.
    #[cfg_attr(feature = "openapi", schema(example = "atlassian-mcp-server"))]
    pub name: String,
    /// Human-readable description of the MCP server.
    #[serde(skip_serializing_if = "Option::is_none")]
    #[cfg_attr(
        feature = "openapi",
        schema(example = "Atlassian MCP Server for Jira and Confluence")
    )]
    pub description: Option<String>,
    /// URL of the MCP server endpoint.
    #[cfg_attr(
        feature = "openapi",
        schema(example = "https://mcp.atlassian.com/v1/mcp")
    )]
    pub url: String,
    /// Transport type (currently only HTTP supported).
    pub transport_type: McpServerTransportType,
    /// Current lifecycle status of the MCP server.
    pub status: McpServerStatus,
    /// Authentication mode for this MCP server.
    #[serde(default)]
    pub auth_mode: McpServerAuthMode,
    /// Protocol-era adoption policy for the MCP client (`auto` negotiates).
    #[serde(default, skip_serializing_if = "McpProtocolMode::is_auto")]
    pub protocol_mode: McpProtocolMode,
    /// Stable provider id used for user-scoped OAuth connections.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub oauth_provider_id: Option<String>,
    /// Whether an API key has been configured.
    pub api_key_set: bool,
    /// Additional HTTP headers for authentication.
    /// Keys are header names, values are header values.
    #[serde(default, skip_serializing_if = "HashMap::is_empty")]
    pub headers: HashMap<String, String>,
    /// Timestamp when the MCP server was created.
    pub created_at: DateTime<Utc>,
    /// Timestamp when the MCP server was last updated.
    pub updated_at: DateTime<Utc>,
    /// Timestamp when the MCP server was archived.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub archived_at: Option<DateTime<Utc>>,
    /// Timestamp when the MCP server was deleted.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub deleted_at: Option<DateTime<Utc>>,
}

/// Session-, agent-, or harness-scoped remote MCP server configuration.
///
/// This intentionally mirrors the `mcpServers` object shape used by common MCP
/// client config files while staying within Everruns' current remote-HTTP-only
/// support.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[cfg_attr(feature = "openapi", derive(ToSchema))]
pub struct ScopedMcpServer {
    /// MCP transport type. Only remote HTTP is supported today.
    #[serde(
        default = "default_scoped_transport_type",
        rename = "type",
        alias = "transport_type"
    )]
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
    /// Provider id used to resolve a user-scoped bearer token.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub oauth_provider_id: Option<String>,
    /// Whether to discover tool definitions live from this server.
    #[serde(
        default = "default_scoped_tool_discovery",
        skip_serializing_if = "is_true"
    )]
    pub tool_discovery: bool,
}

impl Default for ScopedMcpServer {
    fn default() -> Self {
        Self {
            transport_type: McpServerTransportType::Http,
            url: String::new(),
            headers: HashMap::new(),
            auth_mode: McpServerAuthMode::None,
            protocol_mode: McpProtocolMode::Auto,
            oauth_provider_id: None,
            tool_discovery: true,
            command: None,
            args: Vec::new(),
            env: HashMap::new(),
        }
    }
}

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
    definitions: &mut [crate::ToolDefinition],
    bindings: &[McpSecretBindingMetadata],
) {
    for binding in bindings {
        let tool_name = crate::mcp_tool_name(&binding.server_name, &binding.tool_name);
        let Some(crate::ToolDefinition::Builtin(definition)) = definitions
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
///
/// See `knowledge/integrations/mcp.md` for the error contract.
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
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn bound_parameters_are_removed_from_both_schemas_of_only_the_matching_tool() {
        use crate::tool_types::{BuiltinTool, ToolDefinition};
        let schema = json!({"type":"object","properties":{"message":{"type":"string"},"channel_key":{"type":"string"}},"required":["message","channel_key"],"additionalProperties":false});
        let builtin = |name: &str| {
            ToolDefinition::Builtin(BuiltinTool {
                name: name.into(),
                display_name: None,
                description: "Send message".into(),
                parameters: schema.clone(),
                policy: Default::default(),
                category: None,
                deferrable: Default::default(),
                hints: Default::default(),
                full_parameters: Some(schema.clone()),
            })
        };
        let mut definitions = vec![
            builtin("mcp_notify__send"),
            builtin("mcp_other__send"),
            builtin("mcp_notify__read"),
        ];
        let unrelated = serde_json::to_value(&definitions[1..]).unwrap();
        for configured in [true, false] {
            definitions[0] = builtin("mcp_notify__send");
            apply_mcp_secret_binding_schemas(
                &mut definitions,
                &[
                    McpSecretBindingMetadata {
                        server_name: "missing".into(),
                        tool_name: "send".into(),
                        parameter_name: "message".into(),
                        configured: true,
                        setup_url: "/missing".into(),
                    },
                    McpSecretBindingMetadata {
                        server_name: "Notify".into(),
                        tool_name: "send".into(),
                        parameter_name: "channel_key".into(),
                        configured,
                        setup_url: "/agent/credentials".into(),
                    },
                ],
            );
            let ToolDefinition::Builtin(bound) = &definitions[0] else {
                panic!("expected builtin")
            };
            let expected = json!({"type":"object","properties":{"message":{"type":"string"}},"required":["message"],"additionalProperties":false});
            assert_eq!(bound.parameters, expected);
            assert_eq!(bound.full_parameters.as_ref(), Some(&expected));
            let status = if configured {
                "configured"
            } else {
                "setup required"
            };
            assert_eq!(
                bound.description,
                format!(
                    "Send message\n\nCredential 'channel_key' is securely bound ({status}); do not request or supply it. Setup: /agent/credentials"
                )
            );
            assert_eq!(serde_json::to_value(&definitions[1..]).unwrap(), unrelated);
        }
    }

    #[test]
    fn bound_parameter_removal_preserves_missing_or_nonobject_schema_parts() {
        for mut schema in [
            json!(null),
            json!([]),
            json!({}),
            json!({"properties":[],"required":"message"}),
            json!({"properties":{"message":{}},"required":["message",7]}),
        ] {
            let original = schema.clone();
            remove_bound_parameter(&mut schema, "channel_key");
            assert_eq!(schema, original);
        }
    }

    #[test]
    fn protocol_modes_accept_aliases_but_emit_canonical_versions_and_policies() {
        for (mode, canonical, alias, version, stateful) in [
            (McpProtocolMode::Auto, "auto", "auto", None, None),
            (
                McpProtocolMode::V2025March,
                "2025-03-26",
                "legacy",
                Some("2025-03-26"),
                Some(true),
            ),
            (
                McpProtocolMode::V2025June,
                "2025-06-18",
                "stable",
                Some("2025-06-18"),
                Some(true),
            ),
            (
                McpProtocolMode::V2026July,
                "2026-07-28",
                "rc",
                Some("2026-07-28"),
                Some(false),
            ),
        ] {
            assert_eq!(serde_json::to_value(mode).unwrap(), json!(canonical));
            assert_eq!(mode.to_string(), canonical);
            assert_eq!(mode.pinned_version(), version);
            assert_eq!(mode.pinned_stateful(), stateful);
            assert_eq!(mode.is_auto(), canonical == "auto");
            for input in [canonical, alias] {
                assert_eq!(McpProtocolMode::from(input), mode);
                assert_eq!(
                    serde_json::from_value::<McpProtocolMode>(json!(input)).unwrap(),
                    mode
                );
            }
        }
        assert_eq!(McpProtocolMode::from("nonsense"), McpProtocolMode::Auto);
        assert!(serde_json::from_value::<McpProtocolMode>(json!("nonsense")).is_err());
    }

    #[test]
    fn scoped_config_defaults_omit_optional_values_and_canonicalize_aliases() {
        for (input, expected_mode, expected_wire) in [
            (
                json!({"url":"https://example.com/mcp"}),
                McpProtocolMode::Auto,
                json!({"type":"http","url":"https://example.com/mcp"}),
            ),
            (
                json!({"type":"http","url":"https://example.com/mcp","protocol_mode":"rc"}),
                McpProtocolMode::V2026July,
                json!({"type":"http","url":"https://example.com/mcp","protocol_mode":"2026-07-28"}),
            ),
            (
                json!({"transport_type":"http","url":"https://example.com/mcp","protocol_mode":"legacy"}),
                McpProtocolMode::V2025March,
                json!({"type":"http","url":"https://example.com/mcp","protocol_mode":"2025-03-26"}),
            ),
        ] {
            let config: ScopedMcpServer = serde_json::from_value(input).unwrap();
            assert_eq!(config.protocol_mode, expected_mode);
            assert!(config.tool_discovery);
            assert_eq!(serde_json::to_value(config).unwrap(), expected_wire);
        }
        let defaults = ScopedMcpServer::default();
        assert_eq!(defaults.protocol_mode, McpProtocolMode::Auto);
        assert_eq!(
            serde_json::to_value(defaults).unwrap(),
            json!({"type":"http"})
        );
    }

    #[test]
    fn scoped_config_preserves_nondefault_transport_auth_and_discovery_fields() {
        let wire = json!({"type":"stdio","command":"mcp-server","args":["--project","demo"],"env":{"MODE":"test"},"headers":{"X-Trace":"trace"},"auth_mode":"oauth","oauth_provider_id":"provider","protocol_mode":"2025-06-18","tool_discovery":false});
        let config: ScopedMcpServer = serde_json::from_value(wire.clone()).unwrap();
        assert!(config.transport_type.is_local());
        assert!(!config.tool_discovery);
        assert_eq!(config.auth_mode, McpServerAuthMode::OAuth);
        assert_eq!(serde_json::to_value(config).unwrap(), wire);
    }

    #[test]
    fn scoped_merge_replaces_entire_matching_connection_without_mutating_inputs() {
        let base: ScopedMcpServers = serde_json::from_value(json!({
            "base_only":{"url":"https://base.test/mcp"},
            "shared":{"url":"https://old.test/mcp","headers":{"old":"value"},"protocol_mode":"2025-03-26","tool_discovery":false}
        })).unwrap();
        let overlay: ScopedMcpServers = serde_json::from_value(json!({
            "overlay_only":{"url":"https://overlay.test/mcp"},
            "shared":{"url":"https://new.test/mcp","headers":{"new":"value"},"protocol_mode":"2026-07-28","auth_mode":"oauth","oauth_provider_id":"provider"}
        })).unwrap();
        let before_base = base.clone();
        let before_overlay = overlay.clone();
        let merged = merge_scoped_mcp_servers(&base, &overlay);
        let expected = BTreeMap::from([
            ("base_only".into(), base["base_only"].clone()),
            ("shared".into(), overlay["shared"].clone()),
            ("overlay_only".into(), overlay["overlay_only"].clone()),
        ]);
        assert_eq!(merged, expected);
        assert_eq!(base, before_base);
        assert_eq!(overlay, before_overlay);
        assert_eq!(merge_scoped_mcp_servers(&base, &BTreeMap::new()), base);
        assert_eq!(
            merge_scoped_mcp_servers(&BTreeMap::new(), &overlay),
            overlay
        );
    }

    #[test]
    fn normalize_mcp_error_code_maps_legacy_to_rc() {
        for (input, expected) in [
            (-32002, -32602),
            (-32602, -32602),
            (-32601, -32601),
            (0, 0),
            (i64::MIN, i64::MIN),
            (i64::MAX, i64::MAX),
        ] {
            assert_eq!(normalize_mcp_error_code(input), expected);
        }
    }

    #[test]
    fn tool_names_sanitize_server_names_and_preserve_tool_components() {
        for (server, tool, full, prefix) in [
            ("github", "search", "mcp_github__search", "github"),
            (
                "microsoft_learn",
                "docs_search",
                "mcp_microsoft_learn__docs_search",
                "microsoft_learn",
            ),
            (
                "microsoft-learn",
                "search",
                "mcp_microsoft_learn__search",
                "microsoft_learn",
            ),
            ("GitHub", "search", "mcp_github__search", "github"),
            (
                "my.server.name",
                "tool",
                "mcp_my_server_name__tool",
                "my_server_name",
            ),
            (
                "my_long_server_name",
                "my_complex_tool",
                "mcp_my_long_server_name__my_complex_tool",
                "my_long_server_name",
            ),
            ("github", "read__file", "mcp_github__read__file", "github"),
        ] {
            assert_eq!(mcp_tool_name(server, tool), full);
            assert_eq!(
                parse_mcp_tool_name(full),
                Some((prefix.into(), tool.into()))
            );
            assert!(is_mcp_tool(full));
        }
    }

    #[test]
    fn tool_name_parser_rejects_missing_prefix_separator_or_components() {
        for name in [
            "get_weather",
            "mcpsearch",
            "mcp_github_search",
            "mcp___search",
            "mcp_github__",
            "mcp_",
            "",
        ] {
            assert_eq!(parse_mcp_tool_name(name), None, "{name}");
        }
        assert!(!is_mcp_tool("get_weather"));
        assert!(!is_mcp_tool("mcpsearch"));
        // Routing classification intentionally accepts an invalid MCP-shaped name.
        assert!(is_mcp_tool("mcp_"));
    }

    #[test]
    fn error_codes_have_independent_wire_category_and_retry_contracts() {
        for (code, wire, category, retryable) in [
            (
                McpErrorCode::ToolNotFound,
                "tool_not_found",
                McpErrorCategory::Permanent,
                false,
            ),
            (
                McpErrorCode::ToolTimeout,
                "tool_timeout",
                McpErrorCategory::Transient,
                true,
            ),
            (
                McpErrorCode::ToolPanicked,
                "tool_panicked",
                McpErrorCategory::Permanent,
                false,
            ),
            (
                McpErrorCode::InvalidArguments,
                "invalid_arguments",
                McpErrorCategory::Validation,
                false,
            ),
            (
                McpErrorCode::PermissionDenied,
                "permission_denied",
                McpErrorCategory::Auth,
                false,
            ),
            (
                McpErrorCode::QuotaExceeded,
                "quota_exceeded",
                McpErrorCategory::Transient,
                true,
            ),
            (
                McpErrorCode::NetworkBlocked,
                "network_blocked",
                McpErrorCategory::Permanent,
                false,
            ),
            (
                McpErrorCode::McpServerUnreachable,
                "mcp_server_unreachable",
                McpErrorCategory::Transient,
                true,
            ),
            (
                McpErrorCode::Internal,
                "internal",
                McpErrorCategory::Permanent,
                false,
            ),
            (
                McpErrorCode::Unknown,
                "unknown",
                McpErrorCategory::Permanent,
                false,
            ),
        ] {
            assert_eq!(code.as_str(), wire);
            assert_eq!(serde_json::to_value(code).unwrap(), json!(wire));
            assert_eq!(
                serde_json::from_value::<McpErrorCode>(json!(wire)).unwrap(),
                code
            );
            let error = McpExecuteError::new(code, "original message");
            assert_eq!(error.category, category);
            assert_eq!(error.retryable, retryable);
            assert_eq!(error.message, "original message");
        }
    }

    #[test]
    fn unknown_error_code_and_category_deserialize_to_forward_compatible_sentinels() {
        assert_eq!(
            serde_json::from_value::<McpErrorCode>(json!("future_code")).unwrap(),
            McpErrorCode::Unknown
        );
        assert_eq!(
            serde_json::from_value::<McpErrorCategory>(json!("future_category")).unwrap(),
            McpErrorCategory::Unknown
        );
    }

    #[test]
    fn error_classifier_preserves_messages_and_routes_every_marker_and_prefix() {
        for (message, code, category, retryable) in [
            (
                "Tool timed out after 30000ms",
                McpErrorCode::ToolTimeout,
                McpErrorCategory::Transient,
                true,
            ),
            (
                "Command timed out after 5000ms",
                McpErrorCode::ToolTimeout,
                McpErrorCategory::Transient,
                true,
            ),
            (
                "TIMEOUT",
                McpErrorCode::ToolTimeout,
                McpErrorCategory::Transient,
                true,
            ),
            (
                "Unknown tool: github.foo",
                McpErrorCode::ToolNotFound,
                McpErrorCategory::Permanent,
                false,
            ),
            (
                "Missing required parameter: query",
                McpErrorCode::InvalidArguments,
                McpErrorCategory::Validation,
                false,
            ),
            (
                "invalid argument: query",
                McpErrorCode::InvalidArguments,
                McpErrorCategory::Validation,
                false,
            ),
            (
                "permission denied for org",
                McpErrorCode::PermissionDenied,
                McpErrorCategory::Auth,
                false,
            ),
            (
                "Forbidden: org scope not allowed",
                McpErrorCode::PermissionDenied,
                McpErrorCategory::Auth,
                false,
            ),
            (
                "not authorized to call this tool",
                McpErrorCode::PermissionDenied,
                McpErrorCategory::Auth,
                false,
            ),
            (
                "Unauthorized request",
                McpErrorCode::PermissionDenied,
                McpErrorCategory::Auth,
                false,
            ),
            (
                "Quota exceeded for org",
                McpErrorCode::QuotaExceeded,
                McpErrorCategory::Transient,
                true,
            ),
            (
                "Rate limit hit",
                McpErrorCode::QuotaExceeded,
                McpErrorCategory::Transient,
                true,
            ),
            (
                "network blocked",
                McpErrorCode::NetworkBlocked,
                McpErrorCategory::Permanent,
                false,
            ),
            (
                "EGRESS denied",
                McpErrorCode::NetworkBlocked,
                McpErrorCategory::Permanent,
                false,
            ),
            (
                "MCP server unreachable",
                McpErrorCode::McpServerUnreachable,
                McpErrorCategory::Transient,
                true,
            ),
            (
                "tool panicked",
                McpErrorCode::ToolPanicked,
                McpErrorCategory::Permanent,
                false,
            ),
            (
                "bad_request: name must be <=200 chars",
                McpErrorCode::InvalidArguments,
                McpErrorCategory::Validation,
                false,
            ),
            (
                "unprocessable: cycle detected in capability graph",
                McpErrorCode::InvalidArguments,
                McpErrorCategory::Validation,
                false,
            ),
            (
                "conflict: session is already paused",
                McpErrorCode::InvalidArguments,
                McpErrorCategory::Validation,
                false,
            ),
            (
                "not_found: agent agent_xyz not in this org",
                McpErrorCode::ToolNotFound,
                McpErrorCategory::Permanent,
                false,
            ),
            (
                "forbidden: principal lacks SESSION_WRITE",
                McpErrorCode::PermissionDenied,
                McpErrorCategory::Auth,
                false,
            ),
            (
                "internal: storage backend returned 503",
                McpErrorCode::Internal,
                McpErrorCategory::Permanent,
                false,
            ),
            (
                "INTERNAL: upstream timed out",
                McpErrorCode::Internal,
                McpErrorCategory::Permanent,
                false,
            ),
            (
                "bad_request: invalid timeout",
                McpErrorCode::InvalidArguments,
                McpErrorCategory::Validation,
                false,
            ),
            (
                "strange unanticipated message",
                McpErrorCode::Internal,
                McpErrorCategory::Permanent,
                false,
            ),
            (
                "unreachable",
                McpErrorCode::Internal,
                McpErrorCategory::Permanent,
                false,
            ),
            (
                "mcp server available",
                McpErrorCode::Internal,
                McpErrorCategory::Permanent,
                false,
            ),
            (
                "",
                McpErrorCode::Internal,
                McpErrorCategory::Permanent,
                false,
            ),
        ] {
            let error = classify_mcp_execute_error(message);
            assert_eq!(error.code, code, "{message}");
            assert_eq!(error.category, category, "{message}");
            assert_eq!(error.retryable, retryable, "{message}");
            assert_eq!(error.message, message);
        }
    }

    #[test]
    fn error_envelopes_omit_empty_optionals_and_preserve_explicit_overrides() {
        let minimal = McpExecuteError::new(McpErrorCode::ToolNotFound, "no such tool");
        assert_eq!(
            serde_json::to_value(minimal).unwrap(),
            json!({"code":"tool_not_found","message":"no such tool","category":"permanent","retryable":false})
        );
        let full = McpExecuteError::new(McpErrorCode::ToolTimeout, "tool timed out after 30000ms")
            .with_category(McpErrorCategory::Auth)
            .with_retryable(false)
            .with_retry_after_seconds(10)
            .with_hint("Reduce input size before retrying.")
            .with_cause("root cause")
            .with_cause("downstream: upstream gateway timeout");
        assert_eq!(
            serde_json::to_value(full).unwrap(),
            json!({"code":"tool_timeout","message":"tool timed out after 30000ms","category":"auth","retryable":false,"retry_after_seconds":10,"hint":"Reduce input size before retrying.","cause_chain":["root cause","downstream: upstream gateway timeout"]})
        );
    }
    #[test]
    fn auth_modes_use_canonical_wire_values_and_accept_legacy_oauth_spelling() {
        for (mode, wire) in [
            (McpServerAuthMode::None, "none"),
            (McpServerAuthMode::ApiKey, "api_key"),
            (McpServerAuthMode::OAuth, "oauth"),
        ] {
            assert_eq!(serde_json::to_value(&mode).unwrap(), json!(wire));
            assert_eq!(
                serde_json::from_value::<McpServerAuthMode>(json!(wire)).unwrap(),
                mode
            );
            assert_eq!(McpServerAuthMode::from(wire), mode);
            assert_eq!(mode.to_string(), wire);
        }
        let legacy: McpServerAuthMode = serde_json::from_value(json!("o_auth")).unwrap();
        assert_eq!(legacy, McpServerAuthMode::OAuth);
        assert_eq!(serde_json::to_value(legacy).unwrap(), json!("oauth"));
    }
}
