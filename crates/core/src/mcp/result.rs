//! MCP `tools/call` result → internal `ToolResult` mapping.
//!
//! Shared result parsing keeps hosted workers and in-process callers aligned.

use crate::mcp_resources::{base64_decoded_len, resource_part};
use crate::{McpContent, McpToolCallResult};
use everruns_contracts::ToolResultImage;
use serde_json::{Value, json};

/// Convert an MCP `tools/call` result into a `(json_result, images)` pair.
///
/// Images, inline or embedded as an `image/*` resource blob, are returned
/// separately as [`ToolResultImage`] so the LLM receives them as native image
/// content blocks rather than embedded JSON. Other binary data is described,
/// never inlined as base64. Blocks annotated for the user only are left out of
/// the model's result.
pub fn map_tool_call_result(result: &McpToolCallResult) -> (Value, Vec<ToolResultImage>) {
    let for_model = result
        .content
        .iter()
        .filter(|c| !c.annotations().is_some_and(|a| a.is_user_only()));

    if result.is_error {
        let error_text = for_model
            .filter_map(|c| match c {
                McpContent::Text { text, .. } => Some(text.as_str()),
                _ => None,
            })
            .collect::<Vec<_>>()
            .join("\n");
        return (json!({ "error": error_text }), vec![]);
    }

    let mut images = Vec::new();
    let mut parts = Vec::new();
    let mut only_text = true;

    for c in for_model {
        let part = match c {
            McpContent::Text { text, .. } => json!({ "type": "text", "text": text }),
            McpContent::Image {
                data, mime_type, ..
            } => {
                images.push(ToolResultImage {
                    base64: data.clone(),
                    media_type: mime_type.clone(),
                });
                continue;
            }
            McpContent::Audio {
                data, mime_type, ..
            } => json!({
                "type": "audio",
                "mime_type": mime_type,
                "size_bytes": base64_decoded_len(data),
            }),
            McpContent::Resource { resource, .. } => resource_part(resource, &mut images),
            McpContent::ResourceLink {
                uri,
                name,
                title,
                description,
                mime_type,
                size,
                ..
            } => {
                let mut link = json!({ "type": "resource_link", "uri": uri, "name": name });
                let fields = [
                    ("title", title.clone().map(Value::from)),
                    ("description", description.clone().map(Value::from)),
                    ("mime_type", mime_type.clone().map(Value::from)),
                    ("size", size.map(Value::from)),
                ];
                for (key, value) in fields {
                    if let Some(value) = value {
                        link[key] = value;
                    }
                }
                link
            }
            McpContent::Unknown => json!({ "type": "unsupported" }),
        };
        only_text &= part["type"] == "text";
        parts.push(part);
    }

    let json_result = if only_text {
        let texts: Vec<&str> = parts.iter().filter_map(|p| p["text"].as_str()).collect();
        if texts.is_empty() && !images.is_empty() {
            json!({ "result": format!("[{} image(s)]", images.len()) })
        } else {
            json!({ "result": texts.join("\n") })
        }
    } else {
        json!({ "content": parts })
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

    /// Spec-shaped (2025-11-25) content blocks must parse and survive mapping.
    #[test]
    fn parses_and_maps_spec_content_blocks() {
        let result: McpToolCallResult = serde_json::from_value(json!({
            "content": [
                { "type": "text", "text": "summary" },
                { "type": "image", "data": "AAAA", "mimeType": "image/png" },
                { "type": "audio", "data": "AAAAAAAA", "mimeType": "audio/wav" },
                { "type": "resource", "resource": {
                    "uri": "file:///notes.md", "mimeType": "text/markdown", "text": "# Notes"
                } },
                { "type": "resource", "resource": {
                    "uri": "file:///chart.png", "mimeType": "image/png", "blob": "BBBB"
                } },
                { "type": "resource", "resource": {
                    "uri": "file:///data.bin", "mimeType": "application/octet-stream", "blob": "AAAAAAAA"
                } },
                { "type": "resource_link", "uri": "file:///big.csv", "name": "big.csv",
                  "description": "Full export", "mimeType": "text/csv", "size": 1024 },
                { "type": "text", "text": "for the user only",
                  "annotations": { "audience": ["user"], "priority": 0.2 } },
                { "type": "hologram", "data": "?" }
            ]
        }))
        .expect("spec-shaped content parses");

        let (json, images) = map_tool_call_result(&result);
        let media: Vec<_> = images.iter().map(|i| i.media_type.as_str()).collect();
        assert_eq!(media, ["image/png", "image/png"]);
        assert_eq!(images[1].base64, "BBBB");
        assert_eq!(
            json,
            json!({ "content": [
                { "type": "text", "text": "summary" },
                { "type": "audio", "mime_type": "audio/wav", "size_bytes": 6 },
                { "type": "resource", "uri": "file:///notes.md",
                  "mime_type": "text/markdown", "text": "# Notes" },
                { "type": "resource", "uri": "file:///chart.png",
                  "mime_type": "image/png", "image": true },
                { "type": "resource", "uri": "file:///data.bin",
                  "mime_type": "application/octet-stream", "size_bytes": 6 },
                { "type": "resource_link", "uri": "file:///big.csv", "name": "big.csv",
                  "description": "Full export", "mime_type": "text/csv", "size": 1024 },
                { "type": "unsupported" }
            ] })
        );
    }

    #[test]
    fn maps_single_text_result() {
        let result = McpToolCallResult {
            content: vec![McpContent::text("hello")],
            is_error: false,
        };
        let (json, images) = map_tool_call_result(&result);
        assert_eq!(json, json!({ "result": "hello" }));
        assert!(images.is_empty());
    }

    #[test]
    fn maps_error_result() {
        let result = McpToolCallResult {
            content: vec![McpContent::text("boom")],
            is_error: true,
        };
        let (json, _) = map_tool_call_result(&result);
        assert_eq!(json, json!({ "error": "boom" }));
    }

    #[test]
    fn extracts_images_separately() {
        let result = McpToolCallResult {
            content: vec![
                McpContent::text("chart"),
                McpContent::Image {
                    data: "AAAA".into(),
                    mime_type: "image/png".into(),
                    annotations: None,
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
