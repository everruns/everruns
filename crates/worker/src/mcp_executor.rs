// MCP server connection info.
//
// Spec: specs/mcp-servers.md (execution detail), specs/runtime-mcp.md (D5).
//
// The transport-agnostic MCP client (discovery, tools/call, SSE parsing, SSRF)
// now lives in the shared `everruns-mcp` crate. This module only carries the
// gRPC-resolved server descriptor (`McpServerInfo`) used by the worker's
// gRPC adapters; the previous worker-local JSON-RPC executor was removed to
// avoid duplicating that client (goal: no duplication).

use everruns_core::{McpProtocolMode, McpServerAuthMode};
use std::collections::HashMap;

/// MCP server info resolved over gRPC, needed to contact a remote MCP server.
#[derive(Debug, Clone)]
pub struct McpServerInfo {
    pub id: uuid::Uuid,
    pub name: String,
    pub url: String,
    pub api_key: Option<String>,
    pub headers: HashMap<String, String>,
    pub auth_mode: McpServerAuthMode,
    /// Protocol-era adoption policy (`auto` negotiates legacy/current/RC).
    pub protocol_mode: McpProtocolMode,
    pub oauth_provider_id: Option<String>,
}
