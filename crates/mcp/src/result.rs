//! MCP `tools/call` result → internal `ToolResult` mapping.
//!
//! Lifted verbatim from `worker/src/mcp_executor.rs` so behavior is unchanged
//! and the worker stops carrying its own copy (knowledge/integrations/runtime-mcp.md D1/D5).

use everruns_core::{McpContent, McpToolCallResult};
use everruns_provider::ToolResultImage;
use serde_json::{Value, json};

/// Convert an MCP `tools/call` result into a `(json_result, images)` pair.
///
/// Images are returned separately as [`ToolResultImage`] so the LLM receives
/// them as native image content blocks rather than embedded JSON.
pub fn map_tool_call_result(result: &McpToolCallResult) -> (Value, Vec<ToolResultImage>) {
    if result.is_error {
        let error_text = result
            .content
            .iter()
            .filter_map(|c| match c {
                McpContent::Text { text } => Some(text.as_str()),
                _ => None,
            })
            .collect::<Vec<_>>()
            .join("\n");
        return (json!({ "error": error_text }), vec![]);
    }

    let mut images = Vec::new();
    let mut text_parts = Vec::new();
    let mut resource_parts = Vec::new();

    for c in &result.content {
        match c {
            McpContent::Text { text } => text_parts.push(text.clone()),
            McpContent::Image { data, mime_type } => images.push(ToolResultImage {
                base64: data.clone(),
                media_type: mime_type.clone(),
            }),
            McpContent::Resource {
                uri,
                mime_type,
                text,
            } => resource_parts.push(json!({
                "type": "resource",
                "uri": uri,
                "mime_type": mime_type,
                "text": text
            })),
        }
    }

    let json_result = if resource_parts.is_empty() {
        if text_parts.len() == 1 {
            json!({ "result": text_parts[0] })
        } else if text_parts.is_empty() && !images.is_empty() {
            json!({ "result": format!("[{} image(s)]", images.len()) })
        } else {
            json!({ "result": text_parts.join("\n") })
        }
    } else {
        let mut content = Vec::new();
        for t in &text_parts {
            content.push(json!({ "type": "text", "text": t }));
        }
        content.extend(resource_parts);
        json!({ "content": content })
    };

    (json_result, images)
}

/// Extract the JSON-RPC payload from an MCP HTTP response, handling both plain
/// JSON and Server-Sent-Events (`event: message\ndata: {...}`) framing.
pub fn extract_json_from_response(response_text: &str) -> Option<&str> {
    if response_text.starts_with("event:") || response_text.contains("\ndata:") {
        response_text
            .lines()
            .find(|line| line.starts_with("data:"))
            .map(|line| line.trim_start_matches("data:").trim())
    } else {
        Some(response_text.trim())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn extract_plain_json() {
        let r = r#"{"result":{},"id":1,"jsonrpc":"2.0"}"#;
        assert_eq!(extract_json_from_response(r), Some(r));
    }

    #[test]
    fn extract_sse() {
        let r = "event: message\ndata: {\"result\":{},\"id\":1}\n";
        assert_eq!(
            extract_json_from_response(r),
            Some("{\"result\":{},\"id\":1}")
        );
    }

    #[test]
    fn maps_single_text_result() {
        let result = McpToolCallResult {
            content: vec![McpContent::Text {
                text: "hello".into(),
            }],
            is_error: false,
        };
        let (json, images) = map_tool_call_result(&result);
        assert_eq!(json, json!({ "result": "hello" }));
        assert!(images.is_empty());
    }

    #[test]
    fn maps_error_result() {
        let result = McpToolCallResult {
            content: vec![McpContent::Text {
                text: "boom".into(),
            }],
            is_error: true,
        };
        let (json, _) = map_tool_call_result(&result);
        assert_eq!(json, json!({ "error": "boom" }));
    }

    #[test]
    fn extracts_images_separately() {
        let result = McpToolCallResult {
            content: vec![
                McpContent::Text {
                    text: "chart".into(),
                },
                McpContent::Image {
                    data: "AAAA".into(),
                    mime_type: "image/png".into(),
                },
            ],
            is_error: false,
        };
        let (json, images) = map_tool_call_result(&result);
        assert_eq!(json, json!({ "result": "chart" }));
        assert_eq!(images.len(), 1);
        assert_eq!(images[0].media_type, "image/png");
    }
}
