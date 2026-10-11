#![cfg_attr(test, allow(clippy::unwrap_used, clippy::expect_used))]
//! stdio transport integration test against the bundled echo fixture.
//!
//! Only built with the `stdio` feature, mirroring how runtime/CLI hosts opt in.

#![cfg(feature = "mcp-stdio")]

use everruns_core::mcp::{McpConnection, McpEndpoint, McpTransport, StdioTransport};
use everruns_core::{McpProtocolMode, McpServerAuthMode};
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
        elicitation_policy: Default::default(),
        oauth_provider_id: None,
        pending_oauth_provider: None,
        connect_in_chat: Default::default(),
        secret_bindings: HashMap::new(),
        acted_as: None,
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

    let everruns_core::McpContent::Text { text, .. } = &result.content[0] else {
        panic!("expected text content");
    };
    assert_eq!(text, "round-trip");
}

fn fixture_client() -> everruns_core::mcp::McpClient {
    everruns_core::mcp::McpClient::new(
        std::sync::Arc::new(everruns_core::DisabledEgressService),
        std::sync::Arc::new(everruns_core::mcp::NoAuthProvider),
    )
}

#[tokio::test]
async fn lists_resources_page_by_page_over_stdio() {
    let client = fixture_client();
    let connection = fixture_connection();

    let first = client.list_resources(&connection, None).await.unwrap();
    assert_eq!(first.resources.len(), 1);
    assert_eq!(first.resources[0].uri, "file:///readme.md");
    assert_eq!(first.resources[0].title.as_deref(), Some("Read me"));
    assert_eq!(first.resources[0].size, Some(7));
    let cursor = first.next_cursor.expect("first page has a cursor");

    let second = client
        .list_resources(&connection, Some(&cursor))
        .await
        .unwrap();
    assert_eq!(second.resources[0].uri, "file:///logo.png");
    assert!(
        second.resources[0]
            .annotations
            .as_ref()
            .is_some_and(|a| a.is_user_only())
    );
    assert!(second.next_cursor.is_none());

    let templates = client
        .list_resource_templates(&connection, None)
        .await
        .unwrap();
    assert_eq!(
        templates.resource_templates[0].uri_template,
        "file:///{path}"
    );
}

#[tokio::test]
async fn reads_resource_over_stdio() {
    let client = fixture_client();
    let read = client
        .read_resource(&fixture_connection(), "file:///readme.md")
        .await
        .unwrap();
    assert_eq!(read.contents.len(), 1);
    assert_eq!(read.contents[0].text.as_deref(), Some("# Hello"));
    assert_eq!(read.contents[0].mime_type.as_deref(), Some("text/markdown"));
}

#[tokio::test]
async fn unknown_resource_is_a_typed_not_found_error() {
    let client = fixture_client();
    let err = client
        .read_resource(&fixture_connection(), "file:///missing.txt")
        .await
        .unwrap_err();
    let rpc = err
        .downcast_ref::<everruns_core::mcp::McpRpcError>()
        .expect("JSON-RPC error");
    assert_eq!(rpc.code, -32002);
    assert_eq!(
        rpc.data.as_ref().and_then(|d| d["uri"].as_str()),
        Some("file:///missing.txt")
    );
}

fn fixture_executor() -> everruns_core::mcp::McpExecutor {
    everruns_core::mcp::McpExecutor::new(
        std::sync::Arc::new(fixture_client()),
        std::sync::Arc::new(
            everruns_core::mcp::StaticConnectionResolver::new().with(fixture_connection()),
        ),
    )
}

#[tokio::test]
async fn executor_lists_resources_with_templates_on_the_first_page() {
    use everruns_core::{McpResourceOutcome, McpToolInvoker};
    let executor = fixture_executor();
    let Some(McpResourceOutcome::Done(listing)) =
        executor.list_resources("echo", None).await.unwrap()
    else {
        panic!("expected a listing");
    };
    assert_eq!(listing.page.resources[0].uri, "file:///readme.md");
    assert_eq!(listing.templates.len(), 1);

    let Some(McpResourceOutcome::Done(next)) = executor
        .list_resources("echo", listing.page.next_cursor.as_deref())
        .await
        .unwrap()
    else {
        panic!("expected a listing");
    };
    assert_eq!(next.page.resources[0].uri, "file:///logo.png");
    assert!(
        next.templates.is_empty(),
        "templates only on the first page"
    );
}

#[tokio::test]
async fn executor_reads_resources_and_reports_unknown_ones() {
    use everruns_core::{McpResourceOutcome, McpToolInvoker};
    let executor = fixture_executor();
    let Some(McpResourceOutcome::Done(read)) = executor
        .read_resource("echo", "file:///readme.md")
        .await
        .unwrap()
    else {
        panic!("expected contents");
    };
    assert_eq!(read.contents[0].text.as_deref(), Some("# Hello"));

    let err = executor
        .read_resource("echo", "file:///missing.txt")
        .await
        .expect_err("unknown resource");
    assert!(err.to_string().contains("Resource not found"), "{err}");

    let err = executor
        .read_resource("nope", "file:///readme.md")
        .await
        .expect_err("unknown server");
    assert!(err.to_string().contains("not found"), "{err}");
}
