// Tool definitions and policies for agent execution
//
// These are runtime types used by the agent loop for tool execution.
// They are re-exported by everruns-contracts for backward compatibility.

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
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct BuiltinTool {
    /// Tool name (used by LLM)
    pub name: String,
    /// Tool description for LLM
    pub description: String,
    /// JSON schema for tool parameters
    pub parameters: serde_json::Value,
    /// Built-in tool kind
    pub kind: BuiltinToolKind,
    /// Tool policy (auto or requires_approval)
    #[serde(default)]
    pub policy: ToolPolicy,
}

/// Built-in tool types
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum BuiltinToolKind {
    /// HTTP GET request
    HttpGet,
    /// HTTP POST request
    HttpPost,
    /// Read file (future)
    ReadFile,
    /// Write file (future)
    WriteFile,
    /// Get current time in various formats
    CurrentTime,
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
    fn test_builtin_tool_http_get() {
        let json = r#"{
            "type": "builtin",
            "name": "fetch_data",
            "description": "Fetch data from URL",
            "parameters": {"type": "object"},
            "kind": "http_get"
        }"#;

        let tool: ToolDefinition = serde_json::from_str(json).unwrap();
        match tool {
            ToolDefinition::Builtin(builtin) => {
                assert_eq!(builtin.kind, BuiltinToolKind::HttpGet);
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
            "kind": "write_file",
            "policy": "requires_approval"
        }"#;

        let tool: ToolDefinition = serde_json::from_str(json).unwrap();
        match tool {
            ToolDefinition::Builtin(builtin) => {
                assert_eq!(builtin.policy, ToolPolicy::RequiresApproval);
            }
        }
    }
}
