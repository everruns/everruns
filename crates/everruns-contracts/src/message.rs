// Message DTOs (PRIMARY conversation data)

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use utoipa::ToSchema;
use uuid::Uuid;

/// Message role
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq, ToSchema)]
#[serde(rename_all = "snake_case")]
pub enum MessageRole {
    User,
    Assistant,
    ToolCall,
    ToolResult,
    System,
}

impl std::fmt::Display for MessageRole {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            MessageRole::User => write!(f, "user"),
            MessageRole::Assistant => write!(f, "assistant"),
            MessageRole::ToolCall => write!(f, "tool_call"),
            MessageRole::ToolResult => write!(f, "tool_result"),
            MessageRole::System => write!(f, "system"),
        }
    }
}

impl From<&str> for MessageRole {
    fn from(s: &str) -> Self {
        match s {
            "assistant" => MessageRole::Assistant,
            "tool_call" => MessageRole::ToolCall,
            "tool_result" => MessageRole::ToolResult,
            "system" => MessageRole::System,
            _ => MessageRole::User,
        }
    }
}

/// Message - primary conversation data
#[derive(Debug, Clone, Serialize, Deserialize, ToSchema)]
pub struct Message {
    pub id: Uuid,
    pub session_id: Uuid,
    pub sequence: i32,
    pub role: MessageRole,
    pub content: serde_json::Value,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub tool_call_id: Option<String>,
    pub created_at: DateTime<Utc>,
}

/// Request to create a message
#[derive(Debug, Clone, Serialize, Deserialize, ToSchema)]
pub struct CreateMessageRequest {
    pub role: MessageRole,
    pub content: serde_json::Value,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub tool_call_id: Option<String>,
}

/// Text content for user/assistant/system messages
#[derive(Debug, Clone, Serialize, Deserialize, ToSchema)]
pub struct TextContent {
    pub text: String,
}

/// Tool call content
#[derive(Debug, Clone, Serialize, Deserialize, ToSchema)]
pub struct ToolCallContent {
    pub id: String,
    pub name: String,
    pub arguments: serde_json::Value,
}

/// Tool result content
#[derive(Debug, Clone, Serialize, Deserialize, ToSchema)]
pub struct ToolResultContent {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub result: Option<serde_json::Value>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub error: Option<String>,
}
