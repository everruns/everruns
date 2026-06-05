// Generic (provider-agnostic) Tool Search Capability
//
// Brings deferred tool loading to models that have no native tool_search
// support (Anthropic, Gemini, OpenAI Completions, ...). Unlike
// `openai_tool_search`, which relies on the OpenAI Responses API to hide
// parameter schemas server-side, this capability implements tool search
// entirely client-side and therefore works with any provider.
//
// How it works:
//   1. A `tool_definition_hook` (`DeferSchemaHook`) runs at runtime-agent build
//      time. When the agent carries at least `threshold` tools, it replaces the
//      parameter schema of every deferrable tool with a minimal stub. Only the
//      name + description survive, so the model still sees that the tool exists
//      but pays no token cost for its parameters. Tools marked
//      `DeferrablePolicy::Never` (e.g. high-frequency tools) keep full schemas.
//   2. A real `tool_search` tool is added to the registry. When the model calls
//      it, the tool inspects its sibling tools via `ToolContext::tool_registry`
//      (the same mechanism `spawn_background` uses) and returns the full
//      parameter schemas of the tools matching the query.
//   3. A short system-prompt note tells the model to call `tool_search` before
//      using a tool whose parameters it has not loaded yet.
//
// Because the underlying tools stay registered and executable, tool calls and
// results work exactly as before — the only difference is how schemas reach the
// model. No driver or agent-loop changes are required.

use super::{Capability, CapabilityStatus, ToolDefinitionHook};
use crate::mcp_server::is_mcp_tool;
use crate::tool_types::{DeferrablePolicy, ToolDefinition, ToolHints};
use crate::tools::{Tool, ToolExecutionResult};
use crate::traits::ToolContext;
use async_trait::async_trait;
use serde_json::{Value, json};
use std::sync::Arc;

pub use super::openai_tool_search::DEFAULT_TOOL_SEARCH_THRESHOLD;

/// Capability ID for the generic (provider-agnostic) tool search.
pub const TOOL_SEARCH_CAPABILITY_ID: &str = "tool_search";

/// Name of the tool the model calls to load deferred schemas.
pub const TOOL_SEARCH_TOOL_NAME: &str = "tool_search";

/// Maximum number of tools returned by a single `tool_search` call.
const MAX_SEARCH_RESULTS: usize = 12;

const SYSTEM_PROMPT: &str = "Many of your tools are loaded lazily to save context: \
you can see their names and descriptions, but their parameter schemas are hidden \
until you ask for them. Before calling a tool whose parameters you have not yet \
loaded, call `tool_search` with a short query describing what you need (for example \
\"read file\" or \"send email\"). It returns the matching tools with their full JSON \
parameter schemas. Then call the tool with correct arguments. Frequently used tools \
keep their full schemas and do not need to be searched for.";

/// Generic Tool Search capability.
///
/// Adding this capability enables client-side deferred tool loading for any
/// model. `threshold` controls the minimum number of tools before schemas are
/// deferred (default: [`DEFAULT_TOOL_SEARCH_THRESHOLD`]).
pub struct GenericToolSearchCapability {
    threshold: usize,
}

impl GenericToolSearchCapability {
    pub fn new() -> Self {
        Self {
            threshold: DEFAULT_TOOL_SEARCH_THRESHOLD,
        }
    }

    pub fn with_threshold(threshold: usize) -> Self {
        Self { threshold }
    }
}

impl Default for GenericToolSearchCapability {
    fn default() -> Self {
        Self::new()
    }
}

impl Capability for GenericToolSearchCapability {
    fn id(&self) -> &str {
        TOOL_SEARCH_CAPABILITY_ID
    }

    fn name(&self) -> &str {
        "Tool Search"
    }

    fn description(&self) -> &str {
        "Provider-agnostic deferred tool loading. Hides tool parameter schemas \
         until the model loads them via the tool_search tool, reducing token \
         usage for agents with many tools. Works with any model."
    }

    fn status(&self) -> CapabilityStatus {
        CapabilityStatus::Available
    }

    fn category(&self) -> Option<&str> {
        Some("Optimization")
    }

    fn system_prompt_addition(&self) -> Option<&str> {
        Some(SYSTEM_PROMPT)
    }

    fn tools(&self) -> Vec<Box<dyn Tool>> {
        vec![Box::new(ToolSearchTool)]
    }

    fn tool_definition_hooks(&self) -> Vec<Arc<dyn ToolDefinitionHook>> {
        vec![Arc::new(DeferSchemaHook {
            threshold: self.threshold,
        })]
    }
}

// ============================================================================
// DeferSchemaHook — strips parameter schemas from deferrable tools
// ============================================================================

/// Stub schema sent in place of a deferred tool's real parameters.
///
/// An open object so the provider still accepts the tool definition; the
/// description nudges the model toward `tool_search` if it somehow tries to
/// call the tool before loading the schema.
fn deferred_stub_schema() -> Value {
    json!({
        "type": "object",
        "description": "Parameters hidden to save context. Call tool_search to load the full schema before using this tool.",
    })
}

pub(crate) struct DeferSchemaHook {
    threshold: usize,
}

impl ToolDefinitionHook for DeferSchemaHook {
    fn transform(&self, tools: Vec<ToolDefinition>) -> Vec<ToolDefinition> {
        // Below the threshold full schemas fit comfortably; don't defer.
        if tools.len() < self.threshold {
            return tools;
        }

        tools
            .into_iter()
            .map(|tool| {
                // Keep full schemas for: the search tool itself, tools that opt
                // out, and MCP tools. MCP tools are executed via registry proxies
                // built from these definitions in the act path; stripping them
                // here would leave the proxy (and therefore tool_search results)
                // with only the stub schema. Deferring MCP tools would require
                // plumbing their full schemas to the act path separately.
                if tool.name() == TOOL_SEARCH_TOOL_NAME
                    || matches!(tool.deferrable(), DeferrablePolicy::Never)
                    || is_mcp_tool(tool.name())
                {
                    return tool;
                }
                strip_parameters(tool)
            })
            .collect()
    }

    // Mutually exclusive with hosted (openai) tool_search — see build().
    fn applies_with_native_tool_search(&self) -> bool {
        false
    }
}

/// Replace a tool's parameter schema with the deferred stub, keeping name,
/// description, policy, category, and hints intact.
fn strip_parameters(tool: ToolDefinition) -> ToolDefinition {
    match tool {
        ToolDefinition::Builtin(mut b) => {
            b.parameters = deferred_stub_schema();
            ToolDefinition::Builtin(b)
        }
        ToolDefinition::ClientSide(mut c) => {
            c.parameters = deferred_stub_schema();
            ToolDefinition::ClientSide(c)
        }
    }
}

// ============================================================================
// Tool: tool_search
// ============================================================================

/// Tool that returns full parameter schemas for tools matching a query.
pub struct ToolSearchTool;

impl ToolSearchTool {
    /// Rank `defs` against `query` and return the best matches (full schemas).
    ///
    /// Scoring is a simple keyword overlap: each whitespace-separated query term
    /// that appears in a tool's name or description scores a point. Ties keep
    /// registry order. An empty query lists tools (names + descriptions) so the
    /// model can browse. The search tool itself is always excluded.
    fn search(defs: &[ToolDefinition], query: &str) -> Vec<Value> {
        let terms: Vec<String> = query
            .split_whitespace()
            .map(|t| {
                t.trim_matches(|c: char| !c.is_alphanumeric())
                    .to_lowercase()
            })
            .filter(|t| !t.is_empty())
            .collect();

        let mut scored: Vec<(usize, &ToolDefinition)> = defs
            .iter()
            .filter(|d| d.name() != TOOL_SEARCH_TOOL_NAME)
            .filter_map(|d| {
                if terms.is_empty() {
                    return Some((0, d));
                }
                let haystack = format!("{} {}", d.name(), d.description()).to_lowercase();
                let score = terms.iter().filter(|t| haystack.contains(*t)).count();
                (score > 0).then_some((score, d))
            })
            .collect();

        // Stable sort by descending score; equal scores keep registry order.
        scored.sort_by_key(|entry| std::cmp::Reverse(entry.0));

        scored
            .into_iter()
            .take(MAX_SEARCH_RESULTS)
            .map(|(_, d)| {
                json!({
                    "name": d.name(),
                    "description": d.description(),
                    "parameters": d.parameters(),
                })
            })
            .collect()
    }
}

#[async_trait]
impl Tool for ToolSearchTool {
    fn name(&self) -> &str {
        TOOL_SEARCH_TOOL_NAME
    }

    fn display_name(&self) -> Option<&str> {
        Some("Tool Search")
    }

    fn description(&self) -> &str {
        "Search the available tools by keyword and load their full parameter \
         schemas. Returns matching tools with their names, descriptions, and JSON \
         parameter schemas. Call this before using any tool whose parameters you \
         have not loaded yet."
    }

    fn parameters_schema(&self) -> Value {
        json!({
            "type": "object",
            "properties": {
                "query": {
                    "type": "string",
                    "description": "Keywords describing the tool or capability you need (e.g. 'read file', 'run sql', 'send message')."
                }
            },
            "required": ["query"],
            "additionalProperties": false
        })
    }

    fn hints(&self) -> ToolHints {
        ToolHints::default()
            .with_readonly(true)
            .with_idempotent(true)
    }

    // Never defer the search tool's own schema.
    fn to_definition(&self) -> ToolDefinition {
        ToolDefinition::Builtin(crate::tool_types::BuiltinTool {
            name: self.name().to_string(),
            display_name: self.display_name().map(str::to_string),
            description: self.description().to_string(),
            parameters: self.parameters_schema(),
            policy: self.policy(),
            category: None,
            deferrable: DeferrablePolicy::Never,
            hints: self.hints(),
        })
    }

    fn requires_context(&self) -> bool {
        true
    }

    async fn execute(&self, _arguments: Value) -> ToolExecutionResult {
        ToolExecutionResult::tool_error(
            "tool_search requires tool execution context and cannot run standalone.",
        )
    }

    async fn execute_with_context(
        &self,
        arguments: Value,
        context: &ToolContext,
    ) -> ToolExecutionResult {
        let query = arguments
            .get("query")
            .and_then(|v| v.as_str())
            .unwrap_or("")
            .trim();

        let Some(registry) = &context.tool_registry else {
            return ToolExecutionResult::tool_error(
                "Tool registry not available in this context. tool_search requires worker-side tool execution.",
            );
        };

        let defs = registry.tool_definitions();
        let matches = Self::search(&defs, query);

        if matches.is_empty() {
            // No keyword hits — surface the catalogue (names only) so the model
            // can refine its query instead of dead-ending.
            let names: Vec<&str> = defs
                .iter()
                .map(|d| d.name())
                .filter(|n| *n != TOOL_SEARCH_TOOL_NAME)
                .collect();
            return ToolExecutionResult::success(json!({
                "query": query,
                "tools": [],
                "message": "No tools matched the query. Try a different keyword.",
                "available_tools": names,
            }));
        }

        ToolExecutionResult::success(json!({
            "query": query,
            "tools": matches,
        }))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::capabilities::CapabilityRegistry;
    use crate::tool_types::{BuiltinTool, ToolPolicy};

    fn builtin(name: &str, description: &str, deferrable: DeferrablePolicy) -> ToolDefinition {
        ToolDefinition::Builtin(BuiltinTool {
            name: name.to_string(),
            display_name: None,
            description: description.to_string(),
            parameters: json!({
                "type": "object",
                "properties": { "path": { "type": "string" } },
                "required": ["path"]
            }),
            policy: ToolPolicy::Auto,
            category: None,
            deferrable,
            hints: ToolHints::default(),
        })
    }

    fn many_tools(n: usize) -> Vec<ToolDefinition> {
        (0..n)
            .map(|i| {
                builtin(
                    &format!("tool_{i}"),
                    "does something",
                    DeferrablePolicy::Automatic,
                )
            })
            .collect()
    }

    #[test]
    fn test_capability_metadata() {
        let cap = GenericToolSearchCapability::new();
        assert_eq!(cap.id(), TOOL_SEARCH_CAPABILITY_ID);
        assert_eq!(cap.name(), "Tool Search");
        assert_eq!(cap.category(), Some("Optimization"));
        assert!(cap.system_prompt_addition().is_some());
        assert_eq!(cap.tools().len(), 1);
        assert_eq!(cap.tools()[0].name(), TOOL_SEARCH_TOOL_NAME);
    }

    #[test]
    fn test_capability_registered_in_builtins() {
        let registry = CapabilityRegistry::with_builtins();
        let cap = registry.get(TOOL_SEARCH_CAPABILITY_ID).unwrap();
        assert_eq!(cap.id(), TOOL_SEARCH_CAPABILITY_ID);
    }

    #[test]
    fn test_hook_noop_below_threshold() {
        let hook = DeferSchemaHook { threshold: 15 };
        let tools = many_tools(5);
        let out = hook.transform(tools);
        // Schemas untouched below threshold.
        for t in &out {
            assert!(t.parameters().get("properties").is_some());
        }
    }

    #[test]
    fn test_hook_strips_above_threshold() {
        let hook = DeferSchemaHook { threshold: 15 };
        let out = hook.transform(many_tools(20));
        for t in &out {
            // Stub schema: no real properties, carries the deferral hint.
            assert!(t.parameters().get("properties").is_none());
            assert!(t.parameters().get("description").is_some());
        }
    }

    #[test]
    fn test_hook_preserves_never_defer_and_search_tool() {
        let hook = DeferSchemaHook { threshold: 3 };
        let mut tools = many_tools(3);
        tools.push(builtin("write_todos", "todos", DeferrablePolicy::Never));
        tools.push(ToolSearchTool.to_definition());

        let out = hook.transform(tools);

        let todos = out.iter().find(|t| t.name() == "write_todos").unwrap();
        assert!(
            todos.parameters().get("properties").is_some(),
            "never-defer tool keeps full schema"
        );
        let search = out
            .iter()
            .find(|t| t.name() == TOOL_SEARCH_TOOL_NAME)
            .unwrap();
        assert!(
            search.parameters().get("properties").is_some(),
            "search tool keeps full schema"
        );
        // Deferrable tools were stripped.
        let deferred = out.iter().find(|t| t.name() == "tool_0").unwrap();
        assert!(deferred.parameters().get("properties").is_none());
    }

    #[test]
    fn test_hook_preserves_mcp_tools() {
        // MCP tools must keep full schemas: they become registry proxies in the
        // act path, and stripping them would leave tool_search unable to return
        // their parameters.
        let hook = DeferSchemaHook { threshold: 3 };
        let mut tools = many_tools(3);
        tools.push(builtin(
            "mcp_docs__search",
            "search docs",
            DeferrablePolicy::Automatic,
        ));

        let out = hook.transform(tools);

        let mcp = out.iter().find(|t| t.name() == "mcp_docs__search").unwrap();
        assert!(
            mcp.parameters().get("properties").is_some(),
            "MCP tool keeps full schema"
        );
        // Non-MCP deferrable tools are still stripped.
        let deferred = out.iter().find(|t| t.name() == "tool_0").unwrap();
        assert!(deferred.parameters().get("properties").is_none());
    }

    #[test]
    fn test_hook_opts_out_of_native_tool_search() {
        // Generic (client-side) deferral is mutually exclusive with hosted
        // tool_search; build() uses this to skip the hook when native is active.
        let hook = DeferSchemaHook { threshold: 15 };
        assert!(!hook.applies_with_native_tool_search());
    }

    #[test]
    fn test_search_ranks_by_keyword_overlap() {
        let defs = vec![
            builtin(
                "read_file",
                "Read the contents of a file",
                DeferrablePolicy::Automatic,
            ),
            builtin(
                "send_email",
                "Send an email message",
                DeferrablePolicy::Automatic,
            ),
            builtin(
                "write_file",
                "Write contents to a file",
                DeferrablePolicy::Automatic,
            ),
        ];

        let results = ToolSearchTool::search(&defs, "read file");
        assert_eq!(results[0]["name"], "read_file");
        // Full parameter schema is returned, not the stub.
        assert!(results[0]["parameters"].get("properties").is_some());

        let email = ToolSearchTool::search(&defs, "email");
        assert_eq!(email.len(), 1);
        assert_eq!(email[0]["name"], "send_email");
    }

    #[test]
    fn test_search_excludes_itself() {
        let defs = vec![
            ToolSearchTool.to_definition(),
            builtin("read_file", "Read a file", DeferrablePolicy::Automatic),
        ];
        let results = ToolSearchTool::search(&defs, "tool_search read");
        assert!(results.iter().all(|r| r["name"] != TOOL_SEARCH_TOOL_NAME));
    }

    #[tokio::test]
    async fn test_execute_without_registry_errors() {
        let ctx = ToolContext::new(uuid::Uuid::new_v4().into());
        let result = ToolSearchTool
            .execute_with_context(json!({ "query": "file" }), &ctx)
            .await;
        assert!(matches!(result, ToolExecutionResult::ToolError(_)));
    }

    struct MiniTool;
    #[async_trait]
    impl Tool for MiniTool {
        fn name(&self) -> &str {
            "read_file"
        }
        fn description(&self) -> &str {
            "Read the contents of a file"
        }
        fn parameters_schema(&self) -> Value {
            json!({
                "type": "object",
                "properties": { "path": { "type": "string" } },
                "required": ["path"]
            })
        }
        async fn execute(&self, _arguments: Value) -> ToolExecutionResult {
            ToolExecutionResult::success(json!({}))
        }
    }

    #[tokio::test]
    async fn test_execute_with_registry_returns_schemas() {
        use crate::tools::ToolRegistry;

        let mut registry = ToolRegistry::new();
        registry.register(MiniTool);
        registry.register(ToolSearchTool);

        let mut ctx = ToolContext::new(uuid::Uuid::new_v4().into());
        ctx.tool_registry = Some(Arc::new(registry));

        let result = ToolSearchTool
            .execute_with_context(json!({ "query": "file" }), &ctx)
            .await;

        let ToolExecutionResult::Success(value) = result else {
            panic!("expected success");
        };
        let tools = value["tools"].as_array().unwrap();
        let read = tools.iter().find(|t| t["name"] == "read_file").unwrap();
        // Full schema is returned (not the deferred stub).
        assert!(read["parameters"]["properties"]["path"].is_object());
    }
}
