// Tool definitions and policies for agent execution
//
// Design Decision: Tools are identified by name (string) for extensibility.
// The BuiltinToolKind enum has been removed to allow adding new tools
// without code changes. Tool execution happens via the ToolRegistry
// which looks up tools by name.

use serde::{Deserialize, Serialize};

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

/// Tool call from LLM response
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ToolCall {
    /// Unique ID for this tool call
    pub id: String,
    /// Tool name to execute
    pub name: String,
    /// Arguments as JSON
    pub arguments: serde_json::Value,
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
}
