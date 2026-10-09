//! MCP tool execution.
//!
//! [`McpExecutor`] resolves a tool call's server prefix to a [`McpConnection`]
//! and executes it via [`McpClient`]. It implements [`McpToolInvoker`] so hosts
//! register MCP tools as first-class [`crate::tools::Tool`] entries in
//! the regular `ToolRegistry` (via `everruns_core::build_mcp_proxy_tools`),
//! instead of routing `mcp_*` calls through a separate executor.

use crate::capabilities::Capability as _;
use crate::mcp::client::McpClient;
use crate::mcp::elicitation::{ElicitationAction, UrlElicitationPending};
use crate::mcp::form_elicitation::FormElicitationPending;
use crate::mcp::http::McpHttpStatusError;
use crate::mcp::transport::McpConnection;
use crate::mcp_server::sanitize_mcp_server_name;
use crate::{McpServerTools, McpToolInvoker, parse_mcp_tool_name};
use anyhow::{Result, anyhow};
use async_trait::async_trait;
use everruns_contracts::error::{AgentLoopError, Result as CoreResult};
use everruns_contracts::tool_types::{
    FORM_ELICITATION_REQUIRED_CODE, FormElicitationRequired, ToolCall, ToolResult,
    URL_ELICITATION_REQUIRED_CODE, UrlElicitationRequired,
};
use std::collections::HashMap;
use std::sync::Arc;

const REDACTED_CREDENTIAL: &str = "[REDACTED MCP CREDENTIAL]";

fn redact_text(text: &str, secrets: &[String]) -> String {
    secrets
        .iter()
        .filter(|secret| !secret.is_empty())
        .fold(text.to_string(), |redacted, secret| {
            redacted.replace(secret, REDACTED_CREDENTIAL)
        })
}

fn redact_json(value: &mut serde_json::Value, secrets: &[String]) {
    match value {
        serde_json::Value::String(text) => *text = redact_text(text, secrets),
        serde_json::Value::Array(values) => {
            for value in values {
                redact_json(value, secrets);
            }
        }
        serde_json::Value::Object(object) => {
            for value in object.values_mut() {
                redact_json(value, secrets);
            }
        }
        _ => {}
    }
}

fn is_unauthorized(error: &anyhow::Error) -> bool {
    error.chain().any(|cause| {
        cause
            .downcast_ref::<McpHttpStatusError>()
            .is_some_and(McpHttpStatusError::is_unauthorized)
    })
}

fn redact_tool_result(result: &mut ToolResult, secrets: &[String]) {
    // Nothing was injected for this call, so nothing can be reflected. Skip the
    // walk instead of reallocating every string in the result (base64 images in
    // particular) on the overwhelmingly common no-binding path.
    if secrets.is_empty() {
        return;
    }
    if let Some(value) = result.result.as_mut() {
        redact_json(value, secrets);
    }
    if let Some(images) = result.images.as_mut() {
        for image in images {
            image.base64 = redact_text(&image.base64, secrets);
            image.media_type = redact_text(&image.media_type, secrets);
        }
    }
    if let Some(error) = result.error.as_mut() {
        *error = redact_text(error, secrets);
    }
    if let Some(raw_output) = result.raw_output.as_mut() {
        *raw_output = redact_text(raw_output, secrets);
    }
}

/// Resolves a sanitized server prefix to a connection. Implementations differ
/// per host (runtime: effective scoped servers; worker: gRPC lookup).
#[async_trait]
pub trait McpConnectionResolver: Send + Sync {
    fn for_execution(
        &self,
        _input_message_id: uuid::Uuid,
    ) -> Option<Arc<dyn McpConnectionResolver>> {
        None
    }

    async fn resolve(&self, server_prefix: &str) -> Result<Option<McpConnection>>;
    async fn invalidate(
        &self,
        _server_prefix: &str,
        _rejected_connection: &McpConnection,
    ) -> Result<()> {
        Ok(())
    }
}

/// In-memory resolver over a fixed set of connections, keyed by sanitized
/// server name. Used by runtime/CLI hosts that hold the effective scoped
/// servers up front.
#[derive(Default)]
pub struct StaticConnectionResolver {
    connections: HashMap<String, McpConnection>,
}

impl StaticConnectionResolver {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn insert(&mut self, connection: McpConnection) {
        if !crate::mcp_server::is_valid_mcp_server_name(&connection.name) {
            tracing::warn!(server = %connection.name, "MCP server ignored: ambiguous tool prefix");
            return;
        }
        let key = sanitize_mcp_server_name(&connection.name);
        self.connections.insert(key, connection);
    }

    pub fn with(mut self, connection: McpConnection) -> Self {
        self.insert(connection);
        self
    }

    pub fn from_connections(connections: impl IntoIterator<Item = McpConnection>) -> Self {
        let mut resolver = Self::new();
        for connection in connections {
            resolver.insert(connection);
        }
        resolver
    }

    pub fn is_empty(&self) -> bool {
        self.connections.is_empty()
    }
}

#[async_trait]
impl McpConnectionResolver for StaticConnectionResolver {
    async fn resolve(&self, server_prefix: &str) -> Result<Option<McpConnection>> {
        Ok(self.connections.get(server_prefix).cloned())
    }
}

/// Executes `mcp_*` tool calls against remote/local MCP servers.
pub struct McpExecutor {
    client: Arc<McpClient>,
    resolver: Arc<dyn McpConnectionResolver>,
}

impl McpExecutor {
    pub fn new(client: Arc<McpClient>, resolver: Arc<dyn McpConnectionResolver>) -> Self {
        Self { client, resolver }
    }

    pub async fn execute_mcp_tool(&self, tool_call: &ToolCall) -> Result<ToolResult> {
        self.execute_mcp_tool_recorded(tool_call)
            .await
            .map(|(result, _)| result)
    }

    /// Execute a call and report which account its credential came from
    /// (`user` or `service`). A call that never reached the server, such as one
    /// waiting for a sign-in, reports none.
    pub async fn execute_mcp_tool_recorded(
        &self,
        tool_call: &ToolCall,
    ) -> Result<(ToolResult, Option<crate::McpServerActsAs>)> {
        let (server_prefix, original_tool_name) = parse_mcp_tool_name(&tool_call.name)
            .ok_or_else(|| anyhow!("Invalid MCP tool name: {}", tool_call.name))?;

        let connection = self
            .resolver
            .resolve(&server_prefix)
            .await?
            .ok_or_else(|| anyhow!("MCP server not found for prefix: {server_prefix}"))?;

        // The server requires an OAuth grant that has not been configured yet.
        // Return a connection_required result (the host renders an inline
        // connect prompt) instead of letting the call fail with a 401.
        if let Some(required) = &connection.pending_oauth_provider {
            return Ok((
                connection_required_result(tool_call.id.clone(), &connection, required),
                None,
            ));
        }

        let mut arguments = tool_call.arguments.clone();
        let mut injected_secrets = Vec::new();
        if let Some(bindings) = connection.secret_bindings.get(&original_tool_name) {
            let object = arguments
                .as_object_mut()
                .ok_or_else(|| anyhow!("MCP tool arguments must be an object"))?;
            for binding in bindings {
                if object.contains_key(&binding.parameter_name) {
                    return Ok((
                        ToolResult {
                            tool_call_id: tool_call.id.clone(),
                            result: Some(serde_json::json!({
                                "code": "credential_override_rejected",
                                "error": format!(
                                    "Credential parameter '{}' is securely bound and cannot be supplied by the model",
                                    binding.parameter_name
                                ),
                            })),
                            images: None,
                            error: Some("Secure credential override rejected".to_string()),
                            connection_required: None,
                            raw_output: None,
                        },
                        None,
                    ));
                }
                let Some(value) = binding.value.as_ref() else {
                    return Ok((
                        ToolResult {
                            tool_call_id: tool_call.id.clone(),
                            result: Some(serde_json::json!({
                                "code": "credential_required",
                                "error": format!("{} is not configured", binding.label),
                                "setup_url": binding.setup_url,
                                "credential_label": binding.label,
                            })),
                            images: None,
                            // Keep the structured setup payload intact through the
                            // MCP proxy so clients can render a direct affordance.
                            // This is an expected, user-actionable state rather
                            // than a transport failure.
                            error: None,
                            connection_required: None,
                            raw_output: None,
                        },
                        None,
                    ));
                };
                object.insert(
                    binding.parameter_name.clone(),
                    serde_json::Value::String(value.clone()),
                );
                if !value.is_empty() {
                    injected_secrets.push(value.clone());
                }
            }
        }

        let result = self
            .client
            .call_as_tool_result(
                &connection,
                tool_call.id.clone(),
                &original_tool_name,
                arguments,
            )
            .await;
        let result = match result {
            Err(error) if is_unauthorized(&error) => {
                self.resolver
                    .invalidate(&server_prefix, &connection)
                    .await?;
                if let Some(connection) = self.resolver.resolve(&server_prefix).await?
                    && let Some(required) = &connection.pending_oauth_provider
                {
                    return Ok((
                        connection_required_result(tool_call.id.clone(), &connection, required),
                        None,
                    ));
                }
                Err(error)
            }
            result => result,
        };

        // THREAT[TM-TOOL-029]: the remote server can reflect credential-bearing
        // arguments in successful content or any transport/JSON-RPC error.
        // Scrub at the executor boundary before results or errors reach events,
        // model context, persistence, tracing, or caller logs.
        let acted_as = connection.acted_as;
        match result {
            Ok(mut result) => {
                redact_tool_result(&mut result, &injected_secrets);
                Ok((result, acted_as))
            }
            // A URL mode elicitation the user has not completed yet. Like a
            // missing credential binding, this is an expected, user-actionable
            // state rather than a transport failure, so it comes back as a
            // structured result the client can render and the model can relay —
            // and the user re-runs the tool once they are done.
            Err(error) if error.downcast_ref::<UrlElicitationPending>().is_some() => {
                let Some(pending) = error.downcast_ref::<UrlElicitationPending>() else {
                    return Err(error);
                };
                Ok((
                    url_elicitation_result(tool_call.id.clone(), &tool_call.name, pending),
                    acted_as,
                ))
            }
            // A server's questions nobody has answered yet. The same kind of
            // expected, user-actionable state: the engine parks the turn on an
            // `ask_user` card and the retry sends the recorded answer.
            Err(error) if error.downcast_ref::<FormElicitationPending>().is_some() => {
                let Some(pending) = error.downcast_ref::<FormElicitationPending>() else {
                    return Err(error);
                };
                Ok((
                    form_elicitation_result(tool_call.id.clone(), &tool_call.name, pending),
                    acted_as,
                ))
            }
            // Redacting an error flattens it to a string, so keep the original
            // chain intact whenever there is nothing to scrub.
            Err(error) if injected_secrets.is_empty() => Err(error),
            Err(error) => Err(anyhow!(redact_text(&error.to_string(), &injected_secrets))),
        }
    }
}

/// The result for a call whose server is missing a grant.
///
/// With `connectInChat: ask` it carries `connection_required`, which the host
/// turns into an in-chat card and pauses the turn on. With `never` it is an
/// ordinary tool error naming the server and where to connect it, so the turn
/// continues and a channel that cannot render a card still gets a usable link
/// (user MCP servers D5).
fn connection_required_result(
    tool_call_id: String,
    connection: &McpConnection,
    required: &everruns_contracts::ConnectionRequired,
) -> ToolResult {
    let connection_name = &connection.name;
    let subject = match required.subject {
        Some(everruns_contracts::ConnectionRequiredSubject::Agent) => "agent",
        Some(everruns_contracts::ConnectionRequiredSubject::User) => "user",
        None => "user",
    };
    if !connection.connect_in_chat.allows_card() {
        let setup_url = required
            .setup_url
            .clone()
            .unwrap_or_else(|| "/settings/connections".to_string());
        let whose = if subject == "agent" {
            "An admin must authorize the agent's sign-in"
        } else {
            "The person must connect their account"
        };
        return ToolResult {
            tool_call_id,
            result: Some(serde_json::json!({
                "code": "connection_required",
                "server": connection_name,
                "provider": required.provider,
                "subject": subject,
                "setup_url": setup_url,
                "connect_in_chat": connection.connect_in_chat,
            })),
            images: None,
            error: Some(format!(
                "MCP server '{connection_name}' is not connected and cannot be connected from \
                 this chat. {whose} at {setup_url}, then try again."
            )),
            connection_required: None,
            raw_output: None,
        };
    }
    ToolResult {
        tool_call_id,
        result: None,
        images: None,
        error: Some(format!(
            "MCP server '{connection_name}' requires an OAuth connection. \
             Ask the {subject} to connect provider '{}'.",
            required.provider
        )),
        connection_required: Some(required.clone()),
        raw_output: None,
    }
}

/// Turn a pending URL elicitation into the tool result the user sees.
///
/// The URL is passed through untouched (it was validated before any human saw
/// it) and is the only actionable part; the server's message explains why.
fn url_elicitation_result(
    tool_call_id: String,
    retry_tool: &str,
    pending: &UrlElicitationPending,
) -> ToolResult {
    let declined = pending.action == ElicitationAction::Decline;
    let error = if declined {
        format!(
            "You declined to open {}, so '{}' did not run.",
            pending.host, pending.tool_name
        )
    } else {
        // Deliberately no "run it again yourself": a host that can pause the
        // turn puts a consent card in front of the user and re-runs the tool
        // itself, and one that cannot still gets an actionable URL here.
        format!(
            "{} This needs a person to open {} before '{}' can run.",
            pending.message, pending.url, pending.tool_name
        )
    };
    let payload = UrlElicitationRequired {
        code: URL_ELICITATION_REQUIRED_CODE.to_string(),
        error,
        url: pending.url.clone(),
        url_host: pending.host.clone(),
        // Internationalized domains are legitimate, but a client should warn
        // before a user trusts one.
        url_is_punycode: pending.punycode,
        server: pending.server_name.clone(),
        tool: pending.tool_name.clone(),
        retry_tool: retry_tool.to_string(),
        message: pending.message.clone(),
        declined,
    };
    ToolResult {
        tool_call_id,
        result: Some(serde_json::to_value(&payload).unwrap_or(serde_json::Value::Null)),
        images: None,
        // Structured, expected state — not a transport failure.
        error: None,
        connection_required: None,
        raw_output: None,
    }
}

/// Turn a pending form elicitation into the tool result the engine parks on.
fn form_elicitation_result(
    tool_call_id: String,
    retry_tool: &str,
    pending: &FormElicitationPending,
) -> ToolResult {
    let payload = FormElicitationRequired {
        code: FORM_ELICITATION_REQUIRED_CODE.to_string(),
        // Read by the model only when no surface can show the questions, so it
        // says why the tool did not run without repeating the server's text.
        error: format!(
            "MCP server '{}' has {} question(s) that a person must answer before '{}' can run.",
            pending.server_name,
            pending.questions.len(),
            pending.tool_name
        ),
        server: pending.server_name.clone(),
        tool: pending.tool_name.clone(),
        retry_tool: retry_tool.to_string(),
        message: pending.message.clone(),
        questions: pending.questions.clone(),
        fingerprint: pending.fingerprint.clone(),
    };
    ToolResult {
        tool_call_id,
        result: Some(serde_json::to_value(&payload).unwrap_or(serde_json::Value::Null)),
        images: None,
        error: None,
        connection_required: None,
        raw_output: None,
    }
}

#[async_trait]
impl McpToolInvoker for McpExecutor {
    fn for_execution(&self, id: uuid::Uuid) -> Option<Arc<dyn McpToolInvoker>> {
        self.resolver.for_execution(id).map(|resolver| {
            Arc::new(Self::new(self.client.clone(), resolver)) as Arc<dyn McpToolInvoker>
        })
    }

    async fn invoke(&self, tool_call: &ToolCall) -> CoreResult<ToolResult> {
        self.invoke_recorded(tool_call)
            .await
            .map(|(result, _)| result)
    }

    async fn invoke_recorded(
        &self,
        tool_call: &ToolCall,
    ) -> CoreResult<(ToolResult, Option<crate::McpServerActsAs>)> {
        self.execute_mcp_tool_recorded(tool_call)
            .await
            .map_err(|e| {
                tracing::error!(error = %e, "MCP tool execution failed");
                AgentLoopError::tool(e.to_string())
            })
    }

    /// Lists through the connection a call would use, so the tools and the
    /// account match what the turn would have listed had the server not been
    /// deferred. Not cached: the reveal the caller writes moves the server onto
    /// the cached discovery path from the next step.
    async fn list_server_tools(
        &self,
        server_prefix: &str,
        session_id: uuid::Uuid,
    ) -> CoreResult<Option<McpServerTools>> {
        let connection = self
            .resolver
            .resolve(server_prefix)
            .await
            .map_err(|e| AgentLoopError::tool(e.to_string()))?
            .ok_or_else(|| {
                AgentLoopError::tool(format!("MCP server not found for prefix: {server_prefix}"))
            })?;
        if let Some(required) = &connection.pending_oauth_provider {
            return Ok(Some(McpServerTools::ConnectionRequired(
                connection_required_result(String::new(), &connection, required),
            )));
        }
        let tools = self.client.discover(&connection).await.map_err(|e| {
            tracing::warn!(server = %connection.name, error = %e, "MCP tool listing failed");
            AgentLoopError::tool(e.to_string())
        })?;
        // Same capability id the turn's own discovery gives this server.
        let id = uuid::Uuid::new_v5(&session_id, connection.name.as_bytes());
        let definitions =
            crate::mcp::McpCapability::new(id, connection.name, None, tools).tool_definitions();
        Ok(Some(McpServerTools::Listed(definitions)))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::mcp::NoAuthProvider;

    fn executor(connection: McpConnection) -> McpExecutor {
        let client = Arc::new(McpClient::new(
            Arc::new(crate::DisabledEgressService),
            Arc::new(NoAuthProvider),
        ));
        let resolver = StaticConnectionResolver::new().with(connection);
        McpExecutor::new(client, Arc::new(resolver))
    }

    #[tokio::test]
    async fn listing_a_server_without_its_grant_asks_for_the_connection() {
        let mut connection = McpConnection::http("docs", "https://example.com/mcp");
        connection.pending_oauth_provider = Some(
            everruns_contracts::ConnectionRequired::provider_only("docs-provider"),
        );
        let listing = executor(connection)
            .list_server_tools("docs", uuid::Uuid::nil())
            .await
            .unwrap();
        let Some(McpServerTools::ConnectionRequired(result)) = listing else {
            panic!("expected a connection request, got {listing:?}");
        };
        assert!(
            result.error.unwrap_or_default().contains("docs"),
            "names the server"
        );
    }

    #[tokio::test]
    async fn listing_fails_for_a_server_outside_the_resolver_or_unreachable() {
        let executor = executor(McpConnection::http("docs", "https://example.com/mcp"));
        assert!(
            executor
                .list_server_tools("other", uuid::Uuid::nil())
                .await
                .is_err()
        );
        // Egress is disabled here, so the listing itself fails as a tool error.
        assert!(
            executor
                .list_server_tools("docs", uuid::Uuid::nil())
                .await
                .is_err()
        );
    }
}
