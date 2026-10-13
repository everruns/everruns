//! MCP `agent_run` / `session_send_message` with `files`: each file becomes an
//! upload of the caller's org on the message.

use crate::test_harness;

use axum::http::StatusCode;
use serde_json::{Value, json};
use test_harness::TestServer;

async fn tool_call(server: &TestServer, method: &str, params: Value) -> Value {
    server
        .post(
            "/mcp",
            json!({ "jsonrpc": "2.0", "id": 1, "method": method, "params": params }),
        )
        .await
        .assert_success()
        .json()
}

async fn call(server: &TestServer, tool: &str, arguments: Value) -> Value {
    tool_call(
        server,
        "tools/call",
        json!({ "name": tool, "arguments": arguments }),
    )
    .await
}

fn text(resp: &Value) -> String {
    resp["result"]["content"][0]["text"]
        .as_str()
        .unwrap_or("")
        .to_string()
}

fn is_error(resp: &Value) -> bool {
    resp["result"]["isError"].as_bool().unwrap_or(false)
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn message_tools_take_files() {
    let server = TestServer::new().await;
    let tools = tool_call(&server, "tools/list", json!({})).await;
    let tools = tools["result"]["tools"].as_array().unwrap();
    for name in ["agent_run", "session_send_message"] {
        let tool = tools.iter().find(|tool| tool["name"] == name).unwrap();
        assert_eq!(tool["inputSchema"]["properties"]["files"]["type"], "array");
    }

    let run = call(&server, "agent_run", json!({ "message": "Start" })).await;
    assert!(!is_error(&run), "agent_run failed: {}", text(&run));
    let run: Value = serde_json::from_str(&text(&run)).unwrap();
    let session_id = run["session_id"].as_str().unwrap().to_string();

    let sent = call(
        &server,
        "session_send_message",
        json!({
            "session_id": session_id,
            "message": "Read the notes",
            "files": [{ "name": "notes.txt", "media_type": "text/plain", "data": "aGVsbG8=" }]
        }),
    )
    .await;
    assert!(!is_error(&sent), "failed: {}", text(&sent));

    let messages = server
        .get(&format!("/v1/sessions/{session_id}/messages"))
        .await
        .assert_status(StatusCode::OK)
        .text();
    assert!(
        messages.contains(r#""filename":"notes.txt""#),
        "the file is not on the message: {messages}"
    );

    let bad = call(
        &server,
        "session_send_message",
        json!({ "session_id": session_id, "message": "x", "files": "notes.txt" }),
    )
    .await;
    assert!(is_error(&bad));
    assert!(text(&bad).contains("Invalid parameter files"));
}
