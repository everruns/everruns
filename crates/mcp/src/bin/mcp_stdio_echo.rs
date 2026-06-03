//! Minimal MCP stdio server used only by the `stdio` integration test.
//!
//! Implements just enough of the protocol — `initialize`, the
//! `notifications/initialized` notification, `tools/list`, and `tools/call`
//! for a single `echo` tool — to exercise [`everruns_mcp::StdioTransport`]
//! end to end.

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

        let result = match method {
            "initialize" => Some(json!({
                "protocolVersion": "2024-11-05",
                "capabilities": { "tools": {} },
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
            _ => Some(json!({ "error": "unknown method" })),
        };

        // Notifications carry no id and get no response.
        if let (Some(id), Some(result)) = (id, result) {
            let response = json!({ "jsonrpc": "2.0", "id": id, "result": result });
            let mut bytes = serde_json::to_vec(&response).unwrap();
            bytes.push(b'\n');
            if stdout.write_all(&bytes).is_err() {
                break;
            }
            let _ = stdout.flush();
        }
    }
}
