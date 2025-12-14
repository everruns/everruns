// Tool definitions and policies for agent execution

use serde::{Deserialize, Serialize};
use std::collections::HashMap;

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
    /// Webhook tool - calls external HTTP endpoint
    Webhook(WebhookTool),
    /// Built-in tool - executed by the worker
    Builtin(BuiltinTool),
}

/// Webhook tool configuration
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct WebhookTool {
    /// Tool name (used by LLM)
    pub name: String,
    /// Tool description for LLM
    pub description: String,
    /// JSON schema for tool parameters
    pub parameters: serde_json::Value,
    /// Webhook endpoint URL
    pub url: String,
    /// HTTP method (default: POST)
    #[serde(default = "default_http_method")]
    pub method: String,
    /// Request headers
    #[serde(default)]
    pub headers: HashMap<String, String>,
    /// Signing secret for request verification (optional)
    pub signing_secret: Option<String>,
    /// Timeout in seconds (default: 30)
    #[serde(default = "default_timeout")]
    pub timeout_secs: u64,
    /// Maximum retries (default: 3)
    #[serde(default = "default_retries")]
    pub max_retries: u32,
    /// Tool policy (auto or requires_approval)
    #[serde(default)]
    pub policy: ToolPolicy,
}

fn default_http_method() -> String {
    "POST".to_string()
}

fn default_timeout() -> u64 {
    30
}

fn default_retries() -> u32 {
    3
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
    fn test_webhook_tool_defaults() {
        let json = r#"{
            "type": "webhook",
            "name": "send_email",
            "description": "Send an email",
            "parameters": {"type": "object"},
            "url": "https://example.com/webhook"
        }"#;

        let tool: ToolDefinition = serde_json::from_str(json).unwrap();
        match tool {
            ToolDefinition::Webhook(webhook) => {
                assert_eq!(webhook.method, "POST");
                assert_eq!(webhook.timeout_secs, 30);
                assert_eq!(webhook.max_retries, 3);
                assert_eq!(webhook.policy, ToolPolicy::Auto);
            }
            _ => panic!("Expected webhook tool"),
        }
    }

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
            _ => panic!("Expected builtin tool"),
        }
    }

    #[test]
    fn test_tool_policy_requires_approval() {
        let json = r#"{
            "type": "webhook",
            "name": "delete_user",
            "description": "Delete a user",
            "parameters": {"type": "object"},
            "url": "https://example.com/delete",
            "policy": "requires_approval"
        }"#;

        let tool: ToolDefinition = serde_json::from_str(json).unwrap();
        match tool {
            ToolDefinition::Webhook(webhook) => {
                assert_eq!(webhook.policy, ToolPolicy::RequiresApproval);
            }
            _ => panic!("Expected webhook tool"),
        }
    }
}
