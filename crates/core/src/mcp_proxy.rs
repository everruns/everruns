// MCP tool proxy: make MCP server tools first-class registry tools.
//
// MCP servers contribute tool *definitions* (names, descriptions, schemas) to
// the agent, but historically their execution was routed separately (a host
// `CompositeToolExecutor` intercepted `mcp_*` calls). That meant MCP tools were
// invisible to anything that introspects the `ToolRegistry` — `spawn_background`,
// `tool_search`, openai_tool_search namespaces, etc. — and could not be deferred
// or searched.
//
// This module closes that gap. An [`McpProxyTool`] is a real [`Tool`] that wraps
// an MCP tool definition and delegates execution to an [`McpToolInvoker`] (the
// host's MCP client). Hosts register these into the regular `ToolRegistry`, so
// MCP tools behave like any other tool everywhere: discovery, scheduling,
// deferral, and search all work transparently.

use crate::error::Result;
use crate::mcp_server::is_mcp_tool;
use crate::tool_context::ToolContext;
use crate::tool_types::{BuiltinTool, ToolCall, ToolDefinition, ToolHints};
use crate::tools::{Tool, ToolExecutionResult};
use async_trait::async_trait;
use serde_json::Value;
use std::collections::HashSet;
use std::sync::Arc;

/// Host-provided backend that executes an MCP tool call against the right
/// server. Implemented in `everruns-mcp` over the shared MCP client; the
/// implementation owns connection resolution and credentials so the proxy tool
/// stays host-agnostic.
#[async_trait]
pub trait McpToolInvoker: Send + Sync {
    /// Execute a single MCP tool call (its `name` is the prefixed `mcp_*` name)
    /// and return the raw tool result.
    async fn invoke(&self, tool_call: &ToolCall) -> Result<crate::tool_types::ToolResult>;
}

/// MCP invoker wrapper that only permits calls to MCP tools included in the
/// current turn's tool definitions. Guardrails use this wrapper because their
/// configured `server`/`tool` references are out-of-band; without this check a
/// config edit could invoke an org MCP server that was not scoped to the
/// current agent/session.
pub struct ScopedMcpToolInvoker {
    inner: Arc<dyn McpToolInvoker>,
    allowed_tool_names: HashSet<String>,
}

impl ScopedMcpToolInvoker {
    pub fn new(definitions: &[ToolDefinition], inner: Arc<dyn McpToolInvoker>) -> Self {
        let allowed_tool_names = definitions
            .iter()
            .filter_map(|def| match def {
                ToolDefinition::Builtin(builtin) if is_mcp_tool(&builtin.name) => {
                    Some(builtin.name.clone())
                }
                // Client-side tools are session/agent-authored metadata and are
                // not worker-executable MCP proxy tools, even if their names
                // use the reserved mcp_* prefix.
                ToolDefinition::ClientSide(_) | ToolDefinition::Builtin(_) => None,
            })
            .collect();
        Self {
            inner,
            allowed_tool_names,
        }
    }
}

#[async_trait]
impl McpToolInvoker for ScopedMcpToolInvoker {
    async fn invoke(&self, tool_call: &ToolCall) -> Result<crate::tool_types::ToolResult> {
        if !self.allowed_tool_names.contains(&tool_call.name) {
            return Err(crate::AgentLoopError::tool(format!(
                "MCP tool '{}' is not allowed in the current tool scope",
                tool_call.name
            )));
        }
        self.inner.invoke(tool_call).await
    }
}

/// A registry [`Tool`] backed by an MCP server tool definition.
///
/// Holds the tool's definition (so `to_definition()`, scheduling hints, and
/// schema introspection match the non-MCP tools) and delegates execution to the
/// shared [`McpToolInvoker`].
pub struct McpProxyTool {
    definition: BuiltinTool,
    invoker: Arc<dyn McpToolInvoker>,
}

impl McpProxyTool {
    /// Build a proxy from a builtin tool definition (already `mcp_*`-prefixed by
    /// `McpCapability`) and the shared invoker.
    pub fn new(definition: BuiltinTool, invoker: Arc<dyn McpToolInvoker>) -> Self {
        Self {
            definition,
            invoker,
        }
    }

    async fn invoke(&self, tool_call_id: String, arguments: Value) -> ToolExecutionResult {
        let call = ToolCall {
            id: tool_call_id,
            name: self.definition.name.clone(),
            arguments,
        };
        match self.invoker.invoke(&call).await {
            Ok(result) => tool_result_to_execution(result),
            // Surface MCP failures to the model as a tool error (matching the
            // prior executor-based routing), so it sees actionable messages like
            // "MCP server not found" and can refine or recover. The invoker maps
            // transport errors to `AgentLoopError::tool` and logs details upstream.
            Err(error) => ToolExecutionResult::tool_error(error.to_string()),
        }
    }
}

#[async_trait]
impl Tool for McpProxyTool {
    fn name(&self) -> &str {
        &self.definition.name
    }

    fn display_name(&self) -> Option<&str> {
        self.definition.display_name.as_deref()
    }

    fn description(&self) -> &str {
        &self.definition.description
    }

    fn parameters_schema(&self) -> Value {
        self.definition.parameters.clone()
    }

    fn hints(&self) -> ToolHints {
        self.definition.hints.clone()
    }

    fn requires_context(&self) -> bool {
        true
    }

    fn to_definition(&self) -> ToolDefinition {
        ToolDefinition::Builtin(self.definition.clone())
    }

    async fn execute(&self, arguments: Value) -> ToolExecutionResult {
        self.invoke(String::new(), arguments).await
    }

    async fn execute_with_context(
        &self,
        arguments: Value,
        context: &ToolContext,
    ) -> ToolExecutionResult {
        let tool_call_id = context.tool_call_id.clone().unwrap_or_default();
        self.invoke(tool_call_id, arguments).await
    }
}

/// Build proxy tools for every MCP-prefixed definition in `definitions`,
/// delegating execution to `invoker`. Non-MCP definitions are ignored.
///
/// Hosts call this when assembling the per-turn `ToolRegistry`, passing the
/// turn's tool definitions (which already include MCP tools) so no re-discovery
/// is needed.
pub fn build_mcp_proxy_tools(
    definitions: &[ToolDefinition],
    invoker: Arc<dyn McpToolInvoker>,
) -> Vec<Box<dyn Tool>> {
    definitions
        .iter()
        .filter(|def| is_mcp_tool(def.name()))
        .filter_map(|def| match def {
            ToolDefinition::Builtin(builtin) => {
                Some(Box::new(McpProxyTool::new(builtin.clone(), invoker.clone())) as Box<dyn Tool>)
            }
            // MCP capabilities only ever emit Builtin definitions; a ClientSide
            // MCP tool would not be worker-executable, so skip it.
            ToolDefinition::ClientSide(_) => None,
        })
        .collect()
}

/// Map a raw MCP `ToolResult` into the registry's `ToolExecutionResult`.
fn tool_result_to_execution(result: crate::tool_types::ToolResult) -> ToolExecutionResult {
    if let Some(provider) = result.connection_required {
        return ToolExecutionResult::ConnectionRequired { provider };
    }
    if let Some(error) = result.error {
        return ToolExecutionResult::ToolError(error);
    }
    let value = result.result.unwrap_or(Value::Null);
    match result.images {
        Some(images) if !images.is_empty() => ToolExecutionResult::SuccessWithImages {
            result: value,
            images,
        },
        _ => ToolExecutionResult::Success(value),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::tool_types::{ClientSideTool, DeferrablePolicy, ToolPolicy, ToolResult};
    use std::sync::Mutex;

    fn builtin_def(name: &str) -> BuiltinTool {
        BuiltinTool {
            name: name.to_string(),
            display_name: None,
            description: "an mcp tool".to_string(),
            parameters: serde_json::json!({
                "type": "object",
                "properties": { "q": { "type": "string" } }
            }),
            policy: ToolPolicy::Auto,
            category: Some("MCP Servers".to_string()),
            deferrable: DeferrablePolicy::Automatic,
            hints: ToolHints::default().with_open_world(true),
            full_parameters: None,
        }
    }

    fn mcp_def(name: &str) -> ToolDefinition {
        ToolDefinition::Builtin(builtin_def(name))
    }

    fn client_side_mcp_def(name: &str) -> ToolDefinition {
        ToolDefinition::ClientSide(ClientSideTool {
            name: name.to_string(),
            display_name: None,
            description: "an mcp tool".to_string(),
            parameters: serde_json::json!({
                "type": "object",
                "properties": { "q": { "type": "string" } }
            }),
            category: Some("MCP Servers".to_string()),
            deferrable: DeferrablePolicy::Automatic,
            hints: ToolHints::default().with_open_world(true),
            full_parameters: None,
        })
    }

    /// Records the calls it receives and returns a canned result.
    struct RecordingInvoker {
        calls: Mutex<Vec<ToolCall>>,
        result: ToolResult,
    }

    #[async_trait]
    impl McpToolInvoker for RecordingInvoker {
        async fn invoke(&self, tool_call: &ToolCall) -> Result<ToolResult> {
            self.calls.lock().unwrap().push(tool_call.clone());
            Ok(self.result.clone())
        }
    }

    fn ok_result(value: Value) -> ToolResult {
        ToolResult {
            tool_call_id: String::new(),
            result: Some(value),
            images: None,
            error: None,
            connection_required: None,
            raw_output: None,
        }
    }

    #[tokio::test]
    async fn registry_preserves_complete_mcp_definition_and_executes_context() {
        let mut definition = builtin_def("mcp_docs__search");
        definition.display_name = Some("Search docs".into());
        definition.full_parameters = Some(
            serde_json::json!({"type":"object","properties":{"q":{"type":"string","description":"query"}}}),
        );
        definition.deferrable = DeferrablePolicy::Always;
        let expected = ToolDefinition::Builtin(definition.clone());
        let definitions = [
            expected.clone(),
            mcp_def("read_file"),
            client_side_mcp_def("mcp_secret__capture"),
        ];
        let invoker = Arc::new(RecordingInvoker {
            calls: Mutex::new(vec![]),
            result: ok_result(serde_json::json!({"answer":42,"nested":[true,null]})),
        });
        let mut registry = crate::tools::ToolRegistry::new();
        for tool in build_mcp_proxy_tools(&definitions, invoker.clone()) {
            registry.register_boxed(tool);
        }
        assert_eq!(registry.len(), 1);
        assert_eq!(
            serde_json::to_value(registry.tool_definitions()).unwrap(),
            serde_json::json!([expected])
        );
        let tool = registry.get("mcp_docs__search").unwrap();
        assert_eq!(tool.display_name(), Some("Search docs"));
        assert_eq!(tool.description(), "an mcp tool");
        assert_eq!(tool.parameters_schema(), definition.parameters);
        assert_eq!(tool.hints(), definition.hints);
        assert!(tool.requires_context());
        let mut context = ToolContext::new(crate::typed_id::SessionId::from_seed(1));
        context.tool_call_id = Some("call-1".into());
        let arguments = serde_json::json!({"q":"query","nested":{"exact":true}});
        for result in [
            tool.execute_with_context(arguments.clone(), &context).await,
            tool.execute(arguments.clone()).await,
        ] {
            match result {
                ToolExecutionResult::Success(value) => {
                    assert_eq!(value, serde_json::json!({"answer":42,"nested":[true,null]}))
                }
                other => panic!("{other:?}"),
            }
        }
        assert_eq!(
            serde_json::to_value(&*invoker.calls.lock().unwrap()).unwrap(),
            serde_json::json!([
                {"id":"call-1","name":"mcp_docs__search","arguments":arguments},
                {"id":"","name":"mcp_docs__search","arguments":arguments}
            ])
        );
    }

    #[tokio::test]
    async fn proxy_result_mapping_preserves_images_and_connection_error_precedence() {
        let image = crate::tool_types::ToolResultImage {
            media_type: "image/png".into(),
            base64: "YWJj+/==".into(),
        };
        for (connection, error, images, expected) in [
            (
                Some("github"),
                Some("boom"),
                Some(vec![image.clone()]),
                "connection",
            ),
            (None, Some("boom"), Some(vec![image.clone()]), "error"),
            (None, None, Some(vec![image.clone()]), "images"),
            (None, None, Some(vec![]), "success"),
            (None, None, None, "success"),
        ] {
            let mut raw = ok_result(serde_json::Value::Null);
            raw.result = None;
            raw.connection_required = connection.map(str::to_string);
            raw.error = error.map(str::to_string);
            raw.images = images;
            let tool = McpProxyTool::new(
                builtin_def("mcp_docs__search"),
                Arc::new(RecordingInvoker {
                    calls: Mutex::new(vec![]),
                    result: raw,
                }),
            );
            match (expected, tool.execute(serde_json::json!({})).await) {
                ("connection", ToolExecutionResult::ConnectionRequired { provider }) => {
                    assert_eq!(provider, "github")
                }
                ("error", ToolExecutionResult::ToolError(message)) => assert_eq!(message, "boom"),
                ("images", ToolExecutionResult::SuccessWithImages { result, images }) => {
                    assert_eq!(result, serde_json::Value::Null);
                    assert_eq!(
                        serde_json::to_value(images).unwrap(),
                        serde_json::json!([{"media_type":"image/png","base64":"YWJj+/=="}])
                    );
                }
                ("success", ToolExecutionResult::Success(value)) => {
                    assert_eq!(value, serde_json::Value::Null)
                }
                other => panic!("unexpected mapping: {other:?}"),
            }
        }
    }

    #[tokio::test]
    async fn proxy_maps_invoker_error_to_tool_error() {
        struct FailingInvoker;
        #[async_trait]
        impl McpToolInvoker for FailingInvoker {
            async fn invoke(&self, _call: &ToolCall) -> Result<ToolResult> {
                Err(crate::AgentLoopError::tool(
                    "MCP server not found for prefix: docs",
                ))
            }
        }
        let tool = McpProxyTool::new(builtin_def("mcp_docs__search"), Arc::new(FailingInvoker));
        match tool.execute(serde_json::json!({})).await {
            ToolExecutionResult::ToolError(message) => assert_eq!(
                message,
                "Tool execution error: MCP server not found for prefix: docs"
            ),
            other => panic!("{other:?}"),
        }
    }

    #[tokio::test]
    async fn scoped_invoker_rejects_unlisted_non_mcp_and_client_side_tools_before_backend() {
        let expected = ok_result(serde_json::json!({"ok":true,"data":[1,2]}));
        let inner = Arc::new(RecordingInvoker {
            calls: Mutex::new(vec![]),
            result: expected.clone(),
        });
        let scoped = ScopedMcpToolInvoker::new(
            &[
                mcp_def("mcp_docs__search"),
                mcp_def("read_file"),
                client_side_mcp_def("mcp_secret__capture"),
            ],
            inner.clone(),
        );
        for name in [
            "mcp_other__search",
            "read_file",
            "mcp_secret__capture",
            "mcp_docs__search_extra",
        ] {
            let call = ToolCall {
                id: "denied".into(),
                name: name.into(),
                arguments: serde_json::json!({"q":"private"}),
            };
            assert_eq!(
                scoped.invoke(&call).await.unwrap_err().to_string(),
                format!(
                    "Tool execution error: MCP tool '{name}' is not allowed in the current tool scope"
                )
            );
        }
        assert!(inner.calls.lock().unwrap().is_empty());
        let allowed = ToolCall {
            id: "allowed".into(),
            name: "mcp_docs__search".into(),
            arguments: serde_json::json!({"q":"exact","nested":[true]}),
        };
        let result = scoped.invoke(&allowed).await.unwrap();
        assert_eq!(
            serde_json::to_value(result).unwrap(),
            serde_json::to_value(expected).unwrap()
        );
        assert_eq!(
            serde_json::to_value(&*inner.calls.lock().unwrap()).unwrap(),
            serde_json::json!([allowed])
        );
    }
}
