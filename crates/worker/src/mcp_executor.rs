// MCP server connection info.
//
// Spec: knowledge/integrations/mcp-servers.md (execution detail), knowledge/integrations/runtime-mcp.md (D5).
//
// The transport-agnostic MCP client (discovery, tools/call, SSE parsing, SSRF)
// now lives in the core MCP module. This module only carries the
// gRPC-resolved server descriptor (`McpServerInfo`) used by the worker's
// gRPC adapters; the previous worker-local JSON-RPC executor was removed to
// avoid duplicating that client (goal: no duplication).

use crate::core::{McpElicitationPolicy, McpProtocolMode, McpServerActsAs, McpServerAuthMode};
use everruns_internal_protocol::proto;
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
    /// Protocol-era adoption policy (`auto` negotiates every protocol era).
    pub protocol_mode: McpProtocolMode,
    /// Which elicitation modes the operator allows this server to use.
    pub elicitation_policy: McpElicitationPolicy,
    pub oauth_provider_id: Option<String>,
    pub acts_as: McpServerActsAs,
    pub secret_bindings: HashMap<String, Vec<crate::mcp::McpSecretBinding>>,
}

impl McpServerInfo {
    /// Decode the gRPC descriptor. `id` is parsed by the caller, which owns the
    /// error type for a malformed UUID.
    pub(crate) fn from_proto(id: uuid::Uuid, proto_server: proto::McpServerInfo) -> Self {
        let auth_mode = if proto_server.auth_mode.is_empty() && proto_server.api_key.is_some() {
            McpServerAuthMode::ApiKey
        } else {
            McpServerAuthMode::from(proto_server.auth_mode.as_str())
        };

        Self {
            id,
            name: proto_server.name,
            url: proto_server.url,
            api_key: proto_server.api_key,
            headers: proto_server.headers,
            auth_mode,
            protocol_mode: McpProtocolMode::from(proto_server.protocol_mode.as_str()),
            // An older control plane sends nothing, which reads as the default.
            elicitation_policy: McpElicitationPolicy::from(
                proto_server.elicitation_policy.as_str(),
            ),
            oauth_provider_id: proto_server.oauth_provider_id,
            acts_as: McpServerActsAs::from(proto_server.acts_as.as_str()),
            secret_bindings: proto_server.secret_bindings.into_iter().fold(
                HashMap::new(),
                |mut bindings, binding| {
                    bindings
                        .entry(binding.tool_name)
                        .or_insert_with(Vec::new)
                        .push(crate::mcp::McpSecretBinding {
                            parameter_name: binding.parameter_name,
                            value: binding.value,
                            setup_url: binding.setup_url,
                            label: binding.label,
                        });
                    bindings
                },
            ),
        }
    }
}
