//! stdio transport integration test against the bundled echo fixture.
//!
//! Only built with the `stdio` feature, mirroring how runtime/CLI hosts opt in.

#![cfg(feature = "stdio")]

use everruns_core::{McpProtocolMode, McpServerAuthMode};
use everruns_mcp::{McpConnection, McpEndpoint, McpTransport, StdioTransport};
use serde_json::json;
use std::collections::HashMap;

fn fixture_connection() -> McpConnection {
    McpConnection {
        name: "echo".into(),
        endpoint: McpEndpoint::Stdio {
            command: env!("CARGO_BIN_EXE_mcp_stdio_echo").into(),
            args: vec![],
            env: HashMap::new(),
        },
        auth_mode: McpServerAuthMode::None,
        protocol_mode: McpProtocolMode::Auto,
        oauth_provider_id: None,
    }
}

#[tokio::test]
async fn lists_tools_over_stdio() {
    let transport = StdioTransport;
    let tools = transport
        .list_tools(&fixture_connection(), None)
        .await
        .unwrap();
    assert_eq!(tools.len(), 1);
    assert_eq!(tools[0].name, "echo");
}

#[tokio::test]
async fn calls_tool_over_stdio() {
    let transport = StdioTransport;
    let result = transport
        .call_tool(
            &fixture_connection(),
            "echo",
            json!({ "message": "round-trip" }),
            None,
        )
        .await
        .unwrap();
    assert!(!result.is_error);

    let everruns_core::McpContent::Text { text } = &result.content[0] else {
        panic!("expected text content");
    };
    assert_eq!(text, "round-trip");
}
