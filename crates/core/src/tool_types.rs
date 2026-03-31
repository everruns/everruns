// Tool definitions and policies for agent execution
//
// Design Decision: Tools are identified by name (string) for extensibility.
// The BuiltinToolKind enum has been removed to allow adding new tools
// without code changes. Tool execution happens via the ToolRegistry
// which looks up tools by name.

use serde::{Deserialize, Serialize};

#[cfg(feature = "openapi")]
use utoipa::ToSchema;

/// Tool policy determines how tool calls are handled
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq, Default)]
#[cfg_attr(feature = "openapi", derive(ToSchema))]
#[serde(rename_all = "snake_case")]
pub enum ToolPolicy {
    /// Execute immediately without user approval
    #[default]
    Auto,
    /// Require user approval before execution (HITL)
    RequiresApproval,
    /// Client-side tool: pause workflow, send to client for execution
    ClientSide,
}

/// Controls whether a tool's full schema can be deferred (tool_search).
///
/// When tool_search is active and a model supports it, tools marked as
/// `Automatic` or `Always` will have `defer_loading: true` set, meaning
/// only the name+description are sent upfront and full parameter schemas
/// are loaded on-demand by the model.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq, Default)]
#[cfg_attr(feature = "openapi", derive(ToSchema))]
#[serde(rename_all = "snake_case")]
pub enum DeferrablePolicy {
    /// Never defer — always send full schema (e.g., high-frequency tools like write_todos)
    Never,
    /// Let the driver decide based on tool count threshold (default)
    #[default]
    Automatic,
    /// Always defer when tool_search is active, regardless of threshold
    Always,
}

impl DeferrablePolicy {
    /// Returns true when the value is the default (`Automatic`).
    pub fn is_default(&self) -> bool {
        matches!(self, DeferrablePolicy::Automatic)
    }
}

/// Tool definition in agent configuration
#[derive(Debug, Clone, Serialize, Deserialize)]
#[cfg_attr(feature = "openapi", derive(ToSchema))]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum ToolDefinition {
    /// Built-in tool - executed by the worker via ToolRegistry
    Builtin(BuiltinTool),
    /// Client-side tool - executed by the client, not the server
    ClientSide(ClientSideTool),
}

/// Built-in tool configuration
///
/// Note: The `kind` field has been removed. Tools are now identified
/// solely by their `name` field, and execution happens via the ToolRegistry
/// which looks up tools by name.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[cfg_attr(feature = "openapi", derive(ToSchema))]
pub struct BuiltinTool {
    /// Tool name (used by LLM and for registry lookup)
    pub name: String,
    /// Human-readable display name for UI rendering (e.g., "Get Current Time" for `get_current_time`)
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub display_name: Option<String>,
    /// Tool description for LLM
    pub description: String,
    /// JSON schema for tool parameters
    pub parameters: serde_json::Value,
    /// Tool policy (auto or requires_approval)
    #[serde(default)]
    pub policy: ToolPolicy,
    /// Category for tool_search namespace grouping (from parent capability)
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub category: Option<String>,
    /// Whether this tool's schema can be deferred via tool_search
    #[serde(default, skip_serializing_if = "DeferrablePolicy::is_default")]
    pub deferrable: DeferrablePolicy,
    /// Semantic hints describing the tool's behavioral properties
    #[serde(default, skip_serializing_if = "ToolHints::is_empty")]
    pub hints: ToolHints,
}

/// Client-side tool - executed by the client, not the server
/// The server pauses execution and waits for the client to submit results.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[cfg_attr(feature = "openapi", derive(ToSchema))]
pub struct ClientSideTool {
    /// Tool name (used by LLM and for correlation)
    pub name: String,
    /// Human-readable display name for UI rendering
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub display_name: Option<String>,
    /// Tool description for LLM
    pub description: String,
    /// JSON schema for tool parameters
    pub parameters: serde_json::Value,
    /// Category for tool_search namespace grouping (from parent capability)
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub category: Option<String>,
    /// Whether this tool's schema can be deferred via tool_search
    #[serde(default, skip_serializing_if = "DeferrablePolicy::is_default")]
    pub deferrable: DeferrablePolicy,
    /// Semantic hints describing the tool's behavioral properties
    #[serde(default, skip_serializing_if = "ToolHints::is_empty")]
    pub hints: ToolHints,
}

impl ToolDefinition {
    /// Get the tool name regardless of variant
    pub fn name(&self) -> &str {
        match self {
            ToolDefinition::Builtin(b) => &b.name,
            ToolDefinition::ClientSide(c) => &c.name,
        }
    }

    /// Get the tool display name regardless of variant
    pub fn display_name(&self) -> Option<&str> {
        match self {
            ToolDefinition::Builtin(b) => b.display_name.as_deref(),
            ToolDefinition::ClientSide(c) => c.display_name.as_deref(),
        }
    }

    /// Get the tool description regardless of variant
    pub fn description(&self) -> &str {
        match self {
            ToolDefinition::Builtin(b) => &b.description,
            ToolDefinition::ClientSide(c) => &c.description,
        }
    }

    /// Get the tool parameters schema regardless of variant
    pub fn parameters(&self) -> &serde_json::Value {
        match self {
            ToolDefinition::Builtin(b) => &b.parameters,
            ToolDefinition::ClientSide(c) => &c.parameters,
        }
    }

    /// Get the tool policy regardless of variant
    pub fn policy(&self) -> &ToolPolicy {
        match self {
            ToolDefinition::Builtin(b) => &b.policy,
            ToolDefinition::ClientSide(_) => &ToolPolicy::ClientSide,
        }
    }

    /// Get the tool category for namespace grouping
    pub fn category(&self) -> Option<&str> {
        match self {
            ToolDefinition::Builtin(b) => b.category.as_deref(),
            ToolDefinition::ClientSide(c) => c.category.as_deref(),
        }
    }

    /// Get the deferrable policy for tool_search
    pub fn deferrable(&self) -> &DeferrablePolicy {
        match self {
            ToolDefinition::Builtin(b) => &b.deferrable,
            ToolDefinition::ClientSide(c) => &c.deferrable,
        }
    }

    /// Get the tool hints
    pub fn hints(&self) -> &ToolHints {
        match self {
            ToolDefinition::Builtin(b) => &b.hints,
            ToolDefinition::ClientSide(c) => &c.hints,
        }
    }

    /// Set the category on this tool definition (builder pattern)
    pub fn with_category(mut self, category: impl Into<String>) -> Self {
        match &mut self {
            ToolDefinition::Builtin(b) => b.category = Some(category.into()),
            ToolDefinition::ClientSide(c) => c.category = Some(category.into()),
        }
        self
    }

    /// Set the hints on this tool definition (builder pattern)
    pub fn with_hints(mut self, hints: ToolHints) -> Self {
        match &mut self {
            ToolDefinition::Builtin(b) => b.hints = hints,
            ToolDefinition::ClientSide(c) => c.hints = hints,
        }
        self
    }
}

/// Semantic hints describing a tool's behavioral properties.
///
/// Follows the MCP tool annotations convention (readOnlyHint, destructiveHint,
/// idempotentHint, openWorldHint) plus everruns-specific hints. All fields are
/// optional booleans — `None` means "unknown/unspecified". Consumers should
/// treat `None` as the conservative default (e.g., assume not readonly, assume
/// not idempotent).
///
/// These hints are informational — they do not enforce policy. Use `ToolPolicy`
/// for execution gating (auto vs requires_approval).
#[derive(Debug, Clone, Serialize, Deserialize, Default, PartialEq, Eq)]
#[cfg_attr(feature = "openapi", derive(ToSchema))]
pub struct ToolHints {
    /// Tool does not modify any state (read-only queries, lookups).
    /// When true: safe to call speculatively, result can be cached.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub readonly: Option<bool>,

    /// Tool may irreversibly destroy or delete data.
    /// Subset of non-readonly — a tool can be non-readonly (writes) without
    /// being destructive (e.g., create/update operations).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub destructive: Option<bool>,

    /// Calling the tool repeatedly with the same arguments produces the same
    /// effect. Safe to retry on transient failures.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub idempotent: Option<bool>,

    /// Tool interacts with external entities beyond the local system
    /// (network calls, third-party APIs, cloud services).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub open_world: Option<bool>,

    /// Tool requires API keys, credentials, or other secrets to function.
    /// Useful for UI to show connection prompts and for LLMs to anticipate
    /// authentication failures.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub requires_secrets: Option<bool>,

    /// Tool may take significant time to complete (> ~5s typical).
    /// Useful for clients to show progress indicators and set timeouts.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub long_running: Option<bool>,

    /// Tool output should be persisted to session VFS before truncation.
    /// When set, the `tool_output_persistence` capability (EVE-222) writes the
    /// full cleaned output to `/.exec-logs/{tool_call_id}.log` and injects a
    /// `full_output` path into the result so the LLM can retrieve it on demand.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub persist_output: Option<bool>,
}

impl ToolHints {
    /// Returns true when all fields are None (default/empty state).
    pub fn is_empty(&self) -> bool {
        *self == Self::default()
    }

    /// Builder: set readonly hint.
    pub fn with_readonly(mut self, value: bool) -> Self {
        self.readonly = Some(value);
        self
    }

    /// Builder: set destructive hint.
    pub fn with_destructive(mut self, value: bool) -> Self {
        self.destructive = Some(value);
        self
    }

    /// Builder: set idempotent hint.
    pub fn with_idempotent(mut self, value: bool) -> Self {
        self.idempotent = Some(value);
        self
    }

    /// Builder: set open_world hint.
    pub fn with_open_world(mut self, value: bool) -> Self {
        self.open_world = Some(value);
        self
    }

    /// Builder: set requires_secrets hint.
    pub fn with_requires_secrets(mut self, value: bool) -> Self {
        self.requires_secrets = Some(value);
        self
    }

    /// Builder: set long_running hint.
    pub fn with_long_running(mut self, value: bool) -> Self {
        self.long_running = Some(value);
        self
    }

    /// Builder: set persist_output hint.
    pub fn with_persist_output(mut self, value: bool) -> Self {
        self.persist_output = Some(value);
        self
    }
}

/// Tool call from LLM response
#[derive(Debug, Clone, Serialize, Deserialize)]
#[cfg_attr(feature = "openapi", derive(ToSchema))]
pub struct ToolCall {
    /// Unique ID for this tool call
    pub id: String,
    /// Tool name to execute
    pub name: String,
    /// Arguments as JSON
    #[cfg_attr(feature = "openapi", schema(value_type = Object))]
    pub arguments: serde_json::Value,
}

impl ToolCall {
    /// Convert tool call to OpenAI-compatible format
    ///
    /// Returns format: `{id, type: "function", function: {name, arguments}}`
    /// where arguments is stringified JSON.
    pub fn to_openai_format(&self) -> serde_json::Value {
        serde_json::json!({
            "id": self.id,
            "type": "function",
            "function": {
                "name": self.name,
                "arguments": serde_json::to_string(&self.arguments).unwrap_or_else(|_| "{}".to_string())
            }
        })
    }
}

/// Tool execution result
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ToolResult {
    /// Tool call ID this result corresponds to
    pub tool_call_id: String,
    /// Result data (success)
    pub result: Option<serde_json::Value>,
    /// Images returned by the tool (sent as native image content to LLM)
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub images: Option<Vec<crate::tools::ToolResultImage>>,
    /// Error message (failure)
    pub error: Option<String>,
    /// When set, indicates the tool requires a user connection for this provider.
    /// The workflow should pause and prompt the user to configure the connection.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub connection_required: Option<String>,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_builtin_tool_serialization() {
        let json = r#"{
            "type": "builtin",
            "name": "fetch_data",
            "description": "Fetch data from URL",
            "parameters": {"type": "object"}
        }"#;

        let tool: ToolDefinition = serde_json::from_str(json).unwrap();
        match tool {
            ToolDefinition::Builtin(builtin) => {
                assert_eq!(builtin.name, "fetch_data");
                assert_eq!(builtin.policy, ToolPolicy::Auto);
            }
            _ => panic!("expected Builtin variant"),
        }
    }

    #[test]
    fn test_builtin_tool_requires_approval() {
        let json = r#"{
            "type": "builtin",
            "name": "delete_file",
            "description": "Delete a file",
            "parameters": {"type": "object"},
            "policy": "requires_approval"
        }"#;

        let tool: ToolDefinition = serde_json::from_str(json).unwrap();
        match tool {
            ToolDefinition::Builtin(builtin) => {
                assert_eq!(builtin.policy, ToolPolicy::RequiresApproval);
            }
            _ => panic!("expected Builtin variant"),
        }
    }

    #[test]
    fn test_tool_call_serialization() {
        let tool_call = ToolCall {
            id: "call_123".to_string(),
            name: "get_weather".to_string(),
            arguments: serde_json::json!({"city": "New York"}),
        };

        let json = serde_json::to_string(&tool_call).unwrap();
        let parsed: ToolCall = serde_json::from_str(&json).unwrap();

        assert_eq!(parsed.id, tool_call.id);
        assert_eq!(parsed.name, tool_call.name);
    }

    #[test]
    fn test_tool_result_serialization() {
        let result = ToolResult {
            tool_call_id: "call_123".to_string(),
            result: Some(serde_json::json!({"temperature": 72})),
            images: None,
            error: None,
            connection_required: None,
        };

        let json = serde_json::to_string(&result).unwrap();
        let parsed: ToolResult = serde_json::from_str(&json).unwrap();

        assert_eq!(parsed.tool_call_id, result.tool_call_id);
        assert!(parsed.result.is_some());
        assert!(parsed.error.is_none());
    }

    #[test]
    fn test_tool_definition_accessor_methods() {
        let tool = ToolDefinition::Builtin(BuiltinTool {
            name: "test_tool".to_string(),
            display_name: None,
            description: "A test tool".to_string(),
            parameters: serde_json::json!({"type": "object"}),
            policy: ToolPolicy::RequiresApproval,
            category: None,
            deferrable: DeferrablePolicy::default(),
            hints: ToolHints::default(),
        });

        assert_eq!(tool.name(), "test_tool");
        assert_eq!(tool.display_name(), None);
        assert_eq!(tool.description(), "A test tool");
        assert_eq!(tool.parameters(), &serde_json::json!({"type": "object"}));
        assert_eq!(tool.policy(), &ToolPolicy::RequiresApproval);
    }

    #[test]
    fn test_tool_definition_display_name_accessor() {
        let builtin = ToolDefinition::Builtin(BuiltinTool {
            name: "get_weather".to_string(),
            display_name: Some("Get Weather".to_string()),
            description: "Gets weather".to_string(),
            parameters: serde_json::json!({}),
            policy: ToolPolicy::Auto,
            category: None,
            deferrable: DeferrablePolicy::default(),
            hints: ToolHints::default(),
        });
        assert_eq!(builtin.display_name(), Some("Get Weather"));

        let client = ToolDefinition::ClientSide(ClientSideTool {
            name: "deploy".to_string(),
            display_name: Some("Deploy".to_string()),
            description: "Deploys".to_string(),
            parameters: serde_json::json!({}),
            category: None,
            deferrable: DeferrablePolicy::default(),
            hints: ToolHints::default(),
        });
        assert_eq!(client.display_name(), Some("Deploy"));
    }

    #[test]
    fn test_display_name_serialization_skip_none() {
        let tool = BuiltinTool {
            name: "test".to_string(),
            display_name: None,
            description: "test".to_string(),
            parameters: serde_json::json!({}),
            policy: ToolPolicy::Auto,
            category: None,
            deferrable: DeferrablePolicy::default(),
            hints: ToolHints::default(),
        };
        let json = serde_json::to_string(&tool).unwrap();
        assert!(!json.contains("display_name"));

        let tool_with = BuiltinTool {
            name: "test".to_string(),
            display_name: Some("Test".to_string()),
            description: "test".to_string(),
            parameters: serde_json::json!({}),
            policy: ToolPolicy::Auto,
            category: None,
            deferrable: DeferrablePolicy::default(),
            hints: ToolHints::default(),
        };
        let json = serde_json::to_string(&tool_with).unwrap();
        assert!(json.contains("\"display_name\":\"Test\""));
    }

    #[test]
    fn test_tool_call_to_openai_format() {
        let tool_call = ToolCall {
            id: "call_123".to_string(),
            name: "get_weather".to_string(),
            arguments: serde_json::json!({"location": "Tokyo", "units": "celsius"}),
        };

        let converted = tool_call.to_openai_format();

        assert_eq!(converted["id"], "call_123");
        assert_eq!(converted["type"], "function");
        assert_eq!(converted["function"]["name"], "get_weather");
        // Arguments should be stringified JSON
        let args: serde_json::Value =
            serde_json::from_str(converted["function"]["arguments"].as_str().unwrap()).unwrap();
        assert_eq!(args["location"], "Tokyo");
        assert_eq!(args["units"], "celsius");
    }

    #[test]
    fn test_tool_call_to_openai_format_empty_arguments() {
        let tool_call = ToolCall {
            id: "call_456".to_string(),
            name: "list_files".to_string(),
            arguments: serde_json::json!({}),
        };

        let converted = tool_call.to_openai_format();

        assert_eq!(converted["id"], "call_456");
        assert_eq!(converted["function"]["name"], "list_files");
        assert_eq!(converted["function"]["arguments"], "{}");
    }

    #[test]
    fn test_client_side_tool_serialization() {
        let json = r#"{
            "type": "client_side",
            "name": "browser_click",
            "description": "Click an element in the browser",
            "parameters": {"type": "object", "properties": {"selector": {"type": "string"}}}
        }"#;

        let tool: ToolDefinition = serde_json::from_str(json).unwrap();
        match &tool {
            ToolDefinition::ClientSide(client) => {
                assert_eq!(client.name, "browser_click");
                assert_eq!(client.description, "Click an element in the browser");
            }
            _ => panic!("expected ClientSide variant"),
        }

        assert_eq!(tool.name(), "browser_click");
        assert_eq!(tool.policy(), &ToolPolicy::ClientSide);
    }

    #[test]
    fn test_client_side_tool_roundtrip() {
        let tool = ToolDefinition::ClientSide(ClientSideTool {
            name: "run_test".to_string(),
            display_name: None,
            description: "Run a test suite".to_string(),
            parameters: serde_json::json!({"type": "object"}),
            category: None,
            deferrable: DeferrablePolicy::default(),
            hints: ToolHints::default(),
        });

        let json = serde_json::to_string(&tool).unwrap();
        let parsed: ToolDefinition = serde_json::from_str(&json).unwrap();

        assert_eq!(parsed.name(), "run_test");
        assert_eq!(parsed.description(), "Run a test suite");
        assert_eq!(parsed.policy(), &ToolPolicy::ClientSide);
    }

    #[test]
    fn test_client_side_tool_accessor_methods() {
        let tool = ToolDefinition::ClientSide(ClientSideTool {
            name: "deploy_app".to_string(),
            display_name: None,
            description: "Deploy application to staging".to_string(),
            parameters: serde_json::json!({
                "type": "object",
                "properties": {
                    "env": {"type": "string"}
                },
                "required": ["env"]
            }),
            category: None,
            deferrable: DeferrablePolicy::default(),
            hints: ToolHints::default(),
        });

        assert_eq!(tool.name(), "deploy_app");
        assert_eq!(tool.description(), "Deploy application to staging");
        assert_eq!(tool.policy(), &ToolPolicy::ClientSide);
        assert!(tool.parameters().get("properties").is_some());
    }

    #[test]
    fn test_client_side_tool_policy_always_client_side() {
        // ClientSide variant always returns ClientSide policy regardless of content
        let tool = ToolDefinition::ClientSide(ClientSideTool {
            name: "any_tool".to_string(),
            display_name: None,
            description: "".to_string(),
            parameters: serde_json::json!({}),
            category: None,
            deferrable: DeferrablePolicy::default(),
            hints: ToolHints::default(),
        });
        assert_eq!(tool.policy(), &ToolPolicy::ClientSide);
    }

    #[test]
    fn test_tool_policy_serialization() {
        assert_eq!(
            serde_json::to_string(&ToolPolicy::ClientSide).unwrap(),
            r#""client_side""#
        );
        assert_eq!(
            serde_json::to_string(&ToolPolicy::Auto).unwrap(),
            r#""auto""#
        );
        assert_eq!(
            serde_json::to_string(&ToolPolicy::RequiresApproval).unwrap(),
            r#""requires_approval""#
        );
    }

    #[test]
    fn test_mixed_tool_definitions_in_vec() {
        let tools = vec![
            ToolDefinition::Builtin(BuiltinTool {
                name: "server_tool".to_string(),
                display_name: None,
                description: "A server tool".to_string(),
                parameters: serde_json::json!({"type": "object"}),
                policy: ToolPolicy::Auto,
                category: None,
                deferrable: DeferrablePolicy::default(),
                hints: ToolHints::default(),
            }),
            ToolDefinition::ClientSide(ClientSideTool {
                name: "client_tool".to_string(),
                display_name: None,
                description: "A client tool".to_string(),
                parameters: serde_json::json!({"type": "object"}),
                category: None,
                deferrable: DeferrablePolicy::default(),
                hints: ToolHints::default(),
            }),
        ];

        let json = serde_json::to_string(&tools).unwrap();
        let parsed: Vec<ToolDefinition> = serde_json::from_str(&json).unwrap();

        assert_eq!(parsed.len(), 2);
        assert!(matches!(&parsed[0], ToolDefinition::Builtin(_)));
        assert!(matches!(&parsed[1], ToolDefinition::ClientSide(_)));
        assert_eq!(parsed[0].policy(), &ToolPolicy::Auto);
        assert_eq!(parsed[1].policy(), &ToolPolicy::ClientSide);
    }

    #[test]
    fn test_tool_hints_default_is_empty() {
        let hints = ToolHints::default();
        assert!(hints.is_empty());
        assert_eq!(hints.readonly, None);
        assert_eq!(hints.destructive, None);
        assert_eq!(hints.idempotent, None);
        assert_eq!(hints.open_world, None);
        assert_eq!(hints.requires_secrets, None);
        assert_eq!(hints.long_running, None);
    }

    #[test]
    fn test_tool_hints_builder() {
        let hints = ToolHints::default()
            .with_readonly(true)
            .with_destructive(false)
            .with_idempotent(true)
            .with_open_world(true)
            .with_requires_secrets(true)
            .with_long_running(false);

        assert!(!hints.is_empty());
        assert_eq!(hints.readonly, Some(true));
        assert_eq!(hints.destructive, Some(false));
        assert_eq!(hints.idempotent, Some(true));
        assert_eq!(hints.open_world, Some(true));
        assert_eq!(hints.requires_secrets, Some(true));
        assert_eq!(hints.long_running, Some(false));
    }

    #[test]
    fn test_tool_hints_serialization_skip_empty() {
        let tool = BuiltinTool {
            name: "test".to_string(),
            display_name: None,
            description: "test".to_string(),
            parameters: serde_json::json!({}),
            policy: ToolPolicy::Auto,
            category: None,
            deferrable: DeferrablePolicy::default(),
            hints: ToolHints::default(),
        };
        let json = serde_json::to_string(&tool).unwrap();
        assert!(!json.contains("hints"), "empty hints should be skipped");
    }

    #[test]
    fn test_tool_hints_serialization_present() {
        let tool = BuiltinTool {
            name: "test".to_string(),
            display_name: None,
            description: "test".to_string(),
            parameters: serde_json::json!({}),
            policy: ToolPolicy::Auto,
            category: None,
            deferrable: DeferrablePolicy::default(),
            hints: ToolHints::default()
                .with_readonly(true)
                .with_idempotent(true),
        };
        let json = serde_json::to_string(&tool).unwrap();
        assert!(json.contains("\"hints\""));
        assert!(json.contains("\"readonly\":true"));
        assert!(json.contains("\"idempotent\":true"));
        // Unset hints should not appear
        assert!(!json.contains("destructive"));
        assert!(!json.contains("open_world"));
    }

    #[test]
    fn test_tool_hints_deserialization_missing() {
        let json = r#"{
            "type": "builtin",
            "name": "test",
            "description": "test",
            "parameters": {}
        }"#;
        let tool: ToolDefinition = serde_json::from_str(json).unwrap();
        assert!(tool.hints().is_empty());
    }

    #[test]
    fn test_tool_hints_deserialization_present() {
        let json = r#"{
            "type": "builtin",
            "name": "test",
            "description": "test",
            "parameters": {},
            "hints": {"readonly": true, "open_world": true, "requires_secrets": true}
        }"#;
        let tool: ToolDefinition = serde_json::from_str(json).unwrap();
        let hints = tool.hints();
        assert_eq!(hints.readonly, Some(true));
        assert_eq!(hints.open_world, Some(true));
        assert_eq!(hints.requires_secrets, Some(true));
        assert_eq!(hints.destructive, None);
        assert_eq!(hints.idempotent, None);
        assert_eq!(hints.long_running, None);
    }

    #[test]
    fn test_tool_definition_with_hints_builder() {
        let tool = ToolDefinition::Builtin(BuiltinTool {
            name: "test".to_string(),
            display_name: None,
            description: "test".to_string(),
            parameters: serde_json::json!({}),
            policy: ToolPolicy::Auto,
            category: None,
            deferrable: DeferrablePolicy::default(),
            hints: ToolHints::default(),
        })
        .with_hints(ToolHints::default().with_readonly(true));

        assert_eq!(tool.hints().readonly, Some(true));
    }
}
