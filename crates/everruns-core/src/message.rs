// Message types
//
// Message is a DB-agnostic message type that represents
// a single message in the conversation history.

use crate::tool_types::ToolCall;
use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use uuid::Uuid;

/// Message role in the conversation
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum MessageRole {
    /// System message (instructions)
    System,
    /// User message
    User,
    /// Assistant response
    Assistant,
    /// Tool call request from assistant
    ToolCall,
    /// Tool execution result
    ToolResult,
}

impl std::fmt::Display for MessageRole {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            MessageRole::System => write!(f, "system"),
            MessageRole::User => write!(f, "user"),
            MessageRole::Assistant => write!(f, "assistant"),
            MessageRole::ToolCall => write!(f, "tool_call"),
            MessageRole::ToolResult => write!(f, "tool_result"),
        }
    }
}

impl From<&str> for MessageRole {
    fn from(s: &str) -> Self {
        match s.to_lowercase().as_str() {
            "system" => MessageRole::System,
            "user" => MessageRole::User,
            "assistant" => MessageRole::Assistant,
            "tool_call" => MessageRole::ToolCall,
            "tool_result" => MessageRole::ToolResult,
            _ => MessageRole::User,
        }
    }
}

/// A message in the conversation
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Message {
    /// Unique message ID
    pub id: Uuid,

    /// Message role
    pub role: MessageRole,

    /// Message content (text for user/assistant/system, structured for tool calls/results)
    pub content: MessageContent,

    /// Tool call ID (for tool_result messages to correlate with tool_call)
    #[serde(skip_serializing_if = "Option::is_none")]
    pub tool_call_id: Option<String>,

    /// Tool calls requested by assistant (for assistant messages with function calls)
    #[serde(skip_serializing_if = "Option::is_none")]
    pub tool_calls: Option<Vec<ToolCall>>,

    /// Timestamp when the message was created
    pub created_at: DateTime<Utc>,
}

/// Message content variants
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(untagged)]
pub enum MessageContent {
    /// Text content (for user/assistant/system messages)
    Text(String),

    /// Tool call content
    ToolCall {
        id: String,
        name: String,
        arguments: serde_json::Value,
    },

    /// Tool result content
    ToolResult {
        result: Option<serde_json::Value>,
        error: Option<String>,
    },
}

impl MessageContent {
    /// Get text content if this is a text message
    pub fn as_text(&self) -> Option<&str> {
        match self {
            MessageContent::Text(s) => Some(s),
            _ => None,
        }
    }

    /// Convert to text representation for LLM
    pub fn to_llm_string(&self) -> String {
        match self {
            MessageContent::Text(s) => s.clone(),
            MessageContent::ToolCall {
                name, arguments, ..
            } => {
                format!(
                    "Tool call: {} with arguments: {}",
                    name,
                    serde_json::to_string(arguments).unwrap_or_default()
                )
            }
            MessageContent::ToolResult { result, error } => {
                if let Some(err) = error {
                    format!("Tool error: {}", err)
                } else if let Some(res) = result {
                    serde_json::to_string(res).unwrap_or_else(|_| "{}".to_string())
                } else {
                    "{}".to_string()
                }
            }
        }
    }
}

impl Message {
    /// Create a new user message
    pub fn user(content: impl Into<String>) -> Self {
        Self {
            id: Uuid::now_v7(),
            role: MessageRole::User,
            content: MessageContent::Text(content.into()),
            tool_call_id: None,
            tool_calls: None,
            created_at: Utc::now(),
        }
    }

    /// Create a new assistant message
    pub fn assistant(content: impl Into<String>) -> Self {
        Self {
            id: Uuid::now_v7(),
            role: MessageRole::Assistant,
            content: MessageContent::Text(content.into()),
            tool_call_id: None,
            tool_calls: None,
            created_at: Utc::now(),
        }
    }

    /// Create a new assistant message with tool calls
    pub fn assistant_with_tools(content: impl Into<String>, tool_calls: Vec<ToolCall>) -> Self {
        Self {
            id: Uuid::now_v7(),
            role: MessageRole::Assistant,
            content: MessageContent::Text(content.into()),
            tool_call_id: None,
            tool_calls: Some(tool_calls),
            created_at: Utc::now(),
        }
    }

    /// Create a new system message
    pub fn system(content: impl Into<String>) -> Self {
        Self {
            id: Uuid::now_v7(),
            role: MessageRole::System,
            content: MessageContent::Text(content.into()),
            tool_call_id: None,
            tool_calls: None,
            created_at: Utc::now(),
        }
    }

    /// Create a tool call message
    pub fn tool_call(tool_call: &ToolCall) -> Self {
        Self {
            id: Uuid::now_v7(),
            role: MessageRole::ToolCall,
            content: MessageContent::ToolCall {
                id: tool_call.id.clone(),
                name: tool_call.name.clone(),
                arguments: tool_call.arguments.clone(),
            },
            tool_call_id: Some(tool_call.id.clone()),
            tool_calls: None,
            created_at: Utc::now(),
        }
    }

    /// Create a tool result message
    pub fn tool_result(
        tool_call_id: impl Into<String>,
        result: Option<serde_json::Value>,
        error: Option<String>,
    ) -> Self {
        Self {
            id: Uuid::now_v7(),
            role: MessageRole::ToolResult,
            content: MessageContent::ToolResult { result, error },
            tool_call_id: Some(tool_call_id.into()),
            tool_calls: None,
            created_at: Utc::now(),
        }
    }

    /// Get text content if this is a text message
    pub fn text(&self) -> Option<&str> {
        self.content.as_text()
    }

    /// Check if this message has tool calls
    pub fn has_tool_calls(&self) -> bool {
        self.tool_calls.as_ref().is_some_and(|tc| !tc.is_empty())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_user_message() {
        let msg = Message::user("Hello");
        assert_eq!(msg.role, MessageRole::User);
        assert_eq!(msg.text(), Some("Hello"));
    }

    #[test]
    fn test_assistant_message() {
        let msg = Message::assistant("Hi there!");
        assert_eq!(msg.role, MessageRole::Assistant);
        assert_eq!(msg.text(), Some("Hi there!"));
    }

    #[test]
    fn test_tool_result_message() {
        let msg = Message::tool_result(
            "call_123",
            Some(serde_json::json!({"result": "success"})),
            None,
        );
        assert_eq!(msg.role, MessageRole::ToolResult);
        assert_eq!(msg.tool_call_id, Some("call_123".to_string()));
    }
}
