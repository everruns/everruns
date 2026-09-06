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
        if !is_valid_mcp_server_name(&binding.server_name) {
            continue;
        }
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

    #[test]
    fn bound_parameter_is_removed_from_model_schema() {
        let mut schema = serde_json::json!({
            "type": "object",
            "properties": {
                "message": { "type": "string" },
                "channel_key": { "type": "string" }
            },
            "required": ["message", "channel_key"]
        });

        remove_bound_parameter(&mut schema, "channel_key");

        assert!(schema["properties"].get("channel_key").is_none());
        assert_eq!(schema["required"], serde_json::json!(["message"]));
        assert_eq!(schema["properties"]["message"]["type"], "string");
    }

    #[test]
    fn protocol_mode_defaults_to_auto() {
        assert_eq!(McpProtocolMode::default(), McpProtocolMode::Auto);
        assert!(McpProtocolMode::default().is_auto());
    }

    #[test]
    fn protocol_mode_serde_round_trips_version_dates() {
        for (mode, json) in [
            (McpProtocolMode::Auto, "\"auto\""),
            (McpProtocolMode::V2025March, "\"2025-03-26\""),
            (McpProtocolMode::V2025June, "\"2025-06-18\""),
            (McpProtocolMode::V2026July, "\"2026-07-28\""),
        ] {
            assert_eq!(serde_json::to_string(&mode).unwrap(), json);
            let back: McpProtocolMode = serde_json::from_str(json).unwrap();
            assert_eq!(back, mode);
        }
    }

    #[test]
    fn protocol_mode_accepts_pre_release_aliases() {
        // Config stored before 2026-07-28 shipped still deserializes; the
        // aliases are read-only and never emitted again.
        for (json, expected) in [
            ("\"legacy\"", McpProtocolMode::V2025March),
            ("\"stable\"", McpProtocolMode::V2025June),
            ("\"rc\"", McpProtocolMode::V2026July),
        ] {
            let parsed: McpProtocolMode = serde_json::from_str(json).unwrap();
            assert_eq!(parsed, expected);
        }
        assert_eq!(McpProtocolMode::from("rc"), McpProtocolMode::V2026July);
        assert_eq!(
            McpProtocolMode::from("2026-07-28"),
            McpProtocolMode::V2026July
        );
        assert_eq!(McpProtocolMode::from("nonsense"), McpProtocolMode::Auto);
    }

    #[test]
    fn protocol_mode_pinned_version_and_statefulness() {
        assert_eq!(McpProtocolMode::Auto.pinned_version(), None);
        assert_eq!(McpProtocolMode::Auto.pinned_stateful(), None);
        assert_eq!(
            McpProtocolMode::V2025March.pinned_version(),
            Some(MCP_PROTOCOL_VERSION_2025_03)
        );
        assert_eq!(McpProtocolMode::V2025March.pinned_stateful(), Some(true));
        assert_eq!(
            McpProtocolMode::V2025June.pinned_version(),
            Some(MCP_PROTOCOL_VERSION_2025_06)
        );
        assert_eq!(McpProtocolMode::V2025June.pinned_stateful(), Some(true));
        assert_eq!(
            McpProtocolMode::V2026July.pinned_version(),
            Some(MCP_PROTOCOL_VERSION_2026_07)
        );
        assert_eq!(McpProtocolMode::V2026July.pinned_stateful(), Some(false));
    }

    #[test]
    fn scoped_mcp_server_omits_auto_protocol_mode_but_keeps_pinned() {
        // Default (auto) is skipped on the wire so existing config is byte-identical.
        let auto = ScopedMcpServer {
            url: "https://example.com/mcp".to_string(),
            ..Default::default()
        };
        let json = serde_json::to_value(&auto).unwrap();
        assert!(
            json.get("protocol_mode").is_none(),
            "auto protocol_mode must not serialize: {json}"
        );

        // A pinned mode is preserved.
        let pinned = ScopedMcpServer {
            url: "https://example.com/mcp".to_string(),
            protocol_mode: McpProtocolMode::V2025March,
            ..Default::default()
        };
        let json = serde_json::to_value(&pinned).unwrap();
        assert_eq!(
            json.get("protocol_mode").and_then(|v| v.as_str()),
            Some(MCP_PROTOCOL_VERSION_2025_03),
            "pinned modes serialize as the version date, not the retired `legacy` alias"
        );
    }

    #[test]
    fn scoped_mcp_server_parses_protocol_mode_from_mcp_json_shape() {
        // `.mcp.json`-style config can pin an era; absence means auto.
        let with_mode: ScopedMcpServer = serde_json::from_value(serde_json::json!({
            "type": "http",
            "url": "https://example.com/mcp",
            "protocol_mode": "rc"
        }))
        .unwrap();
        assert_eq!(with_mode.protocol_mode, McpProtocolMode::V2026July);

        let without_mode: ScopedMcpServer = serde_json::from_value(serde_json::json!({
            "type": "http",
            "url": "https://example.com/mcp"
        }))
        .unwrap();
        assert_eq!(without_mode.protocol_mode, McpProtocolMode::Auto);
    }

    #[test]
    fn merge_scoped_mcp_servers_lets_later_layer_override_protocol_mode() {
        // Session can pin an era over a harness/agent default — last-wins layering.
        let mut base = ScopedMcpServers::default();
        base.insert(
            "docs".to_string(),
            ScopedMcpServer {
                url: "https://example.com/mcp".to_string(),
                protocol_mode: McpProtocolMode::Auto,
                ..Default::default()
            },
        );
        let mut overlay = ScopedMcpServers::default();
        overlay.insert(
            "docs".to_string(),
            ScopedMcpServer {
                url: "https://example.com/mcp".to_string(),
                protocol_mode: McpProtocolMode::V2025March,
                ..Default::default()
            },
        );
        let merged = merge_scoped_mcp_servers(&base, &overlay);
        assert_eq!(
            merged.get("docs").unwrap().protocol_mode,
            McpProtocolMode::V2025March
        );
    }

    #[test]
    fn normalize_mcp_error_code_maps_legacy_to_rc() {
        // 2026-07-28 renumbered -32002 onto the standard -32602; everything else passes through.
        assert_eq!(normalize_mcp_error_code(-32002), -32602);
        assert_eq!(normalize_mcp_error_code(-32602), -32602);
        assert_eq!(normalize_mcp_error_code(-32601), -32601);
        assert_eq!(normalize_mcp_error_code(0), 0);
    }

    #[test]
    fn test_mcp_tool_name_simple() {
        // Simple server name without special characters
        assert_eq!(mcp_tool_name("github", "search"), "mcp_github__search");
    }

    #[test]
    fn test_mcp_tool_name_with_underscores() {
        // Server name with underscores (e.g., microsoft_learn)
        assert_eq!(
            mcp_tool_name("microsoft_learn", "docs_search"),
            "mcp_microsoft_learn__docs_search"
        );
    }

    #[test]
    fn test_mcp_tool_name_with_dashes() {
        // Server name with dashes gets converted to underscores
        assert_eq!(
            mcp_tool_name("microsoft-learn", "search"),
            "mcp_microsoft_learn__search"
        );
    }

    #[test]
    fn test_mcp_tool_name_uppercase() {
        // Server name is lowercased
        assert_eq!(mcp_tool_name("GitHub", "search"), "mcp_github__search");
    }

    #[test]
    fn test_mcp_tool_name_special_chars() {
        // Special characters are replaced with underscores
        assert_eq!(
            mcp_tool_name("my.server.name", "tool"),
            "mcp_my_server_name__tool"
        );
    }

    #[test]
    fn test_is_mcp_tool() {
        assert!(is_mcp_tool("mcp_github__search"));
        assert!(is_mcp_tool("mcp_microsoft_learn__docs_search"));
        assert!(!is_mcp_tool("get_weather"));
        assert!(!is_mcp_tool("mcpsearch")); // Must have underscore after mcp
    }

    #[test]
    fn test_parse_mcp_tool_name_simple() {
        let result = parse_mcp_tool_name("mcp_github__search");
        assert_eq!(result, Some(("github".to_string(), "search".to_string())));
    }

    #[test]
    fn test_parse_mcp_tool_name_with_underscores() {
        // Server name with underscores should be parsed correctly
        let result = parse_mcp_tool_name("mcp_microsoft_learn__docs_search");
        assert_eq!(
            result,
            Some(("microsoft_learn".to_string(), "docs_search".to_string()))
        );
    }

    #[test]
    fn test_parse_mcp_tool_name_complex() {
        // Multiple underscores in both server name and tool name
        let result = parse_mcp_tool_name("mcp_my_long_server_name__my_complex_tool");
        assert_eq!(
            result,
            Some((
                "my_long_server_name".to_string(),
                "my_complex_tool".to_string()
            ))
        );
    }

    #[test]
    fn test_parse_mcp_tool_name_invalid_prefix() {
        // Not an MCP tool
        assert_eq!(parse_mcp_tool_name("get_weather"), None);
    }

    #[test]
    fn test_parse_mcp_tool_name_no_separator() {
        // Missing double underscore separator
        assert_eq!(parse_mcp_tool_name("mcp_github_search"), None);
    }

    #[test]
    fn test_parse_mcp_tool_name_empty_parts() {
        // Empty server name or tool name
        assert_eq!(parse_mcp_tool_name("mcp___search"), None);
        assert_eq!(parse_mcp_tool_name("mcp_github__"), None);
    }

    #[test]
    fn test_roundtrip() {
        // Generate and parse should roundtrip
        let server = "microsoft_learn";
        let tool = "docs_search";
        let full_name = mcp_tool_name(server, tool);
        let parsed = parse_mcp_tool_name(&full_name);
        assert_eq!(
            parsed,
            Some(("microsoft_learn".to_string(), "docs_search".to_string()))
        );
    }

    // ------------------------------------------------------------------
    // McpExecuteError / McpErrorCode (EVE-492)
    // ------------------------------------------------------------------

    #[test]
    fn mcp_error_code_serializes_to_snake_case_wire_string() {
        assert_eq!(
            serde_json::to_string(&McpErrorCode::ToolTimeout).unwrap(),
            "\"tool_timeout\""
        );
        assert_eq!(
            serde_json::to_string(&McpErrorCode::McpServerUnreachable).unwrap(),
            "\"mcp_server_unreachable\""
        );
    }

    #[test]
    fn mcp_error_code_as_str_matches_serde_wire() {
        for code in [
            McpErrorCode::ToolNotFound,
            McpErrorCode::ToolTimeout,
            McpErrorCode::ToolPanicked,
            McpErrorCode::InvalidArguments,
            McpErrorCode::PermissionDenied,
            McpErrorCode::QuotaExceeded,
            McpErrorCode::NetworkBlocked,
            McpErrorCode::McpServerUnreachable,
            McpErrorCode::Internal,
            McpErrorCode::Unknown,
        ] {
            let wire = serde_json::to_string(&code).unwrap();
            assert_eq!(
                wire,
                format!("\"{}\"", code.as_str()),
                "as_str() must match serde wire for {code:?}"
            );
        }
    }

    #[test]
    fn mcp_error_code_unknown_variant_is_forward_compat_sentinel() {
        // SDKs that receive a code they don't recognise should land on
        // `Unknown`, not fail to deserialise.
        let code: McpErrorCode = serde_json::from_str("\"future_code_we_dont_know_yet\"").unwrap();
        assert_eq!(code, McpErrorCode::Unknown);
    }

    #[test]
    fn classify_recognises_timeout_substrings() {
        let err = classify_mcp_execute_error("Tool timed out after 30000ms");
        assert_eq!(err.code, McpErrorCode::ToolTimeout);
        assert_eq!(err.category, McpErrorCategory::Transient);
        assert!(err.retryable);

        let err = classify_mcp_execute_error("Command timed out after 5000ms");
        assert_eq!(err.code, McpErrorCode::ToolTimeout);
    }

    #[test]
    fn classify_recognises_tool_not_found() {
        let err = classify_mcp_execute_error("Unknown tool: github.foo");
        assert_eq!(err.code, McpErrorCode::ToolNotFound);
        assert_eq!(err.category, McpErrorCategory::Permanent);
        assert!(!err.retryable);
    }

    #[test]
    fn classify_recognises_invalid_arguments() {
        let err = classify_mcp_execute_error("Missing required parameter: query");
        assert_eq!(err.code, McpErrorCode::InvalidArguments);
        assert_eq!(err.category, McpErrorCategory::Validation);
        assert!(!err.retryable);
    }

    #[test]
    fn classify_recognises_permission_denied() {
        for msg in [
            "permission denied for org",
            "Forbidden: org scope not allowed",
            "not authorized to call this tool",
            "Unauthorized request",
        ] {
            let err = classify_mcp_execute_error(msg);
            assert_eq!(
                err.code,
                McpErrorCode::PermissionDenied,
                "expected PermissionDenied for {msg:?}"
            );
            assert_eq!(err.category, McpErrorCategory::Auth);
        }
    }

    #[test]
    fn classify_recognises_quota_and_rate_limit() {
        let err = classify_mcp_execute_error("Quota exceeded for org");
        assert_eq!(err.code, McpErrorCode::QuotaExceeded);
        assert!(err.retryable);

        let err = classify_mcp_execute_error("Rate limit hit");
        assert_eq!(err.code, McpErrorCode::QuotaExceeded);
    }

    #[test]
    fn classify_recognises_catalog_dispatch_prefixes() {
        // `crates/server/src/api/mcp_endpoint/catalog.rs::format_dispatch_error`
        // emits `<kind>: <message>` for inventory-backed query/execute
        // tools. These are the public MCP contract per knowledge/foundations/domains.md,
        // so the classifier must route them to precise codes rather than
        // the catch-all `Internal` bucket.
        for (prefix, expected) in [
            (
                "bad_request: name must be <=200 chars",
                McpErrorCode::InvalidArguments,
            ),
            (
                "unprocessable: cycle detected in capability graph",
                McpErrorCode::InvalidArguments,
            ),
            (
                "conflict: session is already paused",
                McpErrorCode::InvalidArguments,
            ),
            (
                "not_found: agent agent_xyz not in this org",
                McpErrorCode::ToolNotFound,
            ),
            (
                "forbidden: principal lacks SESSION_WRITE",
                McpErrorCode::PermissionDenied,
            ),
            (
                "internal: storage backend returned 503",
                McpErrorCode::Internal,
            ),
        ] {
            let err = classify_mcp_execute_error(prefix);
            assert_eq!(err.code, expected, "expected {expected:?} for {prefix:?}");
        }
    }

    #[test]
    fn classify_falls_open_to_internal() {
        // No known pattern → Internal, not a wrong guess. Retryable
        // defaults to false so callers don't burn retries on unknown
        // permanent failures.
        let err = classify_mcp_execute_error("strange unanticipated message");
        assert_eq!(err.code, McpErrorCode::Internal);
        assert_eq!(err.category, McpErrorCategory::Permanent);
        assert!(!err.retryable);
    }

    #[test]
    fn mcp_execute_error_skips_empty_optional_fields() {
        let err = McpExecuteError::new(McpErrorCode::ToolNotFound, "no such tool");
        let value = serde_json::to_value(&err).unwrap();
        // Required fields present.
        assert_eq!(value["code"], "tool_not_found");
        assert_eq!(value["message"], "no such tool");
        assert_eq!(value["category"], "permanent");
        assert_eq!(value["retryable"], false);
        // Optional fields omitted entirely from the wire when empty.
        assert!(value.get("retry_after_seconds").is_none());
        assert!(value.get("hint").is_none());
        assert!(value.get("cause_chain").is_none());
    }

    #[test]
    fn mcp_execute_error_builders_chain() {
        let err = McpExecuteError::new(McpErrorCode::ToolTimeout, "tool timed out after 30000ms")
            .with_retry_after_seconds(10)
            .with_hint("Reduce input size before retrying.")
            .with_cause("downstream: upstream gateway timeout");
        let value = serde_json::to_value(&err).unwrap();
        assert_eq!(value["code"], "tool_timeout");
        assert_eq!(value["retry_after_seconds"], 10);
        assert_eq!(value["hint"], "Reduce input size before retrying.");
        assert_eq!(
            value["cause_chain"][0],
            "downstream: upstream gateway timeout"
        );
    }
    #[test]
    fn server_name_validation_preserves_unambiguous_generated_names() {
        for name in [
            "",
            "_",
            "docs_",
            "docs-",
            "docs ",
            "docs__private",
            "docs..private",
            "--docs",
        ] {
            assert!(!is_valid_mcp_server_name(name), "{name:?}");
        }
        for (name, prefix) in [
            ("docs", "docs"),
            ("docs-api", "docs_api"),
            ("_docs", "_docs"),
            ("Docs API", "docs_api"),
        ] {
            assert!(is_valid_mcp_server_name(name), "{name:?}");
            for tool in ["search", "_search", "read__file"] {
                assert_eq!(
                    parse_mcp_tool_name(&mcp_tool_name(name, tool)),
                    Some((prefix.into(), tool.into()))
                );
            }
        }
    }
    #[test]
    fn ambiguous_bindings_do_not_rewrite_a_different_tool() {
        let schema = serde_json::json!({"type":"object","properties":{"key":{"type":"string"}},"required":["key"]});
        let mut definitions = vec![crate::ToolDefinition::Builtin(crate::BuiltinTool {
            name: "mcp_docs___search".into(),
            display_name: None,
            description: "Search".into(),
            parameters: schema,
            policy: Default::default(),
            category: None,
            deferrable: Default::default(),
            hints: Default::default(),
            full_parameters: None,
        })];
        let before = serde_json::to_value(&definitions).unwrap();
        let mut binding = McpSecretBindingMetadata {
            server_name: "docs_".into(),
            tool_name: "search".into(),
            parameter_name: "key".into(),
            configured: true,
            setup_url: "/setup".into(),
        };
        apply_mcp_secret_binding_schemas(&mut definitions, &[binding.clone()]);
        assert_eq!(serde_json::to_value(&definitions).unwrap(), before);
        binding.server_name = "docs".into();
        binding.tool_name = "_search".into();
        apply_mcp_secret_binding_schemas(&mut definitions, &[binding]);
        let crate::ToolDefinition::Builtin(definition) = &definitions[0] else {
            panic!("expected builtin")
        };
        assert_eq!(
            definition.parameters,
            serde_json::json!({"type":"object","properties":{},"required":[]})
        );
    }
}
