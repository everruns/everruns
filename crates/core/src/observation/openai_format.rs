// OpenAI-Compatible Message Format Conversion
//
// Convenience functions that wrap Message::to_openai_format() and related methods.
// The actual conversion logic lives on the Message and ContentPart types.
//
// These functions are useful for:
// - Batch operations on message slices
// - Converting standalone ToolCall arrays (not part of a message)

use crate::message::{ContentPart, Message};
use crate::tool_types::ToolCall;

/// Convert messages to OpenAI-compatible format
///
/// Convenience wrapper for batch conversion. Calls `Message::to_openai_format()`
/// on each message.
pub fn convert_messages_to_openai_format(messages: &[Message]) -> Vec<serde_json::Value> {
    messages.iter().map(|m| m.to_openai_format()).collect()
}

/// Convert a single message to OpenAI-compatible format
///
/// Convenience wrapper for `Message::to_openai_format()`.
pub fn convert_message_to_openai_format(msg: &Message) -> serde_json::Value {
    msg.to_openai_format()
}

/// Convert content parts to OpenAI-compatible format
///
/// Converts an array of ContentPart to OpenAI content format.
/// Single text content becomes a string; multiple parts become an array.
pub fn convert_content_to_openai_format(content: &[ContentPart]) -> serde_json::Value {
    // Single text content → string
    if content.len() == 1
        && let ContentPart::Text(t) = &content[0]
    {
        return serde_json::json!(t.text);
    }

    // Convert each content part
    let parts: Vec<serde_json::Value> = content
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

/// Convert tool calls to OpenAI-compatible format
///
/// Used for standalone ToolCall arrays (e.g., from LlmGeneration events).
/// Tool calls embedded in Messages are handled by Message::to_openai_format().
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
