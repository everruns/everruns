// V2 Workflow Types
//
// Decision: Keep types simple and serializable for Temporal compatibility
// Decision: Use Value for tool arguments to maintain flexibility

use serde::{Deserialize, Serialize};
use uuid::Uuid;

/// Configuration for the agent
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AgentConfig {
    /// Agent ID
    pub agent_id: Uuid,
    /// Agent name
    pub name: String,
    /// Model identifier (e.g., "gpt-4", "claude-3")
    pub model: String,
    /// System prompt
    pub system_prompt: Option<String>,
    /// Temperature for LLM calls
    pub temperature: Option<f32>,
    /// Max tokens for LLM response
    pub max_tokens: Option<u32>,
    /// Available tools
    pub tools: Vec<ToolDefinition>,
}

impl AgentConfig {
    /// Create a minimal agent config for testing
    pub fn test(name: &str) -> Self {
        Self {
            agent_id: Uuid::now_v7(),
            name: name.to_string(),
            model: "test-model".to_string(),
            system_prompt: None,
            temperature: None,
            max_tokens: None,
            tools: Vec::new(),
        }
    }

    /// Add a tool to the config
    pub fn with_tool(mut self, tool: ToolDefinition) -> Self {
        self.tools.push(tool);
        self
    }

    /// Set the system prompt
    pub fn with_system_prompt(mut self, prompt: &str) -> Self {
        self.system_prompt = Some(prompt.to_string());
        self
    }
}

/// Tool definition
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ToolDefinition {
    /// Tool name
    pub name: String,
    /// Tool description
    pub description: String,
    /// JSON Schema for parameters
    pub parameters: serde_json::Value,
}

impl ToolDefinition {
    /// Create a simple tool definition
    pub fn new(name: &str, description: &str) -> Self {
        Self {
            name: name.to_string(),
            description: description.to_string(),
            parameters: serde_json::json!({
                "type": "object",
                "properties": {}
            }),
        }
    }

    /// Set the parameters schema
    pub fn with_parameters(mut self, parameters: serde_json::Value) -> Self {
        self.parameters = parameters;
        self
    }
}

/// Message in a conversation
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct Message {
    /// Unique message ID
    pub id: Uuid,
    /// Message role
    pub role: MessageRole,
    /// Message content
    pub content: MessageContent,
    /// Tool call ID (for tool results)
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub tool_call_id: Option<String>,
    /// Tool calls (for assistant messages)
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub tool_calls: Option<Vec<ToolCall>>,
}

impl Message {
    /// Create a user message
    pub fn user(content: &str) -> Self {
        Self {
            id: Uuid::now_v7(),
            role: MessageRole::User,
            content: MessageContent::Text(content.to_string()),
            tool_call_id: None,
            tool_calls: None,
        }
    }

    /// Create an assistant message
    pub fn assistant(content: &str) -> Self {
        Self {
            id: Uuid::now_v7(),
            role: MessageRole::Assistant,
            content: MessageContent::Text(content.to_string()),
            tool_call_id: None,
            tool_calls: None,
        }
    }

    /// Create an assistant message with tool calls
    pub fn assistant_with_tool_calls(content: &str, tool_calls: Vec<ToolCall>) -> Self {
        Self {
            id: Uuid::now_v7(),
            role: MessageRole::Assistant,
            content: MessageContent::Text(content.to_string()),
            tool_call_id: None,
            tool_calls: Some(tool_calls),
        }
    }

    /// Create a tool result message
    pub fn tool_result(tool_call_id: &str, result: serde_json::Value) -> Self {
        Self {
            id: Uuid::now_v7(),
            role: MessageRole::Tool,
            content: MessageContent::ToolResult(result),
            tool_call_id: Some(tool_call_id.to_string()),
            tool_calls: None,
        }
    }

    /// Create a tool error message
    pub fn tool_error(tool_call_id: &str, error: &str) -> Self {
        Self {
            id: Uuid::now_v7(),
            role: MessageRole::Tool,
            content: MessageContent::ToolError(error.to_string()),
            tool_call_id: Some(tool_call_id.to_string()),
            tool_calls: None,
        }
    }

    /// Create a system message
    pub fn system(content: &str) -> Self {
        Self {
            id: Uuid::now_v7(),
            role: MessageRole::System,
            content: MessageContent::Text(content.to_string()),
            tool_call_id: None,
            tool_calls: None,
        }
    }
}

/// Message role
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum MessageRole {
    System,
    User,
    Assistant,
    Tool,
}

/// Message content
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(untagged)]
pub enum MessageContent {
    Text(String),
    ToolResult(serde_json::Value),
    ToolError(String),
}

impl MessageContent {
    /// Get text content if available
    pub fn as_text(&self) -> Option<&str> {
        match self {
            MessageContent::Text(text) => Some(text),
            _ => None,
        }
    }
}

/// Tool call from LLM
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct ToolCall {
    /// Unique tool call ID
    pub id: String,
    /// Tool name
    pub name: String,
    /// Tool arguments (JSON)
    pub arguments: serde_json::Value,
}

impl ToolCall {
    /// Create a new tool call
    pub fn new(name: &str, arguments: serde_json::Value) -> Self {
        Self {
            id: format!("call_{}", Uuid::now_v7().simple()),
            name: name.to_string(),
            arguments,
        }
    }
}

/// Tool execution result
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ToolResult {
    /// Tool call ID
    pub tool_call_id: String,
    /// Result value (if successful)
    pub result: Option<serde_json::Value>,
    /// Error message (if failed)
    pub error: Option<String>,
}

impl ToolResult {
    /// Create a successful result
    pub fn success(tool_call_id: &str, result: serde_json::Value) -> Self {
        Self {
            tool_call_id: tool_call_id.to_string(),
            result: Some(result),
            error: None,
        }
    }

    /// Create an error result
    pub fn error(tool_call_id: &str, error: &str) -> Self {
        Self {
            tool_call_id: tool_call_id.to_string(),
            result: None,
            error: Some(error.to_string()),
        }
    }
}

/// LLM response
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct LlmResponse {
    /// Response text
    pub text: String,
    /// Tool calls (if any)
    pub tool_calls: Vec<ToolCall>,
    /// Whether the response is complete (no more tool calls needed)
    pub is_complete: bool,
}

impl LlmResponse {
    /// Create a text-only response (complete)
    pub fn text(content: &str) -> Self {
        Self {
            text: content.to_string(),
            tool_calls: Vec::new(),
            is_complete: true,
        }
    }

    /// Create a response with tool calls
    pub fn with_tools(text: &str, tool_calls: Vec<ToolCall>) -> Self {
        Self {
            text: text.to_string(),
            tool_calls,
            is_complete: false,
        }
    }
}

/// Input for the session workflow
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SessionInput {
    /// Session ID
    pub session_id: Uuid,
    /// Agent ID
    pub agent_id: Uuid,
}

impl SessionInput {
    /// Create a new session input
    pub fn new(agent_id: Uuid) -> Self {
        Self {
            session_id: Uuid::now_v7(),
            agent_id,
        }
    }
}

/// Signal to add a new message to the session
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct NewMessageSignal {
    /// The message to add
    pub message: Message,
}

/// Output from the session workflow (returned when workflow completes)
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SessionOutput {
    /// Session ID
    pub session_id: Uuid,
    /// Final status
    pub status: SessionStatus,
    /// Total number of turns completed
    pub total_turns: u32,
    /// Error message (if failed)
    pub error: Option<String>,
}

/// Session status
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum SessionStatus {
    /// Waiting for user input
    Waiting,
    /// Processing a turn (agent loop running)
    Running,
    /// Session completed successfully
    Completed,
    /// Session failed
    Failed,
}

// Activity input/output types

/// Input for LoadAgent activity
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct LoadAgentInput {
    pub agent_id: Uuid,
}

/// Output from LoadAgent activity
pub type LoadAgentOutput = AgentConfig;

/// Input for CallLlm activity
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CallLlmInput {
    pub session_id: Uuid,
    pub agent_config: AgentConfig,
    pub messages: Vec<Message>,
}

/// Output from CallLlm activity
pub type CallLlmOutput = LlmResponse;

/// Input for ExecuteTools activity (parallel execution)
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ExecuteToolsInput {
    pub session_id: Uuid,
    pub tool_calls: Vec<ToolCall>,
    pub tool_definitions: Vec<ToolDefinition>,
}

/// Output from ExecuteTools activity
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ExecuteToolsOutput {
    pub results: Vec<ToolResult>,
}

/// Input for a single tool execution (for parallel activities)
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ExecuteSingleToolInput {
    pub session_id: Uuid,
    pub tool_call: ToolCall,
    pub tool_definition: Option<ToolDefinition>,
}

/// Output from a single tool execution
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ExecuteSingleToolOutput {
    pub result: ToolResult,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_message_user() {
        let msg = Message::user("Hello");
        assert_eq!(msg.role, MessageRole::User);
        assert_eq!(msg.content.as_text(), Some("Hello"));
    }

    #[test]
    fn test_message_assistant_with_tools() {
        let tool_calls = vec![ToolCall::new("get_time", serde_json::json!({}))];
        let msg = Message::assistant_with_tool_calls("Let me check", tool_calls);
        assert_eq!(msg.role, MessageRole::Assistant);
        assert!(msg.tool_calls.is_some());
        assert_eq!(msg.tool_calls.as_ref().unwrap().len(), 1);
    }

    #[test]
    fn test_message_tool_result() {
        let msg = Message::tool_result("call_123", serde_json::json!({"time": "12:00"}));
        assert_eq!(msg.role, MessageRole::Tool);
        assert_eq!(msg.tool_call_id, Some("call_123".to_string()));
    }

    #[test]
    fn test_agent_config_builder() {
        let tool = ToolDefinition::new("test_tool", "A test tool");
        let config = AgentConfig::test("test-agent")
            .with_system_prompt("You are helpful")
            .with_tool(tool);

        assert_eq!(config.name, "test-agent");
        assert_eq!(config.system_prompt, Some("You are helpful".to_string()));
        assert_eq!(config.tools.len(), 1);
    }

    #[test]
    fn test_llm_response_serialization() {
        let response = LlmResponse::with_tools(
            "Let me help",
            vec![ToolCall::new(
                "search",
                serde_json::json!({"query": "test"}),
            )],
        );

        let json = serde_json::to_string(&response).unwrap();
        let parsed: LlmResponse = serde_json::from_str(&json).unwrap();

        assert_eq!(parsed.text, "Let me help");
        assert_eq!(parsed.tool_calls.len(), 1);
        assert!(!parsed.is_complete);
    }

    #[test]
    fn test_tool_result_success() {
        let result = ToolResult::success("call_123", serde_json::json!({"data": "value"}));
        assert!(result.result.is_some());
        assert!(result.error.is_none());
    }

    #[test]
    fn test_tool_result_error() {
        let result = ToolResult::error("call_123", "Something went wrong");
        assert!(result.result.is_none());
        assert_eq!(result.error, Some("Something went wrong".to_string()));
    }
}
