//! Host side of the Everruns MCP extensions
//! ([`everruns_contracts::runtime::mcp_extensions`]): reading an injected
//! capability server's extensions and facts, and running its tool hooks.
//!
//! Decisions:
//! - A server that does not serve [`EXTENSIONS_RESOURCE_URI`] speaks no
//!   extension. That is the normal answer from a plain MCP server, not an
//!   error.
//! - A `beforeTool` hook that cannot be reached blocks the call (fail closed):
//!   a server subscribes to a tool to gate it, and running the tool unchecked
//!   would skip that gate. An `afterTool` hook that cannot be reached leaves
//!   the result as it was and logs, because the tool already ran.

use crate::mcp::client::McpClient;
use crate::mcp::http::McpRpcError;
use crate::mcp::transport::McpConnection;
use crate::tool_context::ToolContext;
use crate::tool_hooks::{PostToolExecHook, PreToolUseDecision, PreToolUseHook};
use anyhow::{Result, anyhow};
use async_trait::async_trait;
use everruns_contracts::runtime::capabilities::Fact;
use everruns_contracts::runtime::mcp_extensions::{
    AfterToolParams, AfterToolResult, BeforeToolParams, BeforeToolResult, EXTENSIONS_RESOURCE_URI,
    FACTS_EXTENSION, FACTS_RESOURCE_URI, FactsDocument, HOOKS_AFTER_TOOL_METHOD,
    HOOKS_BEFORE_TOOL_METHOD, HooksDeclaration, ServerExtensions,
};
use everruns_contracts::tool_types::{ToolCall, ToolDefinition, ToolResult};
use serde_json::{Value, json};
use std::sync::Arc;

/// JSON-RPC codes a server answers with when it has no such resource or
/// method: `-32002` (resource not found) and `-32601` (method not found).
fn is_absent(error: &anyhow::Error) -> bool {
    error.chain().any(|cause| {
        cause
            .downcast_ref::<McpRpcError>()
            .is_some_and(|rpc| rpc.code == -32002 || rpc.code == -32601 || rpc.code == -32602)
    })
}

fn first_text(result: crate::McpReadResourceResult) -> Option<String> {
    result.contents.into_iter().find_map(|content| content.text)
}

impl McpClient {
    /// The Everruns extensions this server speaks. Empty for a server that
    /// serves no [`EXTENSIONS_RESOURCE_URI`].
    pub async fn server_extensions(&self, connection: &McpConnection) -> Result<ServerExtensions> {
        match self
            .read_resource(connection, EXTENSIONS_RESOURCE_URI)
            .await
        {
            Ok(result) => match first_text(result) {
                Some(text) => Ok(serde_json::from_str(&text)
                    .map_err(|e| anyhow!("invalid extensions resource: {e}"))?),
                None => Ok(ServerExtensions::default()),
            },
            Err(error) if is_absent(&error) => Ok(ServerExtensions::default()),
            Err(error) => Err(error),
        }
    }

    /// The server's facts for this turn, capped as
    /// [`FactsDocument::into_facts`] describes.
    pub async fn read_facts(&self, connection: &McpConnection) -> Result<Vec<Fact>> {
        match self.read_resource(connection, FACTS_RESOURCE_URI).await {
            Ok(result) => match first_text(result) {
                Some(text) => Ok(serde_json::from_str::<FactsDocument>(&text)
                    .map_err(|e| anyhow!("invalid facts resource: {e}"))?
                    .into_facts()),
                None => Ok(Vec::new()),
            },
            Err(error) if is_absent(&error) => Ok(Vec::new()),
            Err(error) => Err(error),
        }
    }

    /// Ask the server about a call before it runs.
    pub async fn before_tool(
        &self,
        connection: &McpConnection,
        params: &BeforeToolParams,
    ) -> Result<BeforeToolResult> {
        let result = self
            .request_raw(
                connection,
                HOOKS_BEFORE_TOOL_METHOD,
                serde_json::to_value(params)?,
            )
            .await?;
        Ok(serde_json::from_value(result)?)
    }

    /// Tell the server a call ran, and take its changes to the result.
    pub async fn after_tool(
        &self,
        connection: &McpConnection,
        params: &AfterToolParams,
    ) -> Result<AfterToolResult> {
        let result = self
            .request_raw(
                connection,
                HOOKS_AFTER_TOOL_METHOD,
                serde_json::to_value(params)?,
            )
            .await?;
        Ok(serde_json::from_value(result)?)
    }
}

/// What a host learned from one injected server at turn start.
#[derive(Debug, Clone, Default)]
pub struct InjectedServerState {
    pub extensions: ServerExtensions,
    pub facts: Vec<Fact>,
}

/// Read a server's extensions and, when it speaks facts, its facts.
pub async fn load_injected_server_state(
    client: &McpClient,
    connection: &McpConnection,
) -> Result<InjectedServerState> {
    let extensions = client.server_extensions(connection).await?;
    let facts = if extensions.has(FACTS_EXTENSION) {
        client.read_facts(connection).await?
    } else {
        Vec::new()
    };
    Ok(InjectedServerState { extensions, facts })
}

/// Runs one injected server's tool hooks. Register the same value as both a
/// [`PreToolUseHook`] and a [`PostToolExecHook`]; each side does nothing for
/// tools the server did not subscribe to.
pub struct InjectedToolHooks {
    client: Arc<McpClient>,
    connection: McpConnection,
    declaration: HooksDeclaration,
}

impl InjectedToolHooks {
    /// `None` when the server subscribed to no tools.
    pub fn new(
        client: Arc<McpClient>,
        connection: McpConnection,
        declaration: HooksDeclaration,
    ) -> Option<Self> {
        (!declaration.is_empty()).then_some(Self {
            client,
            connection,
            declaration,
        })
    }
}

#[async_trait]
impl PreToolUseHook for InjectedToolHooks {
    async fn before_exec(
        &self,
        mut tool_call: ToolCall,
        _tool_def: &ToolDefinition,
        _context: &ToolContext,
    ) -> PreToolUseDecision {
        if !self.declaration.wants_before(&tool_call.name) {
            return PreToolUseDecision::Continue(tool_call);
        }
        let params = BeforeToolParams {
            tool_call_id: tool_call.id.clone(),
            tool: tool_call.name.clone(),
            arguments: tool_call.arguments.clone(),
        };
        match self.client.before_tool(&self.connection, &params).await {
            Ok(BeforeToolResult::Allow { arguments }) => {
                if let Some(arguments) = arguments {
                    tool_call.arguments = arguments;
                }
                PreToolUseDecision::Continue(tool_call)
            }
            Ok(BeforeToolResult::Deny {
                reason,
                user_message,
            }) => PreToolUseDecision::Block {
                tool_call,
                reason,
                user_message,
            },
            Err(error) => {
                tracing::warn!(
                    server = %self.connection.name,
                    tool = %tool_call.name,
                    error = %error,
                    "injected beforeTool hook failed; blocking the call"
                );
                PreToolUseDecision::Block {
                    reason: format!(
                        "{} could not check this call, so it did not run",
                        self.connection.name
                    ),
                    tool_call,
                    user_message: None,
                }
            }
        }
    }
}

#[async_trait]
impl PostToolExecHook for InjectedToolHooks {
    async fn after_exec(
        &self,
        tool_call: &ToolCall,
        _tool_def: &ToolDefinition,
        result: &mut ToolResult,
        _context: &ToolContext,
    ) {
        if !self.declaration.wants_after(&tool_call.name) {
            return;
        }
        let params = AfterToolParams {
            tool_call_id: tool_call.id.clone(),
            tool: tool_call.name.clone(),
            arguments: tool_call.arguments.clone(),
            result: result.result.clone(),
            error: result.error.clone(),
        };
        match self.client.after_tool(&self.connection, &params).await {
            Ok(change) => apply_after_tool(result, change),
            Err(error) => tracing::warn!(
                server = %self.connection.name,
                tool = %tool_call.name,
                error = %error,
                "injected afterTool hook failed; result left as it was"
            ),
        }
    }
}

/// Apply a server's `afterTool` answer, the same way user hooks patch results
/// (`hook_context` beside the tool's own output).
pub fn apply_after_tool(result: &mut ToolResult, change: AfterToolResult) {
    if let Some(replacement) = change.result {
        result.result = Some(replacement);
    }
    if let Some(error) = change.error {
        result.error = Some(error);
    }
    if let Some(context) = change.additional_context {
        match result.result.as_mut() {
            Some(Value::Object(object)) => {
                object.insert("hook_context".to_string(), json!(context));
            }
            Some(other) => {
                let original = other.take();
                *other = json!({ "result": original, "hook_context": context });
            }
            None => result.result = Some(json!({ "hook_context": context })),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn result(value: Option<Value>) -> ToolResult {
        ToolResult {
            tool_call_id: "c".into(),
            result: value,
            images: None,
            error: None,
            connection_required: None,
            raw_output: None,
            details: None,
        }
    }

    #[test]
    fn after_tool_patches_result_error_and_context() {
        let mut object = result(Some(json!({"rows": 1})));
        apply_after_tool(
            &mut object,
            AfterToolResult {
                additional_context: Some("cached".into()),
                ..Default::default()
            },
        );
        assert_eq!(
            object.result,
            Some(json!({"rows": 1, "hook_context": "cached"}))
        );

        let mut scalar = result(Some(json!("ok")));
        apply_after_tool(
            &mut scalar,
            AfterToolResult {
                additional_context: Some("note".into()),
                ..Default::default()
            },
        );
        assert_eq!(
            scalar.result,
            Some(json!({"result": "ok", "hook_context": "note"}))
        );

        let mut replaced = result(Some(json!("secret")));
        apply_after_tool(
            &mut replaced,
            AfterToolResult {
                result: Some(json!("[redacted]")),
                error: Some("policy".into()),
                additional_context: None,
            },
        );
        assert_eq!(replaced.result, Some(json!("[redacted]")));
        assert_eq!(replaced.error.as_deref(), Some("policy"));

        let mut unchanged = result(Some(json!(1)));
        apply_after_tool(&mut unchanged, AfterToolResult::default());
        assert_eq!(unchanged.result, Some(json!(1)));
    }

    #[test]
    fn hooks_need_a_subscription() {
        let client = Arc::new(McpClient::new(
            Arc::new(crate::DisabledEgressService),
            Arc::new(crate::mcp::NoAuthProvider),
        ));
        let connection = McpConnection::http("knowledge", "https://cp.example/mcp");
        assert!(
            InjectedToolHooks::new(
                client.clone(),
                connection.clone(),
                HooksDeclaration::default()
            )
            .is_none()
        );
        assert!(
            InjectedToolHooks::new(
                client,
                connection,
                HooksDeclaration {
                    before_tool: vec!["*".into()],
                    after_tool: Vec::new()
                }
            )
            .is_some()
        );
    }
}
