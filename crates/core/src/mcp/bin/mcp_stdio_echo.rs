#![cfg_attr(test, allow(clippy::unwrap_used, clippy::expect_used))]
//! Minimal MCP stdio server used only by the `stdio` integration test.
//!
//! Implements just enough of the protocol — `initialize`, the
//! `notifications/initialized` notification, `tools/list`, and `tools/call`
//! for a single `echo` tool, plus two pages of resources, one resource
//! template and `resources/read` — to exercise
//! [`everruns_core::mcp::StdioTransport`] end to end.

use serde_json::{Value, json};
use std::io::{BufRead, Write};

fn main() {
    let stdin = std::io::stdin();
    let mut stdout = std::io::stdout();

    for line in stdin.lock().lines() {
        let Ok(line) = line else { break };
        let line = line.trim();
        if line.is_empty() {
            continue;
        }
        let Ok(request) = serde_json::from_str::<Value>(line) else {
            continue;
        };

        let method = request.get("method").and_then(Value::as_str).unwrap_or("");
        let id = request.get("id").cloned();

        let params = request.get("params").cloned().unwrap_or(Value::Null);
        let result = match method {
            "initialize" => Some(json!({
                "protocolVersion": "2024-11-05",
                "capabilities": { "tools": {}, "resources": {} },
                "serverInfo": { "name": "mcp_stdio_echo", "version": "0" }
            })),
            "notifications/initialized" => None,
            "tools/list" => Some(json!({
                "tools": [{
                    "name": "echo",
                    "description": "Echo the provided message back",
                    "inputSchema": {
                        "type": "object",
                        "properties": { "message": { "type": "string" } },
                        "required": ["message"]
                    }
                }]
            })),
            "tools/call" => {
                let message = request
                    .get("params")
                    .and_then(|p| p.get("arguments"))
                    .and_then(|a| a.get("message"))
                    .and_then(Value::as_str)
                    .unwrap_or("");
                Some(json!({
                    "content": [{ "type": "text", "text": message }],
                    "isError": false
                }))
            }
            "resources/list" => Some(match params.get("cursor").and_then(Value::as_str) {
                None => json!({
                    "resources": [{
                        "uri": "file:///readme.md", "name": "readme.md",
                        "title": "Read me", "mimeType": "text/markdown", "size": 7
                    }],
                    "nextCursor": "page-2"
                }),
                Some(_) => json!({
                    "resources": [{
                        "uri": "file:///logo.png", "name": "logo.png", "mimeType": "image/png",
                        "annotations": { "audience": ["user"], "priority": 0.5 }
                    }]
                }),
            }),
            "resources/templates/list" => Some(json!({
                "resourceTemplates": [{
                    "uriTemplate": "file:///{path}", "name": "Project files",
                    "mimeType": "application/octet-stream"
                }]
            })),
            "resources/read" => match params.get("uri").and_then(Value::as_str) {
                Some("file:///readme.md") => Some(json!({
                    "contents": [{
                        "uri": "file:///readme.md", "mimeType": "text/markdown", "text": "# Hello"
                    }]
                })),
                uri => {
                    reply(
                        &mut stdout,
                        json!({
                            "jsonrpc": "2.0", "id": id.clone(),
                            "error": {
                                "code": -32002, "message": "Resource not found",
                                "data": { "uri": uri }
                            }
                        }),
                    );
                    None
                }
            },
            _ => {
                reply(
                    &mut stdout,
                    json!({
                        "jsonrpc": "2.0", "id": id.clone(),
                        "error": { "code": -32601, "message": "Method not found" }
                    }),
                );
                None
            }
        };

        // Notifications carry no id and get no response.
        if let (Some(id), Some(result)) = (id, result) {
            if !reply(
                &mut stdout,
                json!({ "jsonrpc": "2.0", "id": id, "result": result }),
            ) {
                break;
            }
        }
    }
}

/// Write one JSON-RPC message as a line. Returns false once stdout is gone.
fn reply(stdout: &mut std::io::Stdout, response: Value) -> bool {
    let Ok(mut bytes) = serde_json::to_vec(&response) else {
        return true;
    };
    bytes.push(b'\n');
    if stdout.write_all(&bytes).is_err() {
        return false;
    }
    let _ = stdout.flush();
    true
}
