//! Transport abstraction over MCP connections.
//!
//! A [`McpTransport`] speaks JSON-RPC against one logical server and returns
//! parsed results, so result/content mapping is shared across transports. The
//! HTTP transport is always compiled; stdio is behind the `stdio` feature and
//! is the "hard-off in hosted" boundary.

use crate::mcp::auth::McpCredential;
use crate::{
    McpElicitationPolicy, McpProtocolMode, McpServerAuthMode, McpToolCallResult, McpToolDefinition,
};
use async_trait::async_trait;
use everruns_contracts::ConnectionRequired;
use serde_json::Value;
use std::collections::HashMap;

/// Transport-specific connection target.
#[derive(Debug, Clone)]
pub enum McpEndpoint {
    /// Remote Streamable-HTTP MCP server.
    Http {
        url: String,
        headers: HashMap<String, String>,
    },
    /// Local process speaking MCP over stdio. Only constructible when the
    /// `stdio` feature is enabled (hard-off in hosted builds).
    #[cfg(feature = "mcp-stdio")]
    Stdio {
        command: String,
        args: Vec<String>,
        env: HashMap<String, String>,
    },
}

/// A resolved, transport-agnostic MCP server connection.
#[derive(Debug, Clone)]
pub struct McpConnection {
    /// Logical server name (used for tool-name prefixing and auth lookup).
    pub name: String,
    pub endpoint: McpEndpoint,
    pub auth_mode: McpServerAuthMode,
    /// Protocol-era policy. `Auto` (default) negotiates every protocol era.
    pub protocol_mode: McpProtocolMode,
    /// Which elicitation modes may be declared to this server. `Url` (default)
    /// is the behaviour every server had before the policy existed.
    pub elicitation_policy: McpElicitationPolicy,
    pub oauth_provider_id: Option<String>,
    /// Set by the connection resolver when this server requires an OAuth grant.
    /// The executor short-circuits into a structured `connection_required`
    /// result instead of sending an unauthenticated request.
    pub pending_oauth_provider: Option<ConnectionRequired>,
    /// Whether `pending_oauth_provider` may pause the turn with an in-chat
    /// card (`ask`) or is reported as a plain tool error with the setup link
    /// (`never`). Set from the agent's MCP attachment.
    pub connect_in_chat: crate::McpConnectInChat,
    /// Write-only Agent credentials bound to exact MCP tool parameters. Values
    /// are resolved by the control plane and injected only inside the executor.
    pub secret_bindings: HashMap<String, Vec<McpSecretBinding>>,
    /// Account whose grant supplied this connection's credential (`user` or
    /// `service`), when the resolver picked one. Recorded on the tool call's
    /// event and used to invalidate the right grant after a rejection.
    pub acted_as: Option<crate::McpServerActsAs>,
}

#[derive(Clone)]
pub struct McpSecretBinding {
    pub parameter_name: String,
    pub value: Option<String>,
    pub setup_url: String,
    pub label: String,
}

impl std::fmt::Debug for McpSecretBinding {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("McpSecretBinding")
            .field("parameter_name", &self.parameter_name)
            .field("configured", &self.value.is_some())
            .field("setup_url", &self.setup_url)
            .field("label", &self.label)
            .finish()
    }
}

impl McpConnection {
    /// Convenience constructor for a no-auth HTTP server (protocol `Auto`).
    pub fn http(name: impl Into<String>, url: impl Into<String>) -> Self {
        Self {
            name: name.into(),
            endpoint: McpEndpoint::Http {
                url: url.into(),
                headers: HashMap::new(),
            },
            auth_mode: McpServerAuthMode::None,
            protocol_mode: McpProtocolMode::Auto,
            elicitation_policy: McpElicitationPolicy::Url,
            oauth_provider_id: None,
            pending_oauth_provider: None,
            connect_in_chat: crate::McpConnectInChat::Ask,
            secret_bindings: HashMap::new(),
            acted_as: None,
        }
    }

    /// Pin the protocol-era policy for this connection (builder-style).
    pub fn with_protocol_mode(mut self, mode: McpProtocolMode) -> Self {
        self.protocol_mode = mode;
        self
    }

    /// Set which elicitation modes this server may use (builder-style).
    pub fn with_elicitation_policy(mut self, policy: McpElicitationPolicy) -> Self {
        self.elicitation_policy = policy;
        self
    }
}

/// Speaks MCP JSON-RPC against a single connection.
#[async_trait]
pub trait McpTransport: Send + Sync {
    async fn list_tools(
        &self,
        connection: &McpConnection,
        credential: Option<&McpCredential>,
    ) -> anyhow::Result<Vec<McpToolDefinition>>;

    async fn call_tool(
        &self,
        connection: &McpConnection,
        tool_name: &str,
        arguments: Value,
        credential: Option<&McpCredential>,
    ) -> anyhow::Result<McpToolCallResult>;
}
