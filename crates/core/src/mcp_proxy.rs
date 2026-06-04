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
use crate::tool_types::{BuiltinTool, ToolCall, ToolDefinition, ToolHints};
use crate::tools::{Tool, ToolExecutionResult};
use crate::traits::ToolContext;
use async_trait::async_trait;
use serde_json::Value;
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
            // Transport/connection failures are internal: hide details from the
            // model but log them upstream (the act loop logs InternalError).
            Err(error) => ToolExecutionResult::internal_error_msg(error.to_string()),
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
    use crate::tool_types::{DeferrablePolicy, ToolPolicy, ToolResult};
    use std::sync::Mutex;

    fn mcp_def(name: &str) -> ToolDefinition {
        ToolDefinition::Builtin(BuiltinTool {
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

    #[test]
    fn build_proxies_only_for_mcp_tools() {
        let defs = vec![
            mcp_def("mcp_docs__search"),
            ToolDefinition::Builtin(BuiltinTool {
                name: "read_file".to_string(),
                display_name: None,
                description: "read".to_string(),
                parameters: serde_json::json!({}),
                policy: ToolPolicy::Auto,
                category: None,
                deferrable: DeferrablePolicy::Automatic,
                hints: ToolHints::default(),
            }),
        ];
        let invoker = Arc::new(RecordingInvoker {
            calls: Mutex::new(vec![]),
            result: ok_result(serde_json::json!({})),
        });
        let tools = build_mcp_proxy_tools(&defs, invoker);
        assert_eq!(tools.len(), 1);
        assert_eq!(tools[0].name(), "mcp_docs__search");
        // Schema is preserved for search/introspection.
        assert!(tools[0].parameters_schema()["properties"]["q"].is_object());
    }

    #[tokio::test]
    async fn proxy_delegates_to_invoker_and_maps_success() {
        let invoker = Arc::new(RecordingInvoker {
            calls: Mutex::new(vec![]),
            result: ok_result(serde_json::json!({ "answer": 42 })),
        });
        let tool = McpProxyTool::new(
            match mcp_def("mcp_docs__search") {
                ToolDefinition::Builtin(b) => b,
                _ => unreachable!(),
            },
            invoker.clone(),
        );

        let mut ctx = ToolContext::new(uuid::Uuid::new_v4().into());
        ctx.tool_call_id = Some("call_1".to_string());
        let result = tool
            .execute_with_context(serde_json::json!({ "q": "hi" }), &ctx)
            .await;

        match result {
            ToolExecutionResult::Success(v) => assert_eq!(v["answer"], 42),
            other => panic!("expected success, got {other:?}"),
        }
        let calls = invoker.calls.lock().unwrap();
        assert_eq!(calls.len(), 1);
        assert_eq!(calls[0].name, "mcp_docs__search");
        assert_eq!(calls[0].id, "call_1");
        assert_eq!(calls[0].arguments["q"], "hi");
    }

    #[tokio::test]
    async fn proxy_maps_tool_error() {
        let invoker = Arc::new(RecordingInvoker {
            calls: Mutex::new(vec![]),
            result: ToolResult {
                tool_call_id: String::new(),
                result: Some(serde_json::json!({ "error": "boom" })),
                images: None,
                error: Some("boom".to_string()),
                connection_required: None,
                raw_output: None,
            },
        });
        let tool = McpProxyTool::new(
            match mcp_def("mcp_docs__search") {
                ToolDefinition::Builtin(b) => b,
                _ => unreachable!(),
            },
            invoker,
        );
        let result = tool.execute(serde_json::json!({})).await;
        assert!(matches!(result, ToolExecutionResult::ToolError(ref m) if m == "boom"));
    }

    #[test]
    fn mcp_tool_is_first_class_in_registry() {
        // The whole point: once registered, an MCP tool is indistinguishable
        // from any other tool to registry introspection (what tool_search,
        // spawn_background, and openai_tool_search namespacing read).
        use crate::tools::ToolRegistry;

        let invoker = Arc::new(RecordingInvoker {
            calls: Mutex::new(vec![]),
            result: ok_result(serde_json::json!({})),
        });
        let defs = vec![mcp_def("mcp_docs__search")];

        let mut registry = ToolRegistry::new();
        for tool in build_mcp_proxy_tools(&defs, invoker) {
            registry.register_boxed(tool);
        }

        assert!(registry.has("mcp_docs__search"));
        let registry_defs = registry.tool_definitions();
        let found = registry_defs
            .iter()
            .find(|d| d.name() == "mcp_docs__search")
            .expect("MCP tool visible via registry introspection");
        // Full parameter schema is exposed (so tool_search can return it).
        assert!(found.parameters()["properties"]["q"].is_object());
    }
}
