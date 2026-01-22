// OpenAI-Compatible Message Format Conversion
//
// This module converts our internal message format to OpenAI-compatible format,
// which is used by observability backends like Braintrust.
//
// OpenAI format expectations:
// - Roles: "system", "user", "assistant", "tool", "function", "developer"
// - Content: string or array of {type: "text", text: "..."} objects
//
// Our internal format uses:
// - Roles: "system", "user", "agent", "tool_result"
// - Content: array of ContentPart enums
//
// Role mapping:
// - system → system (no change)
// - user → user (no change)
// - agent → assistant
// - tool_result → tool (with tool_call_id)

use crate::message::{ContentPart, Message, MessageRole};
use crate::tool_types::ToolCall;

/// Convert messages to OpenAI-compatible format
///
/// Transforms our internal message format to the format expected by OpenAI-compatible APIs:
/// - `agent` role → `assistant`
/// - `tool_result` role → `tool` (with tool_call_id)
/// - Content parts are flattened to OpenAI content format
pub fn convert_messages_to_openai_format(messages: &[Message]) -> Vec<serde_json::Value> {
    messages
        .iter()
        .map(convert_message_to_openai_format)
        .collect()
}

/// Convert a single message to OpenAI-compatible format
pub fn convert_message_to_openai_format(msg: &Message) -> serde_json::Value {
    // Map roles to OpenAI-compatible values
    let role = match msg.role {
        MessageRole::System => "system",
        MessageRole::User => "user",
        MessageRole::Assistant => "assistant", // Our "agent" → "assistant"
        MessageRole::ToolResult => "tool",     // Our "tool_result" → "tool"
    };

    // Handle tool result messages specially (need tool_call_id at message level)
    if msg.role == MessageRole::ToolResult {
        // Find the tool_call_id from the first ToolResult content part
        let tool_call_id = msg.content.iter().find_map(|p| match p {
            ContentPart::ToolResult(tr) => Some(tr.tool_call_id.as_str()),
            _ => None,
        });

        // Content for tool messages should be the result string
        let content = msg
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
            "tool_call_id": tool_call_id.unwrap_or("")
        });
    }

    // For assistant messages with tool calls, format them in OpenAI style
    if msg.role == MessageRole::Assistant {
        let tool_calls: Vec<serde_json::Value> = msg
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

        // Get text content
        let text_content: String = msg
            .content
            .iter()
            .filter_map(|p| match p {
                ContentPart::Text(t) => Some(t.text.clone()),
                _ => None,
            })
            .collect::<Vec<_>>()
            .join("\n");

        if tool_calls.is_empty() {
            // Simple assistant message
            return serde_json::json!({
                "role": role,
                "content": text_content
            });
        } else {
            // Assistant message with tool calls
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

    // For system/user messages, convert content to OpenAI format
    let content = convert_content_to_openai_format(&msg.content);
    serde_json::json!({
        "role": role,
        "content": content
    })
}

/// Convert content parts to OpenAI-compatible format
pub fn convert_content_to_openai_format(content: &[ContentPart]) -> serde_json::Value {
    // If single text content, use string format
    if content.len() == 1
        && let ContentPart::Text(t) = &content[0]
    {
        return serde_json::json!(t.text);
    }

    // Convert each content part
    let parts: Vec<serde_json::Value> = content
        .iter()
        .filter_map(|part| match part {
            ContentPart::Text(t) => Some(serde_json::json!({
                "type": "text",
                "text": t.text
            })),
            ContentPart::Image(img) => {
                // Convert to OpenAI image_url format
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
            // Skip ImageFile, ToolCall, ToolResult as they're handled separately
            // or not valid in user/system messages
            _ => None,
        })
        .collect();

    // If we filtered everything out, return empty string
    if parts.is_empty() {
        return serde_json::json!("");
    }

    // If single text part after filtering, use string format
    if parts.len() == 1
        && let Some(text) = parts[0].get("text")
    {
        return text.clone();
    }

    serde_json::json!(parts)
}

/// Convert tool calls to OpenAI-compatible format
pub fn convert_tool_calls_to_openai_format(tool_calls: &[ToolCall]) -> Vec<serde_json::Value> {
    tool_calls
        .iter()
        .map(|tc| {
            serde_json::json!({
                "id": tc.id,
                "type": "function",
                "function": {
                    "name": tc.name,
                    "arguments": serde_json::to_string(&tc.arguments).unwrap_or_else(|_| "{}".to_string())
                }
            })
        })
        .collect()
}

// ============================================================================
// Tests
// ============================================================================

#[cfg(test)]
mod tests {
    use super::*;
    use crate::message::Message;

    #[test]
    fn test_convert_user_message() {
        let msg = Message::user("Hello, world!");
        let converted = convert_message_to_openai_format(&msg);

        assert_eq!(converted["role"], "user");
        assert_eq!(converted["content"], "Hello, world!");
    }

    #[test]
    fn test_convert_system_message() {
        let msg = Message::system("You are a helpful assistant.");
        let converted = convert_message_to_openai_format(&msg);

        assert_eq!(converted["role"], "system");
        assert_eq!(converted["content"], "You are a helpful assistant.");
    }

    #[test]
    fn test_convert_assistant_message_maps_role() {
        // Our internal "agent" role should become "assistant"
        let msg = Message::assistant("Hi there!");
        let converted = convert_message_to_openai_format(&msg);

        assert_eq!(
            converted["role"], "assistant",
            "Internal 'agent' role should map to 'assistant'"
        );
        assert_eq!(converted["content"], "Hi there!");
    }

    #[test]
    fn test_convert_assistant_with_tool_calls() {
        let tool_call = ToolCall {
            id: "call_123".to_string(),
            name: "get_weather".to_string(),
            arguments: serde_json::json!({"location": "Tokyo"}),
        };
        let msg = Message::assistant_with_tools("Let me check.", vec![tool_call]);
        let converted = convert_message_to_openai_format(&msg);

        assert_eq!(converted["role"], "assistant");
        assert_eq!(converted["content"], "Let me check.");

        // Tool calls should be in OpenAI format
        let tool_calls = converted["tool_calls"].as_array().unwrap();
        assert_eq!(tool_calls.len(), 1);
        assert_eq!(tool_calls[0]["id"], "call_123");
        assert_eq!(tool_calls[0]["type"], "function");
        assert_eq!(tool_calls[0]["function"]["name"], "get_weather");
        // Arguments should be stringified JSON
        assert_eq!(
            tool_calls[0]["function"]["arguments"],
            r#"{"location":"Tokyo"}"#
        );
    }

    #[test]
    fn test_convert_tool_result_message() {
        // Our internal "tool_result" role should become "tool"
        let msg = Message::tool_result(
            "call_123",
            Some(serde_json::json!({"temperature": 72})),
            None,
        );
        let converted = convert_message_to_openai_format(&msg);

        assert_eq!(
            converted["role"], "tool",
            "Internal 'tool_result' role should map to 'tool'"
        );
        assert_eq!(converted["tool_call_id"], "call_123");
        assert_eq!(converted["content"], r#"{"temperature":72}"#);
    }

    #[test]
    fn test_convert_tool_result_error() {
        let msg = Message::tool_result("call_456", None, Some("API timeout".to_string()));
        let converted = convert_message_to_openai_format(&msg);

        assert_eq!(converted["role"], "tool");
        assert_eq!(converted["tool_call_id"], "call_456");
        assert_eq!(converted["content"], "Error: API timeout");
    }

    #[test]
    fn test_convert_messages_preserves_order() {
        let messages = vec![
            Message::system("System prompt"),
            Message::user("Hello"),
            Message::assistant("Hi!"),
        ];
        let converted = convert_messages_to_openai_format(&messages);

        assert_eq!(converted.len(), 3);
        assert_eq!(converted[0]["role"], "system");
        assert_eq!(converted[1]["role"], "user");
        assert_eq!(converted[2]["role"], "assistant");
    }

    #[test]
    fn test_convert_full_conversation_with_tools() {
        // Simulate a full conversation: user → assistant (with tool call) → tool result → assistant
        let tool_call = ToolCall {
            id: "call_abc".to_string(),
            name: "search".to_string(),
            arguments: serde_json::json!({"query": "rust"}),
        };

        let messages = vec![
            Message::user("Search for rust"),
            Message::assistant_with_tools("", vec![tool_call]),
            Message::tool_result(
                "call_abc",
                Some(serde_json::json!({"results": ["rust-lang.org"]})),
                None,
            ),
            Message::assistant("Here are the search results."),
        ];
        let converted = convert_messages_to_openai_format(&messages);

        assert_eq!(converted.len(), 4);
        assert_eq!(converted[0]["role"], "user");
        assert_eq!(
            converted[1]["role"], "assistant",
            "agent → assistant mapping"
        );
        assert!(
            converted[1]["tool_calls"].is_array(),
            "should have tool_calls"
        );
        assert_eq!(converted[2]["role"], "tool", "tool_result → tool mapping");
        assert_eq!(converted[2]["tool_call_id"], "call_abc");
        assert_eq!(converted[3]["role"], "assistant");
    }

    #[test]
    fn test_convert_tool_calls() {
        let tool_calls = vec![
            ToolCall {
                id: "call_1".to_string(),
                name: "search".to_string(),
                arguments: serde_json::json!({"query": "test"}),
            },
            ToolCall {
                id: "call_2".to_string(),
                name: "fetch".to_string(),
                arguments: serde_json::json!({"url": "http://example.com"}),
            },
        ];

        let converted = convert_tool_calls_to_openai_format(&tool_calls);

        assert_eq!(converted.len(), 2);
        assert_eq!(converted[0]["id"], "call_1");
        assert_eq!(converted[0]["type"], "function");
        assert_eq!(converted[0]["function"]["name"], "search");
        assert_eq!(converted[1]["id"], "call_2");
        assert_eq!(converted[1]["function"]["name"], "fetch");
    }
}
