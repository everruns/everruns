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
#[serde(rename_all = "snake_case")]
pub enum ToolPolicy {
    /// Execute immediately without user approval
    #[default]
    Auto,
    /// Require user approval before execution (HITL)
    RequiresApproval,
}

/// Tool definition in agent configuration
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum ToolDefinition {
    /// Built-in tool - executed by the worker via ToolRegistry
    Builtin(BuiltinTool),
}

/// Built-in tool configuration
///
/// Note: The `kind` field has been removed. Tools are now identified
/// solely by their `name` field, and execution happens via the ToolRegistry
/// which looks up tools by name.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct BuiltinTool {
    /// Tool name (used by LLM and for registry lookup)
    pub name: String,
    /// Tool description for LLM
    pub description: String,
    /// JSON schema for tool parameters
    pub parameters: serde_json::Value,
    /// Tool policy (auto or requires_approval)
    #[serde(default)]
    pub policy: ToolPolicy,
}

impl ToolDefinition {
    /// Get the tool name regardless of variant
    pub fn name(&self) -> &str {
        match self {
            ToolDefinition::Builtin(b) => &b.name,
        }
    }

    /// Get the tool description regardless of variant
    pub fn description(&self) -> &str {
        match self {
            ToolDefinition::Builtin(b) => &b.description,
        }
    }

    /// Get the tool parameters schema regardless of variant
    pub fn parameters(&self) -> &serde_json::Value {
        match self {
            ToolDefinition::Builtin(b) => &b.parameters,
        }
    }

    /// Get the tool policy regardless of variant
    pub fn policy(&self) -> &ToolPolicy {
        match self {
            ToolDefinition::Builtin(b) => &b.policy,
        }
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
    /// Error message (failure)
    pub error: Option<String>,
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
            error: None,
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
            description: "A test tool".to_string(),
            parameters: serde_json::json!({"type": "object"}),
            policy: ToolPolicy::RequiresApproval,
        });

        assert_eq!(tool.name(), "test_tool");
        assert_eq!(tool.description(), "A test tool");
        assert_eq!(tool.parameters(), &serde_json::json!({"type": "object"}));
        assert_eq!(tool.policy(), &ToolPolicy::RequiresApproval);
    }
}
