// Message DTOs (PRIMARY conversation data)
//
// New contract design:
// - Content is an array of ContentPart (text, image, etc.)
// - Messages have optional metadata (locale, etc.)
// - CreateMessageRequest includes controls and request-level metadata

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
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

// ============================================
// Content Parts - discriminated union for message content
// ============================================

/// Content part type discriminator
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq, ToSchema)]
#[serde(rename_all = "snake_case")]
pub enum ContentPartType {
    Text,
    Image,
    ToolCall,
    ToolResult,
}

/// A part of message content - can be text, image, tool_call, or tool_result
#[derive(Debug, Clone, Serialize, Deserialize, ToSchema)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum ContentPart {
    /// Text content
    Text { text: String },
    /// Image content (base64 or URL)
    Image {
        #[serde(skip_serializing_if = "Option::is_none")]
        url: Option<String>,
        #[serde(skip_serializing_if = "Option::is_none")]
        base64: Option<String>,
        #[serde(skip_serializing_if = "Option::is_none")]
        media_type: Option<String>,
    },
    /// Tool call content (assistant requesting tool execution)
    ToolCall {
        id: String,
        name: String,
        arguments: serde_json::Value,
    },
    /// Tool result content (result of tool execution)
    ToolResult {
        #[serde(skip_serializing_if = "Option::is_none")]
        result: Option<serde_json::Value>,
        #[serde(skip_serializing_if = "Option::is_none")]
        error: Option<String>,
    },
}

impl ContentPart {
    /// Create a text content part
    pub fn text(text: impl Into<String>) -> Self {
        ContentPart::Text { text: text.into() }
    }

    /// Create an image content part from URL
    pub fn image_url(url: impl Into<String>) -> Self {
        ContentPart::Image {
            url: Some(url.into()),
            base64: None,
            media_type: None,
        }
    }

    /// Create an image content part from base64
    pub fn image_base64(base64: impl Into<String>, media_type: impl Into<String>) -> Self {
        ContentPart::Image {
            url: None,
            base64: Some(base64.into()),
            media_type: Some(media_type.into()),
        }
    }

    /// Create a tool call content part
    pub fn tool_call(
        id: impl Into<String>,
        name: impl Into<String>,
        arguments: serde_json::Value,
    ) -> Self {
        ContentPart::ToolCall {
            id: id.into(),
            name: name.into(),
            arguments,
        }
    }

    /// Create a tool result content part
    pub fn tool_result(result: Option<serde_json::Value>, error: Option<String>) -> Self {
        ContentPart::ToolResult { result, error }
    }

    /// Get text if this is a text part
    pub fn as_text(&self) -> Option<&str> {
        match self {
            ContentPart::Text { text } => Some(text),
            _ => None,
        }
    }
}

// ============================================
// Controls - runtime controls for message processing
// ============================================

/// Reasoning configuration for the model
#[derive(Debug, Clone, Serialize, Deserialize, ToSchema)]
pub struct ReasoningConfig {
    /// Effort level for reasoning (low, medium, high)
    #[serde(skip_serializing_if = "Option::is_none")]
    pub effort: Option<String>,
}

/// Runtime controls for message processing
#[derive(Debug, Clone, Serialize, Deserialize, Default, ToSchema)]
pub struct Controls {
    /// Model ID to use for this message (format: "provider/model-name")
    #[serde(skip_serializing_if = "Option::is_none")]
    pub model_id: Option<String>,

    /// Reasoning configuration
    #[serde(skip_serializing_if = "Option::is_none")]
    pub reasoning: Option<ReasoningConfig>,

    /// Maximum tokens to generate
    #[serde(skip_serializing_if = "Option::is_none")]
    pub max_tokens: Option<i32>,

    /// Temperature for generation
    #[serde(skip_serializing_if = "Option::is_none")]
    pub temperature: Option<f32>,
}

// ============================================
// Message - primary conversation data (response)
// ============================================

/// Message - primary conversation data
#[derive(Debug, Clone, Serialize, Deserialize, ToSchema)]
pub struct Message {
    pub id: Uuid,
    pub session_id: Uuid,
    pub sequence: i32,
    pub role: MessageRole,
    /// Array of content parts
    pub content: Vec<ContentPart>,
    /// Message-level metadata (locale, etc.)
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub metadata: Option<HashMap<String, serde_json::Value>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub tool_call_id: Option<String>,
    pub created_at: DateTime<Utc>,
}

// ============================================
// Message Input - the message part of CreateMessageRequest
// ============================================

/// Message input for creating a message
#[derive(Debug, Clone, Serialize, Deserialize, ToSchema)]
pub struct MessageInput {
    /// Message role
    pub role: MessageRole,
    /// Array of content parts
    pub content: Vec<ContentPart>,
    /// Message-level metadata (locale, etc.)
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub metadata: Option<HashMap<String, serde_json::Value>>,
    /// Tool call ID (for tool_result messages)
    #[serde(skip_serializing_if = "Option::is_none")]
    pub tool_call_id: Option<String>,
}

// ============================================
// CreateMessageRequest - full request structure
// ============================================

/// Request to create a message
#[derive(Debug, Clone, Serialize, Deserialize, ToSchema)]
pub struct CreateMessageRequest {
    /// The message to create
    pub message: MessageInput,
    /// Runtime controls (model, reasoning, etc.)
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub controls: Option<Controls>,
    /// Request-level metadata
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub metadata: Option<HashMap<String, serde_json::Value>>,
    /// Tags for the message
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub tags: Option<Vec<String>>,
}

impl CreateMessageRequest {
    /// Create a simple text message request
    pub fn text(role: MessageRole, text: impl Into<String>) -> Self {
        Self {
            message: MessageInput {
                role,
                content: vec![ContentPart::text(text)],
                metadata: None,
                tool_call_id: None,
            },
            controls: None,
            metadata: None,
            tags: None,
        }
    }

    /// Create a user message with text
    pub fn user(text: impl Into<String>) -> Self {
        Self::text(MessageRole::User, text)
    }
}

// ============================================
// Legacy content types (kept for backwards compatibility)
// ============================================

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

// ============================================
// Tests
// ============================================

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_content_part_text_serialization() {
        let part = ContentPart::text("Hello, world!");
        let json = serde_json::to_string(&part).unwrap();
        assert!(json.contains(r#""type":"text""#));
        assert!(json.contains(r#""text":"Hello, world!""#));
    }

    #[test]
    fn test_content_part_image_url_serialization() {
        let part = ContentPart::image_url("https://example.com/image.png");
        let json = serde_json::to_string(&part).unwrap();
        assert!(json.contains(r#""type":"image""#));
        assert!(json.contains(r#""url":"https://example.com/image.png""#));
    }

    #[test]
    fn test_content_part_tool_call_serialization() {
        let part = ContentPart::tool_call(
            "call_123",
            "get_weather",
            serde_json::json!({"city": "Tokyo"}),
        );
        let json = serde_json::to_string(&part).unwrap();
        assert!(json.contains(r#""type":"tool_call""#));
        assert!(json.contains(r#""id":"call_123""#));
        assert!(json.contains(r#""name":"get_weather""#));
    }

    #[test]
    fn test_content_part_deserialization() {
        let json = r#"{"type":"text","text":"Hello!"}"#;
        let part: ContentPart = serde_json::from_str(json).unwrap();
        assert_eq!(part.as_text(), Some("Hello!"));
    }

    #[test]
    fn test_create_message_request_user() {
        let req = CreateMessageRequest::user("Hello, how are you?");
        assert_eq!(req.message.role, MessageRole::User);
        assert_eq!(req.message.content.len(), 1);
        assert_eq!(
            req.message.content[0].as_text(),
            Some("Hello, how are you?")
        );
    }

    #[test]
    fn test_create_message_request_with_controls() {
        let req = CreateMessageRequest {
            message: MessageInput {
                role: MessageRole::User,
                content: vec![ContentPart::text("Hello")],
                metadata: Some(HashMap::from([(
                    "locale".to_string(),
                    serde_json::json!("en-US"),
                )])),
                tool_call_id: None,
            },
            controls: Some(Controls {
                model_id: Some("anthropic/claude-3-5-sonnet".to_string()),
                reasoning: Some(ReasoningConfig {
                    effort: Some("medium".to_string()),
                }),
                max_tokens: None,
                temperature: None,
            }),
            metadata: None,
            tags: Some(vec!["important".to_string()]),
        };

        let json = serde_json::to_string(&req).unwrap();
        assert!(json.contains(r#""model_id":"anthropic/claude-3-5-sonnet""#));
        assert!(json.contains(r#""locale":"en-US""#));
        assert!(json.contains(r#""important""#));
    }

    #[test]
    fn test_message_role_display() {
        assert_eq!(MessageRole::User.to_string(), "user");
        assert_eq!(MessageRole::Assistant.to_string(), "assistant");
        assert_eq!(MessageRole::ToolCall.to_string(), "tool_call");
        assert_eq!(MessageRole::ToolResult.to_string(), "tool_result");
        assert_eq!(MessageRole::System.to_string(), "system");
    }

    #[test]
    fn test_message_role_from_str() {
        assert_eq!(MessageRole::from("user"), MessageRole::User);
        assert_eq!(MessageRole::from("assistant"), MessageRole::Assistant);
        assert_eq!(MessageRole::from("tool_call"), MessageRole::ToolCall);
        assert_eq!(MessageRole::from("unknown"), MessageRole::User);
    }
}
