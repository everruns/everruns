// Message types
//
// Message is a DB-agnostic message type that represents
// a single message in the conversation history.
//
// Content is stored as Vec<ContentPart> for unified representation
// across storage and runtime layers.

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use uuid::Uuid;

#[cfg(feature = "openapi")]
use utoipa::ToSchema;

/// Message role in the conversation
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "openapi", derive(ToSchema))]
#[serde(rename_all = "snake_case")]
pub enum MessageRole {
    /// System message (instructions)
    System,
    /// User message
    User,
    /// Agent response (may contain tool calls in content)
    /// Note: Serializes as "agent" for API consistency
    #[serde(rename = "agent")]
    Assistant,
    /// Tool execution result
    ToolResult,
}

impl std::fmt::Display for MessageRole {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            MessageRole::System => write!(f, "system"),
            MessageRole::User => write!(f, "user"),
            MessageRole::Assistant => write!(f, "agent"),
            MessageRole::ToolResult => write!(f, "tool_result"),
        }
    }
}

impl From<&str> for MessageRole {
    fn from(s: &str) -> Self {
        match s.to_lowercase().as_str() {
            "system" => MessageRole::System,
            "user" => MessageRole::User,
            // Accept both "agent" and legacy "assistant"
            "agent" | "assistant" => MessageRole::Assistant,
            "tool_result" => MessageRole::ToolResult,
            _ => MessageRole::User,
        }
    }
}

// ============================================
// Controls (runtime options for message processing)
// ============================================

/// Reasoning configuration for the model
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[cfg_attr(feature = "openapi", derive(ToSchema))]
pub struct ReasoningConfig {
    /// Effort level for reasoning (low, medium, high)
    #[serde(skip_serializing_if = "Option::is_none")]
    pub effort: Option<String>,
}

/// Runtime controls for message processing
#[derive(Debug, Clone, Serialize, Deserialize, Default, PartialEq)]
#[cfg_attr(feature = "openapi", derive(ToSchema))]
pub struct Controls {
    /// Model ID (UUID) to use for this message.
    /// Overrides session and agent model settings.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub model_id: Option<Uuid>,

    /// Reasoning configuration
    #[serde(skip_serializing_if = "Option::is_none")]
    pub reasoning: Option<ReasoningConfig>,
}

/// A message in the conversation
#[derive(Debug, Clone, Serialize, Deserialize)]
#[cfg_attr(feature = "openapi", derive(ToSchema))]
pub struct Message {
    /// Unique message ID
    pub id: Uuid,

    /// Message role
    pub role: MessageRole,

    /// Message content as array of content parts (text, images, tool calls, tool results)
    pub content: Vec<ContentPart>,

    /// Runtime controls (model, reasoning, etc.)
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub controls: Option<Controls>,

    /// Message-level metadata
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[cfg_attr(feature = "openapi", schema(value_type = Option<Object>))]
    pub metadata: Option<std::collections::HashMap<String, serde_json::Value>>,

    /// Timestamp when the message was created
    pub created_at: DateTime<Utc>,
}

// ============================================
// Content Type Enum
// ============================================

/// Content type discriminator
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "openapi", derive(ToSchema))]
#[serde(rename_all = "snake_case")]
pub enum ContentType {
    Text,
    Image,
    ToolCall,
    ToolResult,
}

impl std::fmt::Display for ContentType {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            ContentType::Text => write!(f, "text"),
            ContentType::Image => write!(f, "image"),
            ContentType::ToolCall => write!(f, "tool_call"),
            ContentType::ToolResult => write!(f, "tool_result"),
        }
    }
}

impl From<&str> for ContentType {
    fn from(s: &str) -> Self {
        match s {
            "image" => ContentType::Image,
            "tool_call" => ContentType::ToolCall,
            "tool_result" => ContentType::ToolResult,
            _ => ContentType::Text,
        }
    }
}

// ============================================
// Content Part Structs
// ============================================

/// Text content part
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[cfg_attr(feature = "openapi", derive(ToSchema))]
pub struct TextContentPart {
    pub text: String,
}

impl TextContentPart {
    pub fn new(text: impl Into<String>) -> Self {
        Self { text: text.into() }
    }
}

/// Image content part (base64 or URL)
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[cfg_attr(feature = "openapi", derive(ToSchema))]
pub struct ImageContentPart {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub url: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub base64: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub media_type: Option<String>,
}

impl ImageContentPart {
    pub fn from_url(url: impl Into<String>) -> Self {
        Self {
            url: Some(url.into()),
            base64: None,
            media_type: None,
        }
    }

    pub fn from_base64(base64: impl Into<String>, media_type: impl Into<String>) -> Self {
        Self {
            url: None,
            base64: Some(base64.into()),
            media_type: Some(media_type.into()),
        }
    }
}

/// Tool call content part (assistant requesting tool execution)
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[cfg_attr(feature = "openapi", derive(ToSchema))]
pub struct ToolCallContentPart {
    pub id: String,
    pub name: String,
    pub arguments: serde_json::Value,
}

impl ToolCallContentPart {
    pub fn new(
        id: impl Into<String>,
        name: impl Into<String>,
        arguments: serde_json::Value,
    ) -> Self {
        Self {
            id: id.into(),
            name: name.into(),
            arguments,
        }
    }
}

/// Tool result content part (result of tool execution)
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[cfg_attr(feature = "openapi", derive(ToSchema))]
pub struct ToolResultContentPart {
    /// ID of the tool call this result corresponds to
    pub tool_call_id: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub result: Option<serde_json::Value>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub error: Option<String>,
}

impl ToolResultContentPart {
    pub fn new(
        tool_call_id: impl Into<String>,
        result: Option<serde_json::Value>,
        error: Option<String>,
    ) -> Self {
        Self {
            tool_call_id: tool_call_id.into(),
            result,
            error,
        }
    }

    pub fn success(tool_call_id: impl Into<String>, result: serde_json::Value) -> Self {
        Self {
            tool_call_id: tool_call_id.into(),
            result: Some(result),
            error: None,
        }
    }

    pub fn error(tool_call_id: impl Into<String>, error: impl Into<String>) -> Self {
        Self {
            tool_call_id: tool_call_id.into(),
            result: None,
            error: Some(error.into()),
        }
    }
}

// ============================================
// Content Part Enums
// ============================================

/// A part of message content - can be text, image, tool_call, or tool_result
///
/// This is the canonical content part type used across the system.
/// API layer enables the "openapi" feature to add ToSchema derive.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[cfg_attr(feature = "openapi", derive(ToSchema))]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum ContentPart {
    /// Text content
    Text(TextContentPart),
    /// Image content (base64 or URL)
    Image(ImageContentPart),
    /// Tool call content (assistant requesting tool execution)
    ToolCall(ToolCallContentPart),
    /// Tool result content (result of tool execution)
    ToolResult(ToolResultContentPart),
}

impl ContentPart {
    /// Create a text content part
    pub fn text(text: impl Into<String>) -> Self {
        ContentPart::Text(TextContentPart::new(text))
    }

    /// Create an image content part from URL
    pub fn image_url(url: impl Into<String>) -> Self {
        ContentPart::Image(ImageContentPart::from_url(url))
    }

    /// Create a tool call content part
    pub fn tool_call(
        id: impl Into<String>,
        name: impl Into<String>,
        arguments: serde_json::Value,
    ) -> Self {
        ContentPart::ToolCall(ToolCallContentPart::new(id, name, arguments))
    }

    /// Create a tool result content part
    pub fn tool_result(
        tool_call_id: impl Into<String>,
        result: Option<serde_json::Value>,
        error: Option<String>,
    ) -> Self {
        ContentPart::ToolResult(ToolResultContentPart::new(tool_call_id, result, error))
    }

    /// Get text if this is a text part
    pub fn as_text(&self) -> Option<&str> {
        match self {
            ContentPart::Text(t) => Some(&t.text),
            _ => None,
        }
    }

    /// Get the content type
    pub fn content_type(&self) -> ContentType {
        match self {
            ContentPart::Text(_) => ContentType::Text,
            ContentPart::Image(_) => ContentType::Image,
            ContentPart::ToolCall(_) => ContentType::ToolCall,
            ContentPart::ToolResult(_) => ContentType::ToolResult,
        }
    }
}

/// Input content part - only text and image (for user input)
///
/// This is a subset of ContentPart that users can send.
/// Tool calls and results are system-generated.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[cfg_attr(feature = "openapi", derive(ToSchema))]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum InputContentPart {
    /// Text content
    Text(TextContentPart),
    /// Image content (base64 or URL)
    Image(ImageContentPart),
}

impl From<InputContentPart> for ContentPart {
    fn from(input: InputContentPart) -> Self {
        match input {
            InputContentPart::Text(t) => ContentPart::Text(t),
            InputContentPart::Image(i) => ContentPart::Image(i),
        }
    }
}

impl InputContentPart {
    /// Create a text content part
    pub fn text(text: impl Into<String>) -> Self {
        InputContentPart::Text(TextContentPart::new(text))
    }

    /// Create an image content part from URL
    pub fn image_url(url: impl Into<String>) -> Self {
        InputContentPart::Image(ImageContentPart::from_url(url))
    }

    /// Get text content if this is a Text part
    pub fn as_text(&self) -> Option<&str> {
        match self {
            InputContentPart::Text(t) => Some(&t.text),
            _ => None,
        }
    }

    /// Get the content type
    pub fn content_type(&self) -> ContentType {
        match self {
            InputContentPart::Text(_) => ContentType::Text,
            InputContentPart::Image(_) => ContentType::Image,
        }
    }
}

impl Message {
    /// Create a new user message
    pub fn user(content: impl Into<String>) -> Self {
        Self {
            id: Uuid::now_v7(),
            role: MessageRole::User,
            content: vec![ContentPart::text(content)],
            controls: None,
            metadata: None,
            created_at: Utc::now(),
        }
    }

    /// Create a new assistant message
    pub fn assistant(content: impl Into<String>) -> Self {
        Self {
            id: Uuid::now_v7(),
            role: MessageRole::Assistant,
            content: vec![ContentPart::text(content)],
            controls: None,
            metadata: None,
            created_at: Utc::now(),
        }
    }

    /// Create a new assistant message with tool calls
    ///
    /// Tool calls are stored as ContentPart::ToolCall in the content array
    /// alongside the text content.
    pub fn assistant_with_tools(
        content: impl Into<String>,
        tool_calls: Vec<crate::tool_types::ToolCall>,
    ) -> Self {
        let mut parts = vec![ContentPart::text(content)];
        for tc in tool_calls {
            parts.push(ContentPart::ToolCall(ToolCallContentPart {
                id: tc.id,
                name: tc.name,
                arguments: tc.arguments,
            }));
        }
        Self {
            id: Uuid::now_v7(),
            role: MessageRole::Assistant,
            content: parts,
            controls: None,
            metadata: None,
            created_at: Utc::now(),
        }
    }

    /// Create a new system message
    pub fn system(content: impl Into<String>) -> Self {
        Self {
            id: Uuid::now_v7(),
            role: MessageRole::System,
            content: vec![ContentPart::text(content)],
            controls: None,
            metadata: None,
            created_at: Utc::now(),
        }
    }

    /// Create a tool result message
    pub fn tool_result(
        tool_call_id: impl Into<String>,
        result: Option<serde_json::Value>,
        error: Option<String>,
    ) -> Self {
        let tool_call_id = tool_call_id.into();
        Self {
            id: Uuid::now_v7(),
            role: MessageRole::ToolResult,
            content: vec![ContentPart::ToolResult(ToolResultContentPart::new(
                tool_call_id,
                result,
                error,
            ))],
            controls: None,
            metadata: None,
            created_at: Utc::now(),
        }
    }

    /// Get the tool_call_id from a tool result message
    ///
    /// Returns the tool_call_id from the first ToolResult content part, if any.
    pub fn tool_call_id(&self) -> Option<&str> {
        self.content.iter().find_map(|p| match p {
            ContentPart::ToolResult(tr) => Some(tr.tool_call_id.as_str()),
            _ => None,
        })
    }

    /// Get first text content from the message
    pub fn text(&self) -> Option<&str> {
        self.content.iter().find_map(|p| p.as_text())
    }

    /// Get all tool calls from the message content
    pub fn tool_calls(&self) -> Vec<&ToolCallContentPart> {
        self.content
            .iter()
            .filter_map(|p| match p {
                ContentPart::ToolCall(tc) => Some(tc),
                _ => None,
            })
            .collect()
    }

    /// Check if this message has tool calls
    pub fn has_tool_calls(&self) -> bool {
        self.content
            .iter()
            .any(|p| matches!(p, ContentPart::ToolCall(_)))
    }

    /// Get the first tool result from the message content
    pub fn tool_result_content(&self) -> Option<&ToolResultContentPart> {
        self.content.iter().find_map(|p| match p {
            ContentPart::ToolResult(tr) => Some(tr),
            _ => None,
        })
    }

    /// Convert content to LLM-compatible string representation
    pub fn content_to_llm_string(&self) -> String {
        self.content
            .iter()
            .map(|part| match part {
                ContentPart::Text(t) => t.text.clone(),
                ContentPart::Image(_) => "[Image]".to_string(),
                ContentPart::ToolCall(tc) => {
                    format!(
                        "Tool call: {} with arguments: {}",
                        tc.name,
                        serde_json::to_string(&tc.arguments).unwrap_or_default()
                    )
                }
                ContentPart::ToolResult(tr) => {
                    if let Some(err) = &tr.error {
                        format!("Tool error: {}", err)
                    } else if let Some(res) = &tr.result {
                        serde_json::to_string(res).unwrap_or_else(|_| "{}".to_string())
                    } else {
                        "{}".to_string()
                    }
                }
            })
            .collect::<Vec<_>>()
            .join("\n")
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
        assert_eq!(msg.tool_call_id(), Some("call_123"));
    }
}
