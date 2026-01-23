// Message types
//
// Message is a DB-agnostic message type that represents
// a single message in the conversation history.
//
// Content is stored as Vec<ContentPart> for unified representation
// across storage and runtime layers.

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};

use crate::typed_id::{ImageId, MessageId, ModelId};

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
    /// Model ID to use for this message (format: model_{32-hex}).
    /// Overrides session and agent model settings.
    #[serde(skip_serializing_if = "Option::is_none")]
    #[cfg_attr(feature = "openapi", schema(value_type = Option<String>, example = "model_01933b5a00007000800000000000001"))]
    pub model_id: Option<ModelId>,

    /// Reasoning configuration
    #[serde(skip_serializing_if = "Option::is_none")]
    pub reasoning: Option<ReasoningConfig>,
}

/// A message in the conversation
#[derive(Debug, Clone, Serialize, Deserialize)]
#[cfg_attr(feature = "openapi", derive(ToSchema))]
pub struct Message {
    /// Unique message ID (format: message_{32-hex})
    #[cfg_attr(feature = "openapi", schema(value_type = String, example = "message_01933b5a00007000800000000000001"))]
    pub id: MessageId,

    /// Message role
    pub role: MessageRole,

    /// Message content as array of content parts (text, images, tool calls, tool results)
    pub content: Vec<ContentPart>,

    /// Thinking content from extended thinking models (Anthropic Claude)
    /// This is the model's chain-of-thought reasoning before producing the response.
    /// Must be included in subsequent API calls when thinking is enabled.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub thinking: Option<String>,

    /// Cryptographic signature for thinking content (Anthropic Claude)
    /// Required when sending thinking back in subsequent API calls.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub thinking_signature: Option<String>,

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
    ImageFile,
    ToolCall,
    ToolResult,
}

impl std::fmt::Display for ContentType {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            ContentType::Text => write!(f, "text"),
            ContentType::Image => write!(f, "image"),
            ContentType::ImageFile => write!(f, "image_file"),
            ContentType::ToolCall => write!(f, "tool_call"),
            ContentType::ToolResult => write!(f, "tool_result"),
        }
    }
}

impl From<&str> for ContentType {
    fn from(s: &str) -> Self {
        match s {
            "image" => ContentType::Image,
            "image_file" => ContentType::ImageFile,
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

/// Image file content part (reference to uploaded image)
///
/// This is used for images uploaded via the /images API.
/// The image data is stored separately and referenced by ID.
/// Note: Currently filtered out before sending to LLM.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[cfg_attr(feature = "openapi", derive(ToSchema))]
pub struct ImageFileContentPart {
    /// ID of the uploaded image (format: img_{32-hex})
    #[cfg_attr(feature = "openapi", schema(value_type = String, example = "img_01933b5a00007000800000000000001"))]
    pub image_id: ImageId,
    /// Original filename (for display)
    #[serde(skip_serializing_if = "Option::is_none")]
    pub filename: Option<String>,
}

impl ImageFileContentPart {
    pub fn new(image_id: ImageId) -> Self {
        Self {
            image_id,
            filename: None,
        }
    }

    pub fn with_filename(image_id: ImageId, filename: impl Into<String>) -> Self {
        Self {
            image_id,
            filename: Some(filename.into()),
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

/// A part of message content - can be text, image, image_file, tool_call, or tool_result
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
    /// Image file content (reference to uploaded image by ID)
    ImageFile(ImageFileContentPart),
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

    /// Create an image file content part (reference to uploaded image)
    pub fn image_file(image_id: ImageId) -> Self {
        ContentPart::ImageFile(ImageFileContentPart::new(image_id))
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

    /// Check if this is an ImageFile part
    pub fn is_image_file(&self) -> bool {
        matches!(self, ContentPart::ImageFile(_))
    }

    /// Get the content type
    pub fn content_type(&self) -> ContentType {
        match self {
            ContentPart::Text(_) => ContentType::Text,
            ContentPart::Image(_) => ContentType::Image,
            ContentPart::ImageFile(_) => ContentType::ImageFile,
            ContentPart::ToolCall(_) => ContentType::ToolCall,
            ContentPart::ToolResult(_) => ContentType::ToolResult,
        }
    }

    /// Convert content part to OpenAI-compatible format
    ///
    /// Returns `None` for content types that aren't valid in user/system messages
    /// (ImageFile, ToolCall, ToolResult are handled at message level).
    pub fn to_openai_format(&self) -> Option<serde_json::Value> {
        match self {
            ContentPart::Text(t) => Some(serde_json::json!({
                "type": "text",
                "text": t.text
            })),
            ContentPart::Image(img) => {
                if let Some(url) = &img.url {
                    Some(serde_json::json!({
                        "type": "image_url",
                        "image_url": { "url": url }
                    }))
                } else if let Some(b64) = &img.base64 {
                    let media_type = img.media_type.as_deref().unwrap_or("image/png");
                    Some(serde_json::json!({
                        "type": "image_url",
                        "image_url": { "url": format!("data:{};base64,{}", media_type, b64) }
                    }))
                } else {
                    None
                }
            }
            // ImageFile, ToolCall, ToolResult handled at message level
            _ => None,
        }
    }
}

/// Input content part - text, image, and image_file (for user input)
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
    /// Image file content (reference to uploaded image by ID)
    ImageFile(ImageFileContentPart),
}

impl From<InputContentPart> for ContentPart {
    fn from(input: InputContentPart) -> Self {
        match input {
            InputContentPart::Text(t) => ContentPart::Text(t),
            InputContentPart::Image(i) => ContentPart::Image(i),
            InputContentPart::ImageFile(f) => ContentPart::ImageFile(f),
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

    /// Create an image file content part (reference to uploaded image)
    pub fn image_file(image_id: ImageId) -> Self {
        InputContentPart::ImageFile(ImageFileContentPart::new(image_id))
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
            InputContentPart::ImageFile(_) => ContentType::ImageFile,
        }
    }
}

impl Message {
    /// Create a new user message
    pub fn user(content: impl Into<String>) -> Self {
        Self {
            id: MessageId::new(),
            role: MessageRole::User,
            content: vec![ContentPart::text(content)],
            thinking: None,
            thinking_signature: None,
            controls: None,
            metadata: None,
            created_at: Utc::now(),
        }
    }

    /// Create a new assistant message
    pub fn assistant(content: impl Into<String>) -> Self {
        Self {
            id: MessageId::new(),
            role: MessageRole::Assistant,
            content: vec![ContentPart::text(content)],
            thinking: None,
            thinking_signature: None,
            controls: None,
            metadata: None,
            created_at: Utc::now(),
        }
    }

    /// Create a new assistant message with tool calls
    ///
    /// Tool calls are stored as ContentPart::ToolCall in the content array
    /// alongside the text content. Empty text content is omitted to avoid
    /// LLM API errors (e.g., Anthropic requires non-empty text blocks).
    pub fn assistant_with_tools(
        content: impl Into<String>,
        tool_calls: Vec<crate::tool_types::ToolCall>,
    ) -> Self {
        let text_content = content.into();
        let mut parts = Vec::new();
        // Only include text part if non-empty
        if !text_content.is_empty() {
            parts.push(ContentPart::text(text_content));
        }
        for tc in tool_calls {
            parts.push(ContentPart::ToolCall(ToolCallContentPart {
                id: tc.id,
                name: tc.name,
                arguments: tc.arguments,
            }));
        }
        Self {
            id: MessageId::new(),
            role: MessageRole::Assistant,
            content: parts,
            thinking: None,
            thinking_signature: None,
            controls: None,
            metadata: None,
            created_at: Utc::now(),
        }
    }

    /// Create a new system message
    pub fn system(content: impl Into<String>) -> Self {
        Self {
            id: MessageId::new(),
            role: MessageRole::System,
            content: vec![ContentPart::text(content)],
            thinking: None,
            thinking_signature: None,
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
            id: MessageId::new(),
            role: MessageRole::ToolResult,
            content: vec![ContentPart::ToolResult(ToolResultContentPart::new(
                tool_call_id,
                result,
                error,
            ))],
            thinking: None,
            thinking_signature: None,
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
                ContentPart::ImageFile(_) => "[Image File]".to_string(),
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

    /// Convert message to OpenAI-compatible format
    ///
    /// Transforms internal message format to OpenAI API format:
    /// - `agent` role → `assistant`
    /// - `tool_result` role → `tool` (with tool_call_id at message level)
    /// - Tool calls formatted as `{id, type: "function", function: {name, arguments}}`
    ///
    /// Used by observability backends (e.g., Braintrust) that expect OpenAI format.
    pub fn to_openai_format(&self) -> serde_json::Value {
        let role = match self.role {
            MessageRole::System => "system",
            MessageRole::User => "user",
            MessageRole::Assistant => "assistant",
            MessageRole::ToolResult => "tool",
        };

        // Handle tool result messages (need tool_call_id at message level)
        if self.role == MessageRole::ToolResult {
            let tool_call_id = self.tool_call_id().unwrap_or("");
            let content = self
                .content
                .iter()
                .find_map(|p| match p {
                    ContentPart::ToolResult(tr) => {
                        if let Some(error) = &tr.error {
                            Some(format!("Error: {}", error))
                        } else if let Some(result) = &tr.result {
                            Some(serde_json::to_string(result).unwrap_or_else(|_| "{}".to_string()))
                        } else {
                            Some("{}".to_string())
                        }
                    }
                    _ => None,
                })
                .unwrap_or_else(|| "{}".to_string());

            return serde_json::json!({
                "role": role,
                "content": content,
                "tool_call_id": tool_call_id
            });
        }

        // Handle assistant messages with tool calls
        if self.role == MessageRole::Assistant {
            let tool_calls: Vec<serde_json::Value> = self
                .content
                .iter()
                .filter_map(|p| match p {
                    ContentPart::ToolCall(tc) => Some(serde_json::json!({
                        "id": tc.id,
                        "type": "function",
                        "function": {
                            "name": tc.name,
                            "arguments": serde_json::to_string(&tc.arguments).unwrap_or_else(|_| "{}".to_string())
                        }
                    })),
                    _ => None,
                })
                .collect();

            let text_content: String = self
                .content
                .iter()
                .filter_map(|p| match p {
                    ContentPart::Text(t) => Some(t.text.clone()),
                    _ => None,
                })
                .collect::<Vec<_>>()
                .join("\n");

            if tool_calls.is_empty() {
                return serde_json::json!({
                    "role": role,
                    "content": text_content
                });
            } else {
                let mut result = serde_json::json!({
                    "role": role,
                    "tool_calls": tool_calls
                });
                if !text_content.is_empty() {
                    result["content"] = serde_json::json!(text_content);
                }
                return result;
            }
        }

        // For system/user messages, convert content parts
        let content = self.content_to_openai_format();
        serde_json::json!({
            "role": role,
            "content": content
        })
    }

    /// Convert content parts to OpenAI-compatible format
    fn content_to_openai_format(&self) -> serde_json::Value {
        // Single text content → string
        if self.content.len() == 1
            && let ContentPart::Text(t) = &self.content[0]
        {
            return serde_json::json!(t.text);
        }

        // Convert each content part
        let parts: Vec<serde_json::Value> = self
            .content
            .iter()
            .filter_map(|part| part.to_openai_format())
            .collect();

        if parts.is_empty() {
            return serde_json::json!("");
        }

        // Single text part after filtering → string
        if parts.len() == 1
            && let Some(text) = parts[0].get("text")
        {
            return text.clone();
        }

        serde_json::json!(parts)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::tool_types::ToolCall;

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

    #[test]
    fn test_assistant_with_tools_and_text() {
        let tool_call = ToolCall {
            id: "call_123".to_string(),
            name: "get_weather".to_string(),
            arguments: serde_json::json!({"location": "Tokyo"}),
        };
        let msg = Message::assistant_with_tools("Let me check the weather.", vec![tool_call]);

        assert_eq!(msg.role, MessageRole::Assistant);
        assert_eq!(msg.text(), Some("Let me check the weather."));
        assert_eq!(msg.tool_calls().len(), 1);
        assert_eq!(msg.tool_calls()[0].name, "get_weather");
    }

    #[test]
    fn test_assistant_with_tools_empty_text() {
        // When LLM returns only tool calls without text, we shouldn't include an empty text block
        // This is important for Anthropic API which rejects empty text content blocks
        let tool_call = ToolCall {
            id: "call_123".to_string(),
            name: "search".to_string(),
            arguments: serde_json::json!({"query": "rust"}),
        };
        let msg = Message::assistant_with_tools("", vec![tool_call]);

        assert_eq!(msg.role, MessageRole::Assistant);
        // Empty text should result in None, not Some("")
        assert_eq!(msg.text(), None);
        // But tool calls should still be present
        assert_eq!(msg.tool_calls().len(), 1);
        assert_eq!(msg.tool_calls()[0].name, "search");
        // Content should only have tool_call, no empty text part
        assert_eq!(msg.content.len(), 1);
        assert!(matches!(msg.content[0], ContentPart::ToolCall(_)));
    }

    #[test]
    fn test_assistant_with_tools_whitespace_text() {
        // Whitespace-only text is not empty (could be intentional)
        let tool_call = ToolCall {
            id: "call_456".to_string(),
            name: "fetch".to_string(),
            arguments: serde_json::json!({}),
        };
        let msg = Message::assistant_with_tools("   ", vec![tool_call]);

        // Whitespace text is preserved (not treated as empty)
        assert_eq!(msg.text(), Some("   "));
        assert_eq!(msg.content.len(), 2); // Text + ToolCall
    }

    #[test]
    fn test_assistant_with_multiple_tool_calls() {
        let tool_calls = vec![
            ToolCall {
                id: "call_1".to_string(),
                name: "search".to_string(),
                arguments: serde_json::json!({"q": "a"}),
            },
            ToolCall {
                id: "call_2".to_string(),
                name: "fetch".to_string(),
                arguments: serde_json::json!({"url": "http://example.com"}),
            },
        ];
        let msg = Message::assistant_with_tools("", tool_calls);

        assert_eq!(msg.tool_calls().len(), 2);
        // Only tool calls, no empty text
        assert_eq!(msg.content.len(), 2);
    }

    // =========================================================================
    // OpenAI Format Conversion Tests
    // =========================================================================

    #[test]
    fn test_to_openai_format_user_message() {
        let msg = Message::user("Hello, world!");
        let converted = msg.to_openai_format();

        assert_eq!(converted["role"], "user");
        assert_eq!(converted["content"], "Hello, world!");
    }

    #[test]
    fn test_to_openai_format_system_message() {
        let msg = Message::system("You are a helpful assistant.");
        let converted = msg.to_openai_format();

        assert_eq!(converted["role"], "system");
        assert_eq!(converted["content"], "You are a helpful assistant.");
    }

    #[test]
    fn test_to_openai_format_assistant_role_mapping() {
        // Internal "agent" role → "assistant"
        let msg = Message::assistant("Hi there!");
        let converted = msg.to_openai_format();

        assert_eq!(converted["role"], "assistant");
        assert_eq!(converted["content"], "Hi there!");
    }

    #[test]
    fn test_to_openai_format_assistant_with_tool_calls() {
        let tool_call = ToolCall {
            id: "call_123".to_string(),
            name: "get_weather".to_string(),
            arguments: serde_json::json!({"location": "Tokyo"}),
        };
        let msg = Message::assistant_with_tools("Let me check.", vec![tool_call]);
        let converted = msg.to_openai_format();

        assert_eq!(converted["role"], "assistant");
        assert_eq!(converted["content"], "Let me check.");

        let tool_calls = converted["tool_calls"].as_array().unwrap();
        assert_eq!(tool_calls.len(), 1);
        assert_eq!(tool_calls[0]["id"], "call_123");
        assert_eq!(tool_calls[0]["type"], "function");
        assert_eq!(tool_calls[0]["function"]["name"], "get_weather");
        assert_eq!(
            tool_calls[0]["function"]["arguments"],
            r#"{"location":"Tokyo"}"#
        );
    }

    #[test]
    fn test_to_openai_format_assistant_tool_calls_only() {
        // Assistant message with only tool calls (no text)
        let tool_call = ToolCall {
            id: "call_abc".to_string(),
            name: "search".to_string(),
            arguments: serde_json::json!({"query": "rust"}),
        };
        let msg = Message::assistant_with_tools("", vec![tool_call]);
        let converted = msg.to_openai_format();

        assert_eq!(converted["role"], "assistant");
        // No content field when text is empty
        assert!(converted.get("content").is_none());
        assert!(converted["tool_calls"].is_array());
    }

    #[test]
    fn test_to_openai_format_tool_result_role_mapping() {
        // Internal "tool_result" role → "tool"
        let msg = Message::tool_result(
            "call_123",
            Some(serde_json::json!({"temperature": 72})),
            None,
        );
        let converted = msg.to_openai_format();

        assert_eq!(converted["role"], "tool");
        assert_eq!(converted["tool_call_id"], "call_123");
        assert_eq!(converted["content"], r#"{"temperature":72}"#);
    }

    #[test]
    fn test_to_openai_format_tool_result_error() {
        let msg = Message::tool_result("call_456", None, Some("API timeout".to_string()));
        let converted = msg.to_openai_format();

        assert_eq!(converted["role"], "tool");
        assert_eq!(converted["tool_call_id"], "call_456");
        assert_eq!(converted["content"], "Error: API timeout");
    }

    #[test]
    fn test_to_openai_format_full_conversation() {
        // Full conversation: user → assistant (tool call) → tool result → assistant
        let tool_call = ToolCall {
            id: "call_abc".to_string(),
            name: "search".to_string(),
            arguments: serde_json::json!({"query": "rust"}),
        };

        let messages = [
            Message::user("Search for rust"),
            Message::assistant_with_tools("", vec![tool_call]),
            Message::tool_result(
                "call_abc",
                Some(serde_json::json!({"results": ["rust-lang.org"]})),
                None,
            ),
            Message::assistant("Here are the search results."),
        ];
        let converted: Vec<_> = messages.iter().map(|m| m.to_openai_format()).collect();

        assert_eq!(converted.len(), 4);
        assert_eq!(converted[0]["role"], "user");
        assert_eq!(converted[1]["role"], "assistant");
        assert!(converted[1]["tool_calls"].is_array());
        assert_eq!(converted[2]["role"], "tool");
        assert_eq!(converted[2]["tool_call_id"], "call_abc");
        assert_eq!(converted[3]["role"], "assistant");
    }

    // =========================================================================
    // ContentPart::to_openai_format Tests
    // =========================================================================

    #[test]
    fn test_content_part_to_openai_format_text() {
        let part = ContentPart::text("Hello");
        let converted = part.to_openai_format().unwrap();

        assert_eq!(converted["type"], "text");
        assert_eq!(converted["text"], "Hello");
    }

    #[test]
    fn test_content_part_to_openai_format_image_url() {
        let part = ContentPart::image_url("https://example.com/img.png");
        let converted = part.to_openai_format().unwrap();

        assert_eq!(converted["type"], "image_url");
        assert_eq!(converted["image_url"]["url"], "https://example.com/img.png");
    }

    #[test]
    fn test_content_part_to_openai_format_image_base64() {
        let part = ContentPart::Image(ImageContentPart::from_base64("abc123", "image/jpeg"));
        let converted = part.to_openai_format().unwrap();

        assert_eq!(converted["type"], "image_url");
        assert_eq!(
            converted["image_url"]["url"],
            "data:image/jpeg;base64,abc123"
        );
    }

    #[test]
    fn test_content_part_to_openai_format_tool_call_returns_none() {
        // ToolCall parts are handled at message level, not content part level
        let part = ContentPart::tool_call("call_1", "search", serde_json::json!({}));
        assert!(part.to_openai_format().is_none());
    }

    #[test]
    fn test_content_part_to_openai_format_tool_result_returns_none() {
        // ToolResult parts are handled at message level
        let part = ContentPart::tool_result("call_1", Some(serde_json::json!({})), None);
        assert!(part.to_openai_format().is_none());
    }
}
