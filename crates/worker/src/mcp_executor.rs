// MCP server connection info.
//
// Spec: knowledge/integrations/mcp-servers.md (execution detail), knowledge/integrations/runtime-mcp.md (D5).
//
// The transport-agnostic MCP client (discovery, tools/call, SSE parsing, SSRF)
// now lives in the shared `everruns-mcp` crate. This module only carries the
// gRPC-resolved server descriptor (`McpServerInfo`) used by the worker's
// gRPC adapters; the previous worker-local JSON-RPC executor was removed to
// avoid duplicating that client (goal: no duplication).

use everruns_core::{
    ConnectionRequiredSubject, McpProtocolMode, McpServerActsAs, McpServerAuthMode,
};
use everruns_internal_protocol::proto;
use everruns_provider::error::{AgentLoopError, Result};
use std::collections::HashMap;
use uuid::Uuid;

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
    pub oauth_provider_id: Option<String>,
    pub acts_as: McpServerActsAs,
    pub connection_subject: Option<ConnectionRequiredSubject>,
    pub connection_setup_url: Option<String>,
    pub secret_bindings: HashMap<String, Vec<everruns_mcp::McpSecretBinding>>,
}

pub(crate) fn proto_mcp_server_to_info(
    proto_server: proto::McpServerInfo,
) -> Result<McpServerInfo> {
    let id = proto_server
        .id
        .as_ref()
        .ok_or_else(|| AgentLoopError::store("gRPC response error: Missing UUID in response"))
        .and_then(|id| {
            Uuid::parse_str(&id.value)
                .map_err(|error| AgentLoopError::store(format!("Invalid UUID: {error}")))
        })?;
    let auth_mode = if proto_server.auth_mode.is_empty() && proto_server.api_key.is_some() {
        McpServerAuthMode::ApiKey
    } else {
        McpServerAuthMode::from(proto_server.auth_mode.as_str())
    };
    let connection_subject = match (
        proto_server.connection_subject_kind.as_deref(),
        proto_server.connection_subject_name,
    ) {
        (Some("agent"), Some(name)) => Some(ConnectionRequiredSubject {
            kind: everruns_core::ConnectionRequiredSubjectKind::Agent,
            name,
        }),
        (Some("user"), Some(name)) => Some(ConnectionRequiredSubject {
            kind: everruns_core::ConnectionRequiredSubjectKind::User,
            name,
        }),
        _ => None,
    };

    Ok(McpServerInfo {
        id,
        name: proto_server.name,
        url: proto_server.url,
        api_key: proto_server.api_key,
        headers: proto_server.headers,
        auth_mode,
        protocol_mode: McpProtocolMode::from(proto_server.protocol_mode.as_str()),
        oauth_provider_id: proto_server.oauth_provider_id,
        acts_as: McpServerActsAs::from(proto_server.acts_as.as_str()),
        connection_subject,
        connection_setup_url: proto_server.connection_setup_url,
        secret_bindings: proto_server.secret_bindings.into_iter().fold(
            HashMap::new(),
            |mut bindings, binding| {
                bindings
                    .entry(binding.tool_name)
                    .or_insert_with(Vec::new)
                    .push(everruns_mcp::McpSecretBinding {
                        parameter_name: binding.parameter_name,
                        value: binding.value,
                        setup_url: binding.setup_url,
                        label: binding.label,
                    });
                bindings
            },
        ),
    })
}
