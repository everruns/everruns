//! Integration tests for the MCP endpoint (POST /mcp)
//!
//! Tests the Everruns MCP server: JSON-RPC protocol, Tier 1 tools
//! (agent_run, session_send_message, session_get_status), and Tier 2 tools
//! (discover, execute with catalog operations).
//!
//! Run with: cargo test -p everruns-server --test mcp_endpoint_test -- --test-threads=1
//!
//! Requirements:
//! - PostgreSQL running with DATABASE_URL set
//! - Migrations applied

mod test_harness;

use axum::http::{Method, StatusCode};
use everruns_core::DEFAULT_ORG_PUBLIC_ID;
use everruns_core::capability_types::VirtualFileTree;
use everruns_core::typed_id::SessionId;
use serde_json::{Value, json};
use test_harness::{TestServer, extract_cookie};

// ============================================================================
// Helpers
// ============================================================================

static UNIQUE_COUNTER: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);
const MCP_PROTOCOL_VERSION_FALLBACK: &str = "2025-03-26";
const MCP_PROTOCOL_VERSION_LATEST: &str = "2025-06-18";

/// Make a JSON-RPC 2.0 call to POST /mcp via the in-process TestServer.
async fn mcp_call(server: &TestServer, method: &str, params: Value) -> Value {
    server
        .post(
            "/mcp",
            json!({
                "jsonrpc": "2.0",
                "id": 1,
                "method": method,
                "params": params
            }),
        )
        .await
        .assert_success()
        .json()
}

async fn mcp_call_with_headers(
    server: &TestServer,
    method: &str,
    params: Value,
    extra_headers: Vec<(&str, &str)>,
) -> Value {
    let mut headers = vec![("content-type", "application/json")];
    headers.extend(extra_headers);

    server
        .request_raw(
            Method::POST,
            "/mcp",
            headers,
            serde_json::to_vec(&json!({
                "jsonrpc": "2.0",
                "id": 1,
                "method": method,
                "params": params
            }))
            .expect("Failed to encode MCP request"),
        )
        .await
        .assert_success()
        .json()
}

async fn mcp_request_raw(
    server: &TestServer,
    method: &str,
    params: Value,
    extra_headers: Vec<(&str, &str)>,
) -> test_harness::TestResponse {
    let mut headers = vec![("content-type", "application/json")];
    headers.extend(extra_headers);

    server
        .request_raw(
            Method::POST,
            "/mcp",
            headers,
            serde_json::to_vec(&json!({
                "jsonrpc": "2.0",
                "id": 1,
                "method": method,
                "params": params
            }))
            .expect("Failed to encode MCP request"),
        )
        .await
}

/// Call tools/call with a given tool name and arguments.
async fn mcp_tool_call(server: &TestServer, tool: &str, arguments: Value) -> Value {
    mcp_call(
        server,
        "tools/call",
        json!({ "name": tool, "arguments": arguments }),
    )
    .await
}

async fn mcp_tool_call_with_headers(
    server: &TestServer,
    tool: &str,
    arguments: Value,
    extra_headers: Vec<(&str, &str)>,
) -> Value {
    mcp_call_with_headers(
        server,
        "tools/call",
        json!({ "name": tool, "arguments": arguments }),
        extra_headers,
    )
    .await
}

/// Extract the text content from a tools/call result.
fn tool_text(resp: &Value) -> String {
    resp["result"]["content"][0]["text"]
        .as_str()
        .unwrap_or("")
        .to_string()
}

/// Check if a tools/call result is an error.
fn tool_is_error(resp: &Value) -> bool {
    resp["result"]["isError"].as_bool().unwrap_or(false)
}

/// Parse the text content of a tool result as JSON.
fn tool_json(resp: &Value) -> Value {
    let text = tool_text(resp);
    serde_json::from_str(&text).unwrap_or_else(|e| panic!("Bad JSON in tool result: {e}\n{text}"))
}

/// Make a JSON-RPC call via HTTP to a serving TestServer.
async fn mcp_call_http(base_url: &str, method: &str, params: Value) -> Value {
    let client = reqwest::Client::new();
    let resp = client
        .post(format!("{base_url}/mcp"))
        .json(&json!({
            "jsonrpc": "2.0",
            "id": 1,
            "method": method,
            "params": params
        }))
        .send()
        .await
        .expect("HTTP request failed");
    assert!(resp.status().is_success());
    resp.json().await.expect("Failed to parse JSON response")
}

async fn mcp_tool_call_http(base_url: &str, tool: &str, arguments: Value) -> Value {
    mcp_call_http(
        base_url,
        "tools/call",
        json!({ "name": tool, "arguments": arguments }),
    )
    .await
}

async fn assert_notification_accepted_no_body(server: &TestServer, request: Value, case: &str) {
    let resp = server.post("/mcp", request).await;
    assert_eq!(
        resp.status(),
        axum::http::StatusCode::ACCEPTED,
        "{case}: expected 202 Accepted"
    );
    let body = resp.text();
    assert!(
        body.is_empty(),
        "{case}: notification response body must be empty, got: {body}"
    );
}

async fn execute_http_command(base_url: &str, commands: impl Into<String>) -> Value {
    mcp_tool_call_http(base_url, "execute", json!({ "commands": commands.into() })).await
}

fn unique_suffix() -> String {
    let millis = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap()
        .as_millis();
    let counter = UNIQUE_COUNTER.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
    format!("{millis}-{counter}")
}

async fn create_agent_via_execute(
    base_url: &str,
    slug: &str,
    display_name: &str,
) -> (String, String) {
    let unique_name = format!("{slug}-{}", unique_suffix());
    let resp = execute_http_command(
        base_url,
        format!(
            "create_agent --name '{unique_name}' --display_name '{display_name}' --system_prompt 'Test prompt'"
        ),
    )
    .await;
    assert!(
        !tool_is_error(&resp),
        "create_agent failed: {}",
        tool_text(&resp)
    );
    let payload = tool_json(&resp);
    (
        payload["id"].as_str().unwrap().to_string(),
        payload["name"].as_str().unwrap().to_string(),
    )
}

async fn create_session_via_execute(base_url: &str, title_prefix: &str) -> (String, String) {
    let title = format!("{title_prefix} {}", unique_suffix());
    let resp = execute_http_command(base_url, format!("create_session --title '{title}'")).await;
    assert!(
        !tool_is_error(&resp),
        "create_session failed: {}",
        tool_text(&resp)
    );
    let payload = tool_json(&resp);
    (
        payload["id"].as_str().unwrap().to_string(),
        payload["title"].as_str().unwrap().to_string(),
    )
}

// ============================================================================
// Protocol tests
// ============================================================================

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn test_mcp_initialize() {
    let server = TestServer::in_memory().await;
    let resp = mcp_call(
        &server,
        "initialize",
        json!({ "protocolVersion": MCP_PROTOCOL_VERSION_LATEST }),
    )
    .await;

    assert_eq!(resp["jsonrpc"], "2.0");
    assert_eq!(resp["id"], 1);
    assert_eq!(
        resp["result"]["protocolVersion"],
        MCP_PROTOCOL_VERSION_LATEST
    );
    assert_eq!(resp["result"]["serverInfo"]["name"], "everruns");
    assert!(resp["result"]["serverInfo"]["version"].is_string());
    assert!(resp["result"]["capabilities"]["tools"].is_object());
    assert_eq!(
        resp["result"]["capabilities"]["tools"]["listChanged"],
        false
    );
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn test_mcp_initialize_defaults_to_fallback_protocol() {
    let server = TestServer::in_memory().await;
    let resp = mcp_call(&server, "initialize", json!({})).await;

    assert_eq!(
        resp["result"]["protocolVersion"],
        MCP_PROTOCOL_VERSION_FALLBACK
    );
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn test_mcp_ping() {
    let server = TestServer::in_memory().await;
    let resp = mcp_call(&server, "ping", json!({})).await;

    assert_eq!(resp["jsonrpc"], "2.0");
    assert_eq!(resp["result"], json!({}));
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn test_mcp_tools_list() {
    let server = TestServer::in_memory().await;
    let resp = mcp_call_with_headers(
        &server,
        "tools/list",
        json!({}),
        vec![("MCP-Protocol-Version", MCP_PROTOCOL_VERSION_LATEST)],
    )
    .await;

    let tools = resp["result"]["tools"]
        .as_array()
        .expect("Expected tools array");
    assert_eq!(tools.len(), 9, "Expected 9 MCP tools");

    let names: Vec<&str> = tools.iter().map(|t| t["name"].as_str().unwrap()).collect();
    assert!(names.contains(&"me"), "Missing me");
    assert!(
        names.contains(&"list_organizations"),
        "Missing list_organizations"
    );
    assert!(
        !names.contains(&"switch_organization"),
        "switch_organization should not be exposed"
    );
    let list_organizations = tools
        .iter()
        .find(|tool| tool["name"] == "list_organizations")
        .unwrap();
    assert!(
        list_organizations["description"]
            .as_str()
            .unwrap()
            .contains("MCP has no current-organization switch")
    );
    assert!(names.contains(&"agent_run"), "Missing agent_run");
    assert!(
        names.contains(&"session_send_message"),
        "Missing session_send_message"
    );
    assert!(
        names.contains(&"session_get_status"),
        "Missing session_get_status"
    );
    assert!(names.contains(&"discover"), "Missing discover");
    assert!(names.contains(&"query"), "Missing query");
    assert!(names.contains(&"execute"), "Missing execute");
    // Card tools (specs/mcp-cards.md) only exposed under 2025-06-18.
    assert!(names.contains(&"agent_get_card"), "Missing agent_get_card");

    // Verify each tool has inputSchema
    for tool in tools {
        assert!(
            tool["inputSchema"].is_object(),
            "Tool {} missing inputSchema",
            tool["name"]
        );
        assert_eq!(tool["inputSchema"]["type"], "object");
    }

    let me = tools.iter().find(|tool| tool["name"] == "me").unwrap();
    assert_eq!(me["title"], "Current User");
    assert_eq!(me["annotations"]["readOnlyHint"], true);
    assert_eq!(me["outputSchema"]["type"], "object");
    assert!(me.as_object().unwrap().get("_meta").is_none());

    let agent_run = tools
        .iter()
        .find(|tool| tool["name"] == "agent_run")
        .unwrap();
    assert_eq!(agent_run["title"], "Run Agent");
    assert_eq!(agent_run["outputSchema"]["type"], "object");
    assert_eq!(agent_run["annotations"]["openWorldHint"], true);
    assert!(agent_run.as_object().unwrap().get("_meta").is_none());

    let discover = tools
        .iter()
        .find(|tool| tool["name"] == "discover")
        .unwrap();
    assert_eq!(discover["title"], "Discover Operations");
    assert_eq!(
        discover["inputSchema"]["properties"]["organization_id"]["pattern"],
        "^org_[0-9a-f]{32}$"
    );
    assert!(
        discover["inputSchema"]["properties"]["organization_id"]["description"]
            .as_str()
            .unwrap()
            .contains("pass this on each org-scoped call")
    );
    assert_eq!(discover["outputSchema"]["type"], "object");
    assert!(discover.as_object().unwrap().get("_meta").is_none());

    let query = tools.iter().find(|tool| tool["name"] == "query").unwrap();
    assert_eq!(query["title"], "Query Commands");
    assert_eq!(query["annotations"]["readOnlyHint"], true);
    assert_eq!(query["annotations"]["destructiveHint"], false);
    assert_eq!(query["annotations"]["idempotentHint"], true);
    assert_eq!(query["annotations"]["openWorldHint"], false);
    assert!(query.as_object().unwrap().get("outputSchema").is_none());
    assert!(query.as_object().unwrap().get("_meta").is_none());

    let execute = tools.iter().find(|tool| tool["name"] == "execute").unwrap();
    assert_eq!(execute["title"], "Execute Commands");
    assert_eq!(execute["annotations"]["readOnlyHint"], false);
    assert_eq!(execute["annotations"]["destructiveHint"], true);
    assert_eq!(execute["annotations"]["idempotentHint"], false);
    assert_eq!(execute["annotations"]["openWorldHint"], true);
    assert!(execute.as_object().unwrap().get("outputSchema").is_none());
    assert!(execute.as_object().unwrap().get("_meta").is_none());
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn test_mcp_tools_list_fallback_omits_2025_06_fields() {
    let server = TestServer::in_memory().await;
    let resp = mcp_call_with_headers(
        &server,
        "tools/list",
        json!({}),
        vec![("MCP-Protocol-Version", MCP_PROTOCOL_VERSION_FALLBACK)],
    )
    .await;

    let tools = resp["result"]["tools"]
        .as_array()
        .expect("Expected tools array");
    let me = tools.iter().find(|tool| tool["name"] == "me").unwrap();
    assert!(me.as_object().unwrap().get("title").is_none());
    assert!(me.as_object().unwrap().get("outputSchema").is_none());
    assert!(me.as_object().unwrap().get("_meta").is_none());

    let agent_run = tools
        .iter()
        .find(|tool| tool["name"] == "agent_run")
        .unwrap();
    assert!(agent_run.as_object().unwrap().get("title").is_none());
    assert!(agent_run.as_object().unwrap().get("outputSchema").is_none());
    assert_eq!(agent_run["annotations"]["openWorldHint"], true);

    // Card tools require 2025-06-18 features and must not be advertised
    // under the fallback protocol. See specs/mcp-cards.md.
    let card_names: Vec<&str> = tools
        .iter()
        .filter_map(|t| t["name"].as_str())
        .filter(|n| n.ends_with("_get_card"))
        .collect();
    assert!(
        card_names.is_empty(),
        "card tools must not appear under {MCP_PROTOCOL_VERSION_FALLBACK}, got {card_names:?}"
    );
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn test_mcp_rejects_unsupported_protocol_header() {
    let server = TestServer::in_memory().await;
    let resp = mcp_request_raw(
        &server,
        "ping",
        json!({}),
        vec![("MCP-Protocol-Version", "2024-11-05")],
    )
    .await;

    assert_eq!(resp.status(), StatusCode::BAD_REQUEST);
    let body: Value = resp.json();
    assert_eq!(body["error"]["code"], -32600);
    assert!(
        body["error"]["message"]
            .as_str()
            .unwrap()
            .contains("Unsupported MCP-Protocol-Version")
    );
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn test_mcp_tool_call_returns_structured_content() {
    let server = TestServer::in_memory().await;
    let resp = mcp_tool_call_with_headers(
        &server,
        "me",
        json!({}),
        vec![("MCP-Protocol-Version", MCP_PROTOCOL_VERSION_LATEST)],
    )
    .await;

    assert!(
        !tool_is_error(&resp),
        "tool returned error: {}",
        tool_text(&resp)
    );
    let structured = resp["result"]["structuredContent"].clone();
    assert!(
        structured.is_object(),
        "structuredContent should be present"
    );
    assert!(structured["user"]["id"].is_string());
    assert!(structured["current_organization"]["id"].is_string());
    assert_eq!(tool_json(&resp), structured);
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn test_mcp_tool_call_fallback_omits_structured_content() {
    let server = TestServer::in_memory().await;
    let resp = mcp_tool_call_with_headers(
        &server,
        "me",
        json!({}),
        vec![("MCP-Protocol-Version", MCP_PROTOCOL_VERSION_FALLBACK)],
    )
    .await;

    assert!(
        !tool_is_error(&resp),
        "tool returned error: {}",
        tool_text(&resp)
    );
    assert!(resp["result"].get("structuredContent").is_none());
    assert!(tool_json(&resp)["user"]["id"].is_string());
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn test_mcp_invalid_jsonrpc_version() {
    let server = TestServer::in_memory().await;
    let resp: Value = server
        .post(
            "/mcp",
            json!({
                "jsonrpc": "1.0",
                "id": 1,
                "method": "ping"
            }),
        )
        .await
        .assert_success()
        .json();

    assert!(resp["error"].is_object());
    assert_eq!(resp["error"]["code"], -32600);
    assert!(resp["error"]["message"].as_str().unwrap().contains("2.0"));
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn test_mcp_unknown_method() {
    let server = TestServer::in_memory().await;
    let resp = mcp_call(&server, "nonexistent/method", json!({})).await;

    assert!(resp["error"].is_object());
    assert_eq!(resp["error"]["code"], -32601);
}

// Contract matrix for recent MCP notification/no-id regressions:
// - EVE-322: notifications must always return 202 with an empty body
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn test_mcp_notification_contract_matrix() {
    let server = TestServer::in_memory().await;
    let cases = vec![
        (
            "initialized lifecycle notification",
            json!({
                "jsonrpc": "2.0",
                "method": "notifications/initialized"
            }),
        ),
        (
            "known notification with params",
            json!({
                "jsonrpc": "2.0",
                "method": "notifications/cancelled",
                "params": { "requestId": "abc" }
            }),
        ),
        (
            "unknown notification method",
            json!({
                "jsonrpc": "2.0",
                "method": "notifications/not-real",
                "params": { "reason": "ignored" }
            }),
        ),
        (
            "invalid jsonrpc version without id",
            json!({
                "jsonrpc": "1.0",
                "method": "notifications/initialized"
            }),
        ),
        (
            "malformed params without id",
            json!({
                "jsonrpc": "2.0",
                "method": "notifications/cancelled",
                "params": "not-an-object"
            }),
        ),
    ];

    for (case, request) in cases {
        assert_notification_accepted_no_body(&server, request, case).await;
    }
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn test_mcp_tools_call_unknown_tool() {
    let server = TestServer::new().await;
    let resp = mcp_tool_call(&server, "nonexistent_tool", json!({})).await;

    assert!(tool_is_error(&resp), "Expected isError for unknown tool");
    assert!(tool_text(&resp).contains("Unknown tool"));
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn test_mcp_tools_call_missing_name() {
    let server = TestServer::new().await;
    let resp = mcp_call(&server, "tools/call", json!({})).await;

    assert!(resp["error"].is_object());
    assert_eq!(resp["error"]["code"], -32602);
}

// ============================================================================
// Tier 1: agent_run
// ============================================================================

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn test_mcp_agent_run() {
    let server = TestServer::new().await;

    // Create an agent first via the REST API
    let agent: Value = server
        .post(
            "/v1/agents",
            json!({
                "name": "mcp-test-agent",
                "display_name": "MCP Test Agent",
                "system_prompt": "You are a test agent."
            }),
        )
        .await
        .assert_success()
        .json();
    let agent_id = agent["id"].as_str().unwrap();

    // Run via MCP
    let resp = mcp_tool_call(
        &server,
        "agent_run",
        json!({
            "agent_id": agent_id,
            "message": "Hello from MCP test",
            "title": "MCP Integration Test"
        }),
    )
    .await;

    assert!(
        !tool_is_error(&resp),
        "agent_run failed: {}",
        tool_text(&resp)
    );

    let result = tool_json(&resp);
    assert!(
        result["session_id"]
            .as_str()
            .unwrap()
            .starts_with("session_")
    );
    assert!(
        result["message_id"]
            .as_str()
            .unwrap()
            .starts_with("message_")
    );
    assert!(result["hint"].is_string());
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn test_mcp_agent_run_without_agent() {
    let server = TestServer::new().await;

    // agent_run without agent_id should still work (uses base harness)
    let resp = mcp_tool_call(
        &server,
        "agent_run",
        json!({ "message": "Hello without agent" }),
    )
    .await;

    assert!(
        !tool_is_error(&resp),
        "agent_run failed: {}",
        tool_text(&resp)
    );
    let result = tool_json(&resp);
    assert!(
        result["session_id"]
            .as_str()
            .unwrap()
            .starts_with("session_")
    );
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn test_mcp_agent_run_missing_message() {
    let server = TestServer::new().await;
    let resp = mcp_tool_call(&server, "agent_run", json!({})).await;

    assert!(tool_is_error(&resp), "Expected error for missing message");
    assert!(tool_text(&resp).contains("message"));
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn test_mcp_agent_run_invalid_agent_id() {
    let server = TestServer::new().await;
    let resp = mcp_tool_call(
        &server,
        "agent_run",
        json!({
            "agent_id": "not-a-valid-id",
            "message": "Hello"
        }),
    )
    .await;

    assert!(tool_is_error(&resp), "Expected error for invalid agent_id");
}

// ============================================================================
// Tier 1: session_send_message
// ============================================================================

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn test_mcp_session_send_message() {
    let server = TestServer::new().await;

    // Create a session via agent_run
    let run_resp = mcp_tool_call(
        &server,
        "agent_run",
        json!({ "message": "Initial message" }),
    )
    .await;
    assert!(
        !tool_is_error(&run_resp),
        "agent_run failed: {}",
        tool_text(&run_resp)
    );
    let session_id = tool_json(&run_resp)["session_id"]
        .as_str()
        .unwrap()
        .to_string();

    // Send a follow-up message
    let resp = mcp_tool_call(
        &server,
        "session_send_message",
        json!({
            "session_id": session_id,
            "message": "Follow-up message"
        }),
    )
    .await;

    assert!(
        !tool_is_error(&resp),
        "session_send_message failed: {}",
        tool_text(&resp)
    );
    let result = tool_json(&resp);
    assert!(
        result["message_id"]
            .as_str()
            .unwrap()
            .starts_with("message_")
    );
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn test_mcp_session_send_message_missing_params() {
    let server = TestServer::new().await;

    // Missing session_id
    let resp = mcp_tool_call(
        &server,
        "session_send_message",
        json!({ "message": "Hello" }),
    )
    .await;
    assert!(tool_is_error(&resp));

    // Missing message
    let resp = mcp_tool_call(
        &server,
        "session_send_message",
        json!({ "session_id": "session_00000000000000000000000000000000" }),
    )
    .await;
    assert!(tool_is_error(&resp));
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn test_mcp_session_send_message_nonexistent_session() {
    let server = TestServer::new().await;
    let resp = mcp_tool_call(
        &server,
        "session_send_message",
        json!({
            "session_id": "session_00000000000000000000000000000000",
            "message": "Hello"
        }),
    )
    .await;

    assert!(
        tool_is_error(&resp),
        "Expected error for nonexistent session"
    );
}

// ============================================================================
// Tier 1: session_get_status
// ============================================================================

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn test_mcp_session_get_status() {
    let server = TestServer::new().await;

    // Create a session
    let run_resp = mcp_tool_call(&server, "agent_run", json!({ "message": "Status test" })).await;
    assert!(
        !tool_is_error(&run_resp),
        "agent_run failed: {}",
        tool_text(&run_resp)
    );
    let session_id = tool_json(&run_resp)["session_id"]
        .as_str()
        .unwrap()
        .to_string();

    // Get status
    let resp = mcp_tool_call(
        &server,
        "session_get_status",
        json!({ "session_id": session_id }),
    )
    .await;

    assert!(
        !tool_is_error(&resp),
        "session_get_status failed: {}",
        tool_text(&resp)
    );
    let result = tool_json(&resp);
    assert_eq!(result["session_id"], session_id);
    assert!(result["status"].is_string());
    assert!(result["events"].is_array());
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn test_mcp_session_get_status_with_event_filter() {
    let server = TestServer::new().await;

    // Create a session
    let run_resp = mcp_tool_call(&server, "agent_run", json!({ "message": "Filter test" })).await;
    assert!(
        !tool_is_error(&run_resp),
        "agent_run failed: {}",
        tool_text(&run_resp)
    );
    let session_id = tool_json(&run_resp)["session_id"]
        .as_str()
        .unwrap()
        .to_string();

    // Get status with event type filter
    let resp = mcp_tool_call(
        &server,
        "session_get_status",
        json!({
            "session_id": session_id,
            "event_types": ["session.idled"]
        }),
    )
    .await;

    assert!(
        !tool_is_error(&resp),
        "session_get_status failed: {}",
        tool_text(&resp)
    );
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn test_mcp_session_get_status_missing_session_id() {
    let server = TestServer::new().await;
    let resp = mcp_tool_call(&server, "session_get_status", json!({})).await;
    assert!(tool_is_error(&resp));
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn test_mcp_session_get_status_nonexistent() {
    let server = TestServer::new().await;
    let resp = mcp_tool_call(
        &server,
        "session_get_status",
        json!({ "session_id": "session_00000000000000000000000000000000" }),
    )
    .await;
    assert!(
        tool_is_error(&resp),
        "Expected error for nonexistent session"
    );
}

// ============================================================================
// Tier 2: discover
// ============================================================================

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn test_mcp_discover_agents() {
    let server = TestServer::in_memory().await;
    let resp = mcp_tool_call(&server, "discover", json!({ "query": "agents" })).await;

    assert!(
        !tool_is_error(&resp),
        "discover failed: {}",
        tool_text(&resp)
    );
    let payload = tool_json(&resp);
    let text = payload.to_string();
    assert!(
        text.contains("list_agents"),
        "discover should find list_agents, got: {text}"
    );
    let list_agents = payload["operations"]
        .as_array()
        .unwrap()
        .iter()
        .find(|op| op["name"] == "list_agents")
        .expect("list_agents operation");
    assert_eq!(list_agents["output_shape"], "paginated");
    assert_eq!(
        list_agents["output_schema"]["properties"]["data"]["type"],
        "array"
    );
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn test_mcp_discover_sessions() {
    let server = TestServer::in_memory().await;
    let resp = mcp_tool_call(&server, "discover", json!({ "query": "sessions" })).await;

    assert!(
        !tool_is_error(&resp),
        "discover failed: {}",
        tool_text(&resp)
    );
    let payload = tool_json(&resp);
    let text = payload.to_string();
    // "sessions" matches list_sessions, create_session, get_session, etc.
    assert!(
        text.contains("list_sessions"),
        "discover should find list_sessions, got: {text}"
    );
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn test_mcp_discover_harnesses() {
    let server = TestServer::in_memory().await;
    let resp = mcp_tool_call(&server, "discover", json!({ "query": "harnesses" })).await;

    assert!(
        !tool_is_error(&resp),
        "discover failed: {}",
        tool_text(&resp)
    );
    let payload = tool_json(&resp);
    let text = payload.to_string();
    assert!(
        text.contains("list_harnesses"),
        "discover should find list_harnesses"
    );
    let list_harnesses = payload["operations"]
        .as_array()
        .unwrap()
        .iter()
        .find(|op| op["name"] == "list_harnesses")
        .expect("list_harnesses operation");
    assert_eq!(list_harnesses["output_shape"], "array");
    assert_eq!(list_harnesses["output_schema"]["type"], "array");
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn test_mcp_discover_mcp_servers() {
    let server = TestServer::in_memory().await;
    let resp = mcp_tool_call(&server, "discover", json!({ "query": "mcp" })).await;

    assert!(
        !tool_is_error(&resp),
        "discover failed: {}",
        tool_text(&resp)
    );
    let payload = tool_json(&resp);
    let text = payload.to_string();
    assert!(
        text.contains("mcp"),
        "discover should find MCP-related operations"
    );
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn test_mcp_discover_missing_query() {
    let server = TestServer::in_memory().await;
    let resp = mcp_tool_call(&server, "discover", json!({})).await;
    assert!(tool_is_error(&resp), "Expected error for missing query");
    assert!(
        tool_text(&resp).contains("query"),
        "Expected query guidance, got: {}",
        tool_text(&resp)
    );
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn test_mcp_discover_empty_query() {
    let server = TestServer::in_memory().await;
    let resp = mcp_tool_call(&server, "discover", json!({ "query": "" })).await;
    assert!(tool_is_error(&resp), "Expected error for empty query");
    assert!(
        tool_text(&resp).contains("all: true"),
        "Expected all:true guidance, got: {}",
        tool_text(&resp)
    );
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn test_mcp_discover_all() {
    let server = TestServer::in_memory().await;
    let resp = mcp_tool_call(&server, "discover", json!({ "all": true })).await;

    assert!(
        !tool_is_error(&resp),
        "discover --all failed: {}",
        tool_text(&resp)
    );
    let payload = tool_json(&resp);
    let categories = payload["categories"].as_array().expect("categories array");
    assert!(
        categories.iter().any(|cat| cat["category"] == "agents"),
        "Should list agents category"
    );
    assert!(
        categories.iter().any(|cat| cat["category"] == "sessions"),
        "Should list sessions category"
    );
    assert!(
        payload.to_string().contains("create_agent"),
        "Should include create_agent"
    );
    assert_eq!(payload["include_schemas"], false);
    let first_operation = payload["categories"][0]["operations"][0]
        .as_object()
        .expect("operation object");
    assert!(
        !first_operation.contains_key("input_schema"),
        "discover --all should omit input schemas by default"
    );
    assert!(
        !first_operation.contains_key("output_schema"),
        "discover --all should omit output schemas by default"
    );
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn test_mcp_discover_fuzzy_search() {
    let server = TestServer::in_memory().await;
    // "create agent" should match "create_agent" via token matching
    let resp = mcp_tool_call(&server, "discover", json!({ "query": "create agent" })).await;

    assert!(
        !tool_is_error(&resp),
        "discover fuzzy search failed: {}",
        tool_text(&resp)
    );
    let payload = tool_json(&resp);
    let text = payload.to_string();
    assert!(
        text.contains("create_agent"),
        "fuzzy search 'create agent' should find create_agent, got: {text}"
    );
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn test_mcp_discover_capabilities() {
    let server = TestServer::in_memory().await;
    let resp = mcp_tool_call(&server, "discover", json!({ "query": "capabilities" })).await;

    assert!(
        !tool_is_error(&resp),
        "discover capabilities failed: {}",
        tool_text(&resp)
    );
    let payload = tool_json(&resp);
    let text = payload.to_string();
    assert!(
        text.contains("list_capabilities"),
        "Should find list_capabilities, got: {text}"
    );
    assert!(
        text.contains("get_capability"),
        "Should find get_capability, got: {text}"
    );
    let list_capabilities = payload["operations"]
        .as_array()
        .unwrap()
        .iter()
        .find(|op| op["name"] == "list_capabilities")
        .expect("list_capabilities operation");
    assert_eq!(list_capabilities["output_shape"], "paginated");
}

// ============================================================================
// Tier 2: query/execute scripted tools
// ============================================================================

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn test_mcp_query_list_harnesses() {
    let server = TestServer::in_memory().await;
    let resp = mcp_tool_call(&server, "query", json!({ "commands": "list_harnesses" })).await;
    assert!(
        !tool_is_error(&resp),
        "query list_harnesses failed: {}",
        tool_text(&resp)
    );

    let result = tool_json(&resp);
    let harnesses = result.as_array().expect("Expected harnesses array");
    assert!(!harnesses.is_empty(), "Should have seed harnesses");
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn test_mcp_query_missing_commands() {
    let server = TestServer::in_memory().await;
    let resp = mcp_tool_call(&server, "query", json!({})).await;
    assert!(tool_is_error(&resp), "Expected error for missing commands");
    assert!(
        tool_text(&resp).contains("commands"),
        "Expected commands guidance, got: {}",
        tool_text(&resp)
    );
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn test_mcp_execute_missing_commands() {
    let server = TestServer::in_memory().await;
    let resp = mcp_tool_call(&server, "execute", json!({})).await;
    assert!(tool_is_error(&resp), "Expected error for missing commands");
    assert!(
        tool_text(&resp).contains("commands"),
        "Expected commands guidance, got: {}",
        tool_text(&resp)
    );
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn test_mcp_query_list_harnesses_wrong_jq_shape_is_concise() {
    let server = TestServer::in_memory().await;
    let resp = mcp_tool_call(
        &server,
        "query",
        json!({
            "commands": "list_harnesses --limit 100 | jq -c '.data[] | {id, name, system_prompt}'"
        }),
    )
    .await;

    assert!(tool_is_error(&resp), "Expected jq shape mismatch error");
    let text = tool_text(&resp);
    assert!(
        text.contains("jq: runtime error") || text.contains("jq: error"),
        "Expected jq error, got: {text}"
    );
    assert!(
        text.contains("output_shape"),
        "Expected discover guidance, got: {text}"
    );
    assert!(
        !text.contains("system_prompt"),
        "Error must not dump harness fields: {text}"
    );
    assert!(
        !text.contains("You are"),
        "Error must not dump prompt content: {text}"
    );
    assert!(
        text.len() < 240,
        "Error should be concise, got {} chars: {text}",
        text.len()
    );
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn test_mcp_query_allows_read_only_post_helpers() {
    let server = TestServer::in_memory().await;
    let resp = mcp_tool_call(
        &server,
        "query",
        json!({ "commands": "preview_agent --system_prompt 'Preview via query'" }),
    )
    .await;
    assert!(
        !tool_is_error(&resp),
        "query preview_agent failed: {}",
        tool_text(&resp)
    );

    let result = tool_json(&resp);
    assert_eq!(result["system_prompt"], "Preview via query");
    assert!(result["tools"].is_array());
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn test_mcp_query_rejects_mutating_commands() {
    let server = TestServer::in_memory().await;
    let resp = mcp_tool_call(
        &server,
        "query",
        json!({
            "commands": "create_agent --name 'query-should-fail' --display_name 'Query Should Fail' --system_prompt 'nope'"
        }),
    )
    .await;
    assert!(
        tool_is_error(&resp),
        "query create_agent unexpectedly succeeded"
    );

    let text = tool_text(&resp);
    assert!(
        text.contains("create_agent"),
        "Expected command name in error, got: {text}"
    );
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn test_mcp_execute_health_check() {
    let (_server, url) = TestServer::serving().await;

    let resp = mcp_tool_call_http(&url, "execute", json!({ "commands": "health_check" })).await;
    assert!(
        !tool_is_error(&resp),
        "execute health_check failed: {}",
        tool_text(&resp)
    );

    let result = tool_json(&resp);
    assert_eq!(result["status"], "ok");
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn test_mcp_execute_list_harnesses() {
    let (_server, url) = TestServer::serving().await;

    let resp = mcp_tool_call_http(&url, "execute", json!({ "commands": "list_harnesses" })).await;
    assert!(
        !tool_is_error(&resp),
        "execute list_harnesses failed: {}",
        tool_text(&resp)
    );

    let result = tool_json(&resp);
    let harnesses = result["data"].as_array().expect("Expected harnesses array");
    assert!(!harnesses.is_empty(), "Should have seed harnesses");

    // Verify seed harnesses exist (Base and Generic are always seeded)
    let names: Vec<&str> = harnesses
        .iter()
        .filter_map(|h| h["name"].as_str())
        .collect();
    assert!(names.contains(&"base"), "Should have Base harness");
    assert!(names.contains(&"generic"), "Should have Generic harness");
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn test_mcp_execute_get_session_context_report() {
    let server = TestServer::in_memory().await;
    let session: Value = server
        .post(
            "/v1/sessions",
            json!({
                "harness_id": server.seed_base_harness_id,
                "title": "MCP Context Report"
            }),
        )
        .await
        .assert_status(StatusCode::CREATED)
        .json();
    let session_id = session["id"].as_str().expect("session id").to_string();

    let resp = mcp_tool_call(
        &server,
        "execute",
        json!({ "commands": format!("get_session_context_report {session_id}") }),
    )
    .await;
    assert!(
        !tool_is_error(&resp),
        "execute get_session_context_report failed: {}",
        tool_text(&resp)
    );

    let result = tool_json(&resp);
    assert_eq!(result["session_id"], session_id);
    assert_eq!(result["estimated_input_tokens"], 0);
}

#[tokio::test]
async fn test_mcp_execute_session_files_reads_virtual_mounts() {
    let server = TestServer::in_memory().await;

    let agent = server
        .post(
            "/v1/agents",
            json!({
                "name": format!("mcp-virtual-files-{}", unique_suffix()),
                "display_name": "MCP Virtual Files",
                "system_prompt": "Test",
            }),
        )
        .await
        .assert_status(StatusCode::CREATED)
        .json_value();

    let session = server
        .post(
            "/v1/sessions",
            json!({
                "harness_id": server.seed_base_harness_id,
                "agent_id": agent["id"].as_str().unwrap(),
            }),
        )
        .await
        .assert_status(StatusCode::CREATED)
        .json_value();

    let session_id: SessionId = session["id"]
        .as_str()
        .expect("session id")
        .parse()
        .expect("valid session id");

    let mut tree = VirtualFileTree::new();
    tree.insert_text("/docs/readme.md", "# Hello");
    server.virtual_registry.register(
        session_id.uuid(),
        "/docs".into(),
        std::sync::Arc::new(tree),
        "test_cap".into(),
    );

    let resp = mcp_tool_call(
        &server,
        "execute",
        json!({ "commands": format!("get_session_file --session_id {} --path '/docs/readme.md'", session_id) }),
    )
    .await;

    assert!(
        !tool_is_error(&resp),
        "execute get_session_file failed: {}",
        tool_text(&resp)
    );
    let payload = tool_json(&resp);
    assert_eq!(payload["path"], "/docs/readme.md");
    assert_eq!(payload["content"], "# Hello");
    assert_eq!(payload["is_readonly"], true);
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn test_mcp_execute_list_agents() {
    let (_server, url) = TestServer::serving().await;

    let resp = mcp_tool_call_http(&url, "execute", json!({ "commands": "list_agents" })).await;
    assert!(
        !tool_is_error(&resp),
        "execute list_agents failed: {}",
        tool_text(&resp)
    );

    let result = tool_json(&resp);
    assert!(result["data"].is_array());
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn test_mcp_execute_create_and_get_agent() {
    let (_server, url) = TestServer::serving().await;

    // Create agent via execute
    let resp = mcp_tool_call_http(
        &url,
        "execute",
        json!({
            "commands": "create_agent --name 'mcp-execute-agent' --display_name 'MCP Execute Agent' --system_prompt 'Test prompt'"
        }),
    )
    .await;
    assert!(
        !tool_is_error(&resp),
        "create_agent failed: {}",
        tool_text(&resp)
    );

    let created = tool_json(&resp);
    let agent_id = created["id"].as_str().unwrap();
    assert!(agent_id.starts_with("agent_"));

    // Get agent by ID
    let resp = mcp_tool_call_http(
        &url,
        "execute",
        json!({ "commands": format!("get_agent --id {agent_id}") }),
    )
    .await;
    assert!(
        !tool_is_error(&resp),
        "get_agent failed: {}",
        tool_text(&resp)
    );

    let fetched = tool_json(&resp);
    assert_eq!(fetched["name"], "mcp-execute-agent");
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn test_mcp_execute_list_models() {
    let (_server, url) = TestServer::serving().await;

    let resp = mcp_tool_call_http(&url, "execute", json!({ "commands": "list_models" })).await;
    assert!(
        !tool_is_error(&resp),
        "execute list_models failed: {}",
        tool_text(&resp)
    );
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn test_mcp_execute_list_capabilities() {
    let (_server, url) = TestServer::serving().await;

    let resp =
        mcp_tool_call_http(&url, "execute", json!({ "commands": "list_capabilities" })).await;
    assert!(
        !tool_is_error(&resp),
        "execute list_capabilities failed: {}",
        tool_text(&resp)
    );
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn test_mcp_execute_app_commands_support_schedule_and_webhook_channels() {
    let server = TestServer::in_memory().await;
    let app_name = format!("repo-checker-{}", unique_suffix());

    let create_resp = mcp_tool_call(
        &server,
        "execute",
        json!({
            "commands": format!(
                "create_app --name '{app_name}' --description 'repo checks' --harness_id {}",
                server.seed_generic_harness_id
            )
        }),
    )
    .await;
    assert!(
        !tool_is_error(&create_resp),
        "create_app failed: {}",
        tool_text(&create_resp)
    );
    let app = tool_json(&create_resp);
    let app_id = app["id"].as_str().expect("app id");

    let add_schedule_resp = mcp_tool_call(
        &server,
        "execute",
        json!({
            "commands": format!(
                "add_schedule_app_channel --app_id {app_id} --cron_expression '0 * * * * * *' --timezone UTC --session_mode shared_session --message 'run checks'"
            )
        }),
    )
    .await;
    assert!(
        !tool_is_error(&add_schedule_resp),
        "add_app_channel schedule failed: {}",
        tool_text(&add_schedule_resp)
    );
    assert_eq!(tool_json(&add_schedule_resp)["channel_type"], "schedule");

    let add_webhook_resp = mcp_tool_call(
        &server,
        "execute",
        json!({
            "commands": format!(
                "add_webhook_app_channel --app_id {app_id} --token 'secret-1' --session_mode session_per_invocation --message 'process payload'"
            )
        }),
    )
    .await;
    assert!(
        !tool_is_error(&add_webhook_resp),
        "add_app_channel webhook failed: {}",
        tool_text(&add_webhook_resp)
    );
    assert_eq!(tool_json(&add_webhook_resp)["channel_type"], "webhook");

    let get_resp = mcp_tool_call(
        &server,
        "execute",
        json!({ "commands": format!("get_app {app_id}") }),
    )
    .await;
    assert!(
        !tool_is_error(&get_resp),
        "get_app failed: {}",
        tool_text(&get_resp)
    );
    let fetched_app = tool_json(&get_resp);
    let channel_types = fetched_app["channels"]
        .as_array()
        .expect("channels array")
        .iter()
        .filter_map(|channel| channel["channel_type"].as_str())
        .collect::<Vec<_>>();
    assert!(channel_types.contains(&"schedule"));
    assert!(channel_types.contains(&"webhook"));
}

// Contract matrix for recent MCP execute regressions:
// - EVE-323: positional path args must work on execute commands
// - EVE-324: stringly flags must coerce into typed command params
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn test_mcp_execute_argument_coercion_contract_matrix() {
    let (_server, url) = TestServer::serving().await;

    let flag_cases = vec![
        (
            "inventory limit string coerces to integer",
            "list_capabilities --limit 5".to_string(),
            vec![("limit", json!(5))],
            Some(5usize),
        ),
        (
            "inventory numeric pagination flags survive round-trip",
            "list_agents --offset 0 --limit 10".to_string(),
            vec![("offset", json!(0)), ("limit", json!(10))],
            None,
        ),
    ];

    for (case, commands, expected_fields, max_len) in flag_cases {
        let resp = execute_http_command(&url, commands).await;
        assert!(!tool_is_error(&resp), "{case}: {}", tool_text(&resp));

        let payload = tool_json(&resp);
        assert!(payload["data"].is_array(), "{case}: expected data array");
        for (field, expected) in expected_fields {
            assert_eq!(payload[field], expected, "{case}: unexpected {field}");
        }
        if let Some(max_len) = max_len {
            assert!(
                payload["data"].as_array().unwrap().len() <= max_len,
                "{case}: expected at most {max_len} rows"
            );
        }
    }

    let (archived_agent_id, archived_agent_name) =
        create_agent_via_execute(&url, "eve343-bool-flag-agent", "EVE-343 Bool Flag").await;
    let delete_resp = execute_http_command(&url, format!("delete_agent {archived_agent_id}")).await;
    assert!(
        !tool_is_error(&delete_resp),
        "inventory bool flag setup failed: {}",
        tool_text(&delete_resp)
    );
    assert_eq!(tool_json(&delete_resp)["deleted"], json!(true));

    let resp = execute_http_command(
        &url,
        format!("list_agents --include_archived true --search {archived_agent_name} --limit 10"),
    )
    .await;
    assert!(
        !tool_is_error(&resp),
        "inventory bool flag survives round-trip: {}",
        tool_text(&resp)
    );
    let payload = tool_json(&resp);
    assert_eq!(payload["limit"], json!(10));
    assert!(
        payload["data"].as_array().unwrap().iter().any(|agent| {
            agent["id"] == json!(archived_agent_id) && agent["name"] == json!(archived_agent_name)
        }),
        "inventory bool flag survives round-trip: archived agent missing from include_archived results"
    );

    let (agent_id, agent_name) = create_agent_via_execute(
        &url,
        "eve343-positional-get-agent",
        "EVE-343 Positional Get",
    )
    .await;
    let (session_id, session_title) =
        create_session_via_execute(&url, "EVE-343 Positional Session").await;
    let (delete_agent_id, _) = create_agent_via_execute(
        &url,
        "eve343-positional-delete-agent",
        "EVE-343 Positional Delete",
    )
    .await;

    let positional_cases = vec![
        (
            "inventory positional id rewrite",
            format!("get_agent {agent_id}"),
            vec![("id", json!(agent_id)), ("name", json!(agent_name))],
        ),
        (
            "static catalog positional session_id rewrite",
            format!("get_session {session_id}"),
            vec![("id", json!(session_id)), ("title", json!(session_title))],
        ),
        (
            "inventory positional delete rewrite",
            format!("delete_agent {delete_agent_id}"),
            vec![("deleted", json!(true))],
        ),
    ];

    for (case, commands, expected_fields) in positional_cases {
        let resp = execute_http_command(&url, commands).await;
        assert!(!tool_is_error(&resp), "{case}: {}", tool_text(&resp));

        let payload = tool_json(&resp);
        for (field, expected) in expected_fields {
            assert_eq!(payload[field], expected, "{case}: unexpected {field}");
        }
    }
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn test_mcp_execute_list_mcp_servers() {
    let (_server, url) = TestServer::serving().await;

    let resp = mcp_tool_call_http(&url, "execute", json!({ "commands": "list_mcp_servers" })).await;
    assert!(
        !tool_is_error(&resp),
        "execute list_mcp_servers failed: {}",
        tool_text(&resp)
    );
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn test_mcp_execute_create_mcp_server() {
    let (_server, url) = TestServer::serving().await;

    // Use unique name to avoid conflicts with prior test runs
    let unique_name = format!(
        "Test MCP {}",
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_millis()
    );
    let resp = mcp_tool_call_http(
        &url,
        "execute",
        json!({
            "commands": format!("create_mcp_server --name '{unique_name}' --url 'https://mcp.test.com/v1'")
        }),
    )
    .await;
    assert!(
        !tool_is_error(&resp),
        "create_mcp_server failed: {}",
        tool_text(&resp)
    );

    let created = tool_json(&resp);
    assert_eq!(created["name"], unique_name);
    assert!(created["id"].as_str().unwrap().starts_with("mcp_"));
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn test_mcp_adversarial_tool_chain_cannot_escape_org_scope() {
    let server = TestServer::in_memory().await;
    let marker = format!("org2-agent-marker-{}", unique_suffix());
    let escaped_name = format!("mcp-escape-success-{}", unique_suffix());

    let org2: Value = server
        .post(
            "/v1/orgs",
            json!({ "name": format!("MCP Escape Target {}", unique_suffix()) }),
        )
        .await
        .assert_status(StatusCode::CREATED)
        .json();
    let org2_id = org2["id"].as_str().expect("org2 id").to_string();

    let switch_resp = server
        .post("/v1/users/me/switch-org", json!({ "org_id": org2_id }))
        .await
        .assert_status(StatusCode::OK);
    let org2_cookie = extract_cookie(switch_resp.headers(), "everruns_org");

    let create_agent_resp = mcp_tool_call_with_headers(
        &server,
        "execute",
        json!({
            "commands": format!(
                "create_agent --name '{marker}' --display_name 'Org 2 Marker' --system_prompt 'tenant marker'"
            )
        }),
        vec![("cookie", org2_cookie.as_str())],
    )
    .await;
    assert!(
        !tool_is_error(&create_agent_resp),
        "org2 setup create_agent failed: {}",
        tool_text(&create_agent_resp)
    );
    let org2_agent = tool_json(&create_agent_resp);
    let org2_agent_id = org2_agent["id"].as_str().expect("org2 agent id");

    let run_org2_resp = mcp_tool_call_with_headers(
        &server,
        "agent_run",
        json!({
            "agent_id": org2_agent_id,
            "message": "seed cross-org session",
            "title": "org2 adversarial session",
        }),
        vec![("cookie", org2_cookie.as_str())],
    )
    .await;
    assert!(
        !tool_is_error(&run_org2_resp),
        "org2 setup agent_run failed: {}",
        tool_text(&run_org2_resp)
    );
    let org2_session = tool_json(&run_org2_resp);
    let org2_session_id = org2_session["session_id"]
        .as_str()
        .expect("org2 session id");

    let discover_agents = mcp_tool_call(
        &server,
        "discover",
        json!({ "query": "agent", "include_schemas": true }),
    )
    .await;
    assert!(
        !tool_is_error(&discover_agents),
        "discover agent failed: {}",
        tool_text(&discover_agents)
    );
    let discover_agent_text = tool_text(&discover_agents);
    assert!(discover_agent_text.contains("get_agent"));
    assert!(discover_agent_text.contains("update_agent"));

    let discover_sessions = mcp_tool_call(
        &server,
        "discover",
        json!({ "query": "session", "include_schemas": true }),
    )
    .await;
    assert!(
        !tool_is_error(&discover_sessions),
        "discover session failed: {}",
        tool_text(&discover_sessions)
    );
    let discover_session_text = tool_text(&discover_sessions);
    assert!(discover_session_text.contains("get_session"));
    assert!(discover_session_text.contains("create_message"));

    let wrong_org_query = mcp_tool_call(
        &server,
        "query",
        json!({ "commands": format!("get_agent {org2_agent_id}") }),
    )
    .await;
    assert!(
        tool_is_error(&wrong_org_query),
        "default org query unexpectedly read org2 agent: {}",
        tool_text(&wrong_org_query)
    );
    assert!(
        !tool_text(&wrong_org_query).contains(&marker),
        "cross-org query leaked org2 marker: {}",
        tool_text(&wrong_org_query)
    );

    let wrong_org_probe = mcp_tool_call(
        &server,
        "query",
        json!({
            "commands": format!(
                "list_agents --limit 200 | jq -e --arg id '{org2_agent_id}' '.data[] | select(.id == $id)'"
            )
        }),
    )
    .await;
    assert!(
        tool_is_error(&wrong_org_probe),
        "default org query unexpectedly found org2 agent: {}",
        tool_text(&wrong_org_probe)
    );
    assert!(
        !tool_text(&wrong_org_probe).contains(&marker),
        "cross-org list probe leaked org2 marker: {}",
        tool_text(&wrong_org_probe)
    );

    let wrong_org_execute = mcp_tool_call(
        &server,
        "execute",
        json!({
            "commands": format!("update_agent --id {org2_agent_id} --name '{escaped_name}'")
        }),
    )
    .await;
    assert!(
        tool_is_error(&wrong_org_execute),
        "default org execute unexpectedly mutated org2 agent: {}",
        tool_text(&wrong_org_execute)
    );

    let wrong_org_agent_run = mcp_tool_call(
        &server,
        "agent_run",
        json!({
            "agent_id": org2_agent_id,
            "message": "run the other tenant's agent",
            "title": "cross-org run attempt",
        }),
    )
    .await;
    assert!(
        tool_is_error(&wrong_org_agent_run),
        "default org agent_run unexpectedly used org2 agent: {}",
        tool_text(&wrong_org_agent_run)
    );

    let wrong_org_status = mcp_tool_call(
        &server,
        "session_get_status",
        json!({ "session_id": org2_session_id }),
    )
    .await;
    assert!(
        tool_is_error(&wrong_org_status),
        "default org session_get_status unexpectedly read org2 session: {}",
        tool_text(&wrong_org_status)
    );

    let wrong_org_send = mcp_tool_call(
        &server,
        "session_send_message",
        json!({
            "session_id": org2_session_id,
            "message": "write into the other tenant session",
        }),
    )
    .await;
    assert!(
        tool_is_error(&wrong_org_send),
        "default org session_send_message unexpectedly wrote org2 session: {}",
        tool_text(&wrong_org_send)
    );

    let fake_org_override = mcp_tool_call(
        &server,
        "execute",
        json!({
            "organization_id": "org_ffffffffffffffffffffffffffffffff",
            "commands": format!("update_agent --id {org2_agent_id} --name '{escaped_name}'"),
        }),
    )
    .await;
    assert!(
        tool_is_error(&fake_org_override),
        "non-member organization_id override unexpectedly executed: {}",
        tool_text(&fake_org_override)
    );
    assert!(
        tool_text(&fake_org_override).contains("Organization not found")
            || tool_text(&fake_org_override).contains("not a member"),
        "expected membership failure, got: {}",
        tool_text(&fake_org_override)
    );

    let default_org_override = mcp_tool_call(
        &server,
        "query",
        json!({
            "organization_id": DEFAULT_ORG_PUBLIC_ID,
            "commands": "list_agents --limit 5",
        }),
    )
    .await;
    assert!(
        !tool_is_error(&default_org_override),
        "default org override should remain usable: {}",
        tool_text(&default_org_override)
    );
    assert!(
        !tool_text(&default_org_override).contains(&marker),
        "default org override leaked org2 marker: {}",
        tool_text(&default_org_override)
    );

    let org2_after_attack = mcp_tool_call_with_headers(
        &server,
        "query",
        json!({ "commands": format!("get_agent {org2_agent_id}") }),
        vec![("cookie", org2_cookie.as_str())],
    )
    .await;
    assert!(
        !tool_is_error(&org2_after_attack),
        "org2 verification get_agent failed: {}",
        tool_text(&org2_after_attack)
    );
    let org2_agent_after_attack = tool_json(&org2_after_attack);
    assert_eq!(org2_agent_after_attack["name"], marker);
    assert_ne!(org2_agent_after_attack["name"], escaped_name);
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn test_mcp_resources_read_cannot_escape_org_scope() {
    let server = TestServer::in_memory().await;
    let marker = format!("org2-resource-marker-{}", unique_suffix());

    let org2: Value = server
        .post(
            "/v1/orgs",
            json!({ "name": format!("MCP Resource Target {}", unique_suffix()) }),
        )
        .await
        .assert_status(StatusCode::CREATED)
        .json();
    let org2_id = org2["id"].as_str().expect("org2 id").to_string();

    let switch_resp = server
        .post("/v1/users/me/switch-org", json!({ "org_id": org2_id }))
        .await
        .assert_status(StatusCode::OK);
    let org2_cookie = extract_cookie(switch_resp.headers(), "everruns_org");

    let create_agent_resp = mcp_tool_call_with_headers(
        &server,
        "execute",
        json!({
            "commands": format!(
                "create_agent --name '{marker}' --display_name 'Org 2 Resource Marker' --system_prompt 'tenant marker'"
            )
        }),
        vec![("cookie", org2_cookie.as_str())],
    )
    .await;
    assert!(
        !tool_is_error(&create_agent_resp),
        "org2 setup create_agent failed: {}",
        tool_text(&create_agent_resp)
    );

    let default_resources = mcp_call(
        &server,
        "resources/read",
        json!({ "uri": "everruns://agents" }),
    )
    .await;
    assert!(
        default_resources["error"].is_null(),
        "default resources/read failed: {default_resources}"
    );
    let default_text = default_resources["result"]["contents"][0]["text"]
        .as_str()
        .expect("default resource text");
    assert!(
        !default_text.contains(&marker),
        "default org resources/read leaked org2 marker: {default_text}"
    );

    let org2_resources = mcp_call_with_headers(
        &server,
        "resources/read",
        json!({ "uri": "everruns://agents" }),
        vec![("cookie", org2_cookie.as_str())],
    )
    .await;
    assert!(
        org2_resources["error"].is_null(),
        "org2 resources/read failed: {org2_resources}"
    );
    let org2_text = org2_resources["result"]["contents"][0]["text"]
        .as_str()
        .expect("org2 resource text");
    assert!(
        org2_text.contains(&marker),
        "org2 resources/read should see its own marker: {org2_text}"
    );
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn test_mcp_execute_bash_pipe() {
    let (_server, url) = TestServer::serving().await;

    // Use a pipe to count harnesses (tests bash pipe support in execute)
    let resp = mcp_tool_call_http(
        &url,
        "execute",
        json!({ "commands": "list_harnesses | jq '.data | length'" }),
    )
    .await;
    assert!(!tool_is_error(&resp), "pipe failed: {}", tool_text(&resp));

    let count: i64 = tool_text(&resp).trim().parse().unwrap();
    assert!(count >= 2, "Should have at least 2 harnesses, got {count}");
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn test_mcp_execute_bash_script() {
    let (_server, url) = TestServer::serving().await;

    // Multi-line script: create agent then list to verify
    let script = r#"
        create_agent --name 'script-agent' --display_name 'Script Agent' --system_prompt 'scripted' > /dev/null
        list_agents | jq '.data | length'
    "#;

    let resp = mcp_tool_call_http(&url, "execute", json!({ "commands": script })).await;
    assert!(
        !tool_is_error(&resp),
        "bash script failed: {}",
        tool_text(&resp)
    );

    let count: i64 = tool_text(&resp).trim().parse().unwrap();
    assert!(count >= 1, "Should have at least 1 agent after creation");
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn test_mcp_execute_list_providers() {
    let (_server, url) = TestServer::serving().await;

    let resp = mcp_tool_call_http(&url, "execute", json!({ "commands": "list_providers" })).await;
    assert!(
        !tool_is_error(&resp),
        "list_providers failed: {}",
        tool_text(&resp)
    );
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn test_mcp_execute_list_orgs() {
    let (_server, url) = TestServer::serving().await;

    let resp = mcp_tool_call_http(&url, "execute", json!({ "commands": "list_orgs" })).await;
    assert!(
        !tool_is_error(&resp),
        "list_orgs failed: {}",
        tool_text(&resp)
    );
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn test_mcp_execute_discover_categories() {
    let (_server, url) = TestServer::serving().await;

    let resp = mcp_tool_call_http(
        &url,
        "execute",
        json!({ "commands": "discover --categories" }),
    )
    .await;
    assert!(
        !tool_is_error(&resp),
        "discover --categories failed: {}",
        tool_text(&resp)
    );

    let text = tool_text(&resp);
    assert!(text.contains("agents"), "Should list agents category");
    assert!(text.contains("sessions"), "Should list sessions category");
    assert!(text.contains("harnesses"), "Should list harnesses category");
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn test_mcp_execute_discover_all() {
    let (_server, url) = TestServer::serving().await;

    let resp = mcp_tool_call_http(&url, "execute", json!({ "commands": "discover --all" })).await;
    assert!(
        !tool_is_error(&resp),
        "discover --all failed: {}",
        tool_text(&resp)
    );

    let result = tool_json(&resp);
    assert!(result["success"].as_bool().unwrap());
    let stdout = result["stdout"].as_str().unwrap();
    assert!(stdout.contains("agents"), "Should list agents category");
    assert!(stdout.contains("sessions"), "Should list sessions category");
}

// ============================================================================
// Tier 2: execute — CRUD workflow via MCP
// ============================================================================

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn test_mcp_execute_agent_crud_workflow() {
    let (_server, url) = TestServer::serving().await;

    // Create
    let resp = mcp_tool_call_http(
        &url,
        "execute",
        json!({
            "commands": "create_agent --name 'crud-agent' --display_name 'CRUD Agent' --system_prompt 'CRUD test'"
        }),
    )
    .await;
    assert!(
        !tool_is_error(&resp),
        "create_agent failed: {}",
        tool_text(&resp)
    );
    let created = tool_json(&resp);
    let agent_id = created["id"].as_str().unwrap().to_string();

    // Update
    let resp = mcp_tool_call_http(
        &url,
        "execute",
        json!({
            "commands": format!("update_agent --id {agent_id} --name 'updated-crud-agent'")
        }),
    )
    .await;
    assert!(
        !tool_is_error(&resp),
        "update_agent failed: {}",
        tool_text(&resp)
    );
    let updated = tool_json(&resp);
    assert_eq!(updated["name"], "updated-crud-agent");

    // Delete (archive)
    let resp = mcp_tool_call_http(
        &url,
        "execute",
        json!({ "commands": format!("delete_agent --id {agent_id}") }),
    )
    .await;
    assert!(
        !tool_is_error(&resp),
        "delete_agent failed: {}",
        tool_text(&resp)
    );
}

// EVE-409: bashkit's flag parser stringifies any flag whose schema is not a
// scalar, so JSON arrays/objects passed to generated builtins arrive at the
// MCP catalog callback as strings. `coerce_json_text_params` parses them
// back into structured JSON before the domain dispatcher sees them. These
// tests exercise both `create_agent` and `update_agent` end-to-end through
// the bashkit pipeline to make sure the fix works for every typed-aggregate
// field on every generated operation, including the `#[serde(flatten)]`
// shape used by `UpdateAgentCmd`.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn test_mcp_execute_array_flags_create_and_update_agent() {
    let (_server, url) = TestServer::serving().await;

    // create_agent with --capabilities and --tags as JSON-stringified arrays.
    let suffix = unique_suffix();
    let create_cmd = format!(
        "create_agent --name 'eve409-arr-{suffix}' --display_name 'EVE-409 Array Flags' \
         --system_prompt 'Test array flags.' \
         --tags '[\"alpha\",\"beta\"]' \
         --capabilities '[{{\"ref\":\"current_time\"}}]'"
    );
    let resp = execute_http_command(&url, create_cmd).await;
    assert!(
        !tool_is_error(&resp),
        "create_agent with array flags failed: {}",
        tool_text(&resp)
    );
    let created = tool_json(&resp);
    let agent_id = created["id"].as_str().unwrap().to_string();
    assert_eq!(
        created["tags"],
        json!(["alpha", "beta"]),
        "tags should round-trip as an array"
    );
    let cap_refs: Vec<String> = created["capabilities"]
        .as_array()
        .expect("capabilities array")
        .iter()
        .map(|c| c["ref"].as_str().unwrap_or_default().to_string())
        .collect();
    assert!(
        cap_refs.iter().any(|r| r == "current_time"),
        "expected current_time capability, got: {cap_refs:?}"
    );

    // update_agent --capabilities '[{"ref":"current_time"}]' — the exact
    // failing case from EVE-409.
    let update_resp = execute_http_command(
        &url,
        format!("update_agent --id {agent_id} --capabilities '[{{\"ref\":\"current_time\"}}]'"),
    )
    .await;
    assert!(
        !tool_is_error(&update_resp),
        "update_agent --capabilities failed: {}",
        tool_text(&update_resp)
    );
    let updated_caps: Vec<String> = tool_json(&update_resp)["capabilities"]
        .as_array()
        .expect("capabilities array")
        .iter()
        .map(|c| c["ref"].as_str().unwrap_or_default().to_string())
        .collect();
    assert!(
        updated_caps.iter().any(|r| r == "current_time"),
        "update_agent should persist capabilities, got: {updated_caps:?}"
    );

    // update_agent --tags '["a","b"]'
    let tags_resp = execute_http_command(
        &url,
        format!("update_agent --id {agent_id} --tags '[\"codex-created\",\"joke\"]'"),
    )
    .await;
    assert!(
        !tool_is_error(&tags_resp),
        "update_agent --tags failed: {}",
        tool_text(&tags_resp)
    );
    assert_eq!(
        tool_json(&tags_resp)["tags"],
        json!(["codex-created", "joke"]),
        "update_agent --tags should round-trip"
    );

    // Malformed JSON in an array flag must still fail loudly so the user
    // gets a non-zero exit. Bashkit sanitizes the callback message itself
    // (THREAT[TM-INF-030]) so we only assert that the call errors out and
    // that we did not silently coerce garbage into success.
    let bad_resp = execute_http_command(
        &url,
        format!("update_agent --id {agent_id} --tags '[not, valid'"),
    )
    .await;
    assert!(
        tool_is_error(&bad_resp),
        "expected error for malformed --tags JSON"
    );
}

// ============================================================================
// Tier 1 + Tier 2 combined: full flow via MCP
// ============================================================================

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn test_mcp_full_flow_agent_run_then_status() {
    let server = TestServer::new().await;

    // Create agent
    let agent: Value = server
        .post(
            "/v1/agents",
            json!({
                "name": "flow-agent",
                "display_name": "Flow Agent",
                "system_prompt": "You are a flow test agent."
            }),
        )
        .await
        .assert_success()
        .json();
    let agent_id = agent["id"].as_str().unwrap();

    // agent_run
    let run_resp = mcp_tool_call(
        &server,
        "agent_run",
        json!({
            "agent_id": agent_id,
            "message": "What is 2+2?",
            "title": "Math test"
        }),
    )
    .await;
    assert!(!tool_is_error(&run_resp));
    let run_result = tool_json(&run_resp);
    let session_id = run_result["session_id"].as_str().unwrap();

    // session_get_status
    let status_resp = mcp_tool_call(
        &server,
        "session_get_status",
        json!({ "session_id": session_id }),
    )
    .await;
    assert!(!tool_is_error(&status_resp));
    let status = tool_json(&status_resp);
    assert_eq!(status["session_id"], session_id);

    // session_send_message
    let msg_resp = mcp_tool_call(
        &server,
        "session_send_message",
        json!({
            "session_id": session_id,
            "message": "And what is 3+3?"
        }),
    )
    .await;
    assert!(!tool_is_error(&msg_resp));

    // Check status again — should have more events
    let status2_resp = mcp_tool_call(
        &server,
        "session_get_status",
        json!({ "session_id": session_id }),
    )
    .await;
    assert!(!tool_is_error(&status2_resp));
    let status2 = tool_json(&status2_resp);
    let first_event_count = status["event_count"].as_i64().unwrap_or(0);
    let second_event_count = status2["event_count"].as_i64().unwrap();
    assert!(
        second_event_count > first_event_count,
        "Expected more events after send_message (before={first_event_count}, after={second_event_count})"
    );
}

// ============================================================================
// MCP OAuth token endpoint tests
// ============================================================================

/// Helper: register an OAuth client and return client_id + client_secret.
async fn register_oauth_client(server: &TestServer) -> (String, String) {
    let resp: Value = server
        .post(
            "/oauth/register",
            json!({
                "client_name": "test-client",
                "redirect_uris": ["http://localhost:9999/callback"]
            }),
        )
        .await
        .assert_success()
        .json();
    let client_id = resp["client_id"].as_str().unwrap().to_string();
    let client_secret = resp["client_secret"].as_str().unwrap().to_string();
    (client_id, client_secret)
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn test_oauth_token_json_invalid_grant_type() {
    let server = TestServer::new().await;
    let resp = server
        .request_raw(
            axum::http::Method::POST,
            "/oauth/token",
            vec![("content-type", "application/json")],
            serde_json::to_vec(&json!({
                "grant_type": "invalid_type",
                "client_id": "nonexistent"
            }))
            .unwrap(),
        )
        .await;
    assert_eq!(resp.status(), axum::http::StatusCode::BAD_REQUEST);
    let body: Value = resp.json();
    assert_eq!(body["error"], "unsupported_grant_type");
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn test_oauth_token_form_encoded_invalid_grant_type() {
    let server = TestServer::new().await;
    let resp = server
        .request_raw(
            axum::http::Method::POST,
            "/oauth/token",
            vec![("content-type", "application/x-www-form-urlencoded")],
            b"grant_type=invalid_type&client_id=nonexistent".to_vec(),
        )
        .await;
    assert_eq!(resp.status(), axum::http::StatusCode::BAD_REQUEST);
    let body: Value = resp.json();
    assert_eq!(body["error"], "unsupported_grant_type");
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn test_oauth_token_json_missing_code() {
    let server = TestServer::new().await;
    let (client_id, _) = register_oauth_client(&server).await;
    let resp = server
        .request_raw(
            axum::http::Method::POST,
            "/oauth/token",
            vec![("content-type", "application/json")],
            serde_json::to_vec(&json!({
                "grant_type": "authorization_code",
                "client_id": client_id
            }))
            .unwrap(),
        )
        .await;
    assert_eq!(resp.status(), axum::http::StatusCode::BAD_REQUEST);
    let body: Value = resp.json();
    assert_eq!(body["error"], "invalid_request");
    assert!(
        body["error_description"]
            .as_str()
            .unwrap_or("")
            .contains("code"),
    );
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn test_oauth_token_invalid_content_type_falls_back_to_form() {
    let server = TestServer::new().await;
    // With a random content-type, it should try form parsing and fail gracefully
    let resp = server
        .request_raw(
            axum::http::Method::POST,
            "/oauth/token",
            vec![("content-type", "text/plain")],
            b"this is not valid form data at all {{{".to_vec(),
        )
        .await;
    assert_eq!(resp.status(), axum::http::StatusCode::BAD_REQUEST);
    let body: Value = resp.json();
    assert_eq!(body["error"], "invalid_request");
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn test_oauth_register_rejects_unsafe_redirect_uris() {
    let server = TestServer::in_memory().await;
    for uri in [
        "javascript:alert(1)",
        "data:text/html,<script>alert(1)</script>",
        "file:///tmp/cb",
        "http://example.com/cb",
        "myapp://callback",
        "//example.com/cb",
        "/relative",
        "",
    ] {
        let resp = server
            .post(
                "/oauth/register",
                json!({
                    "client_name": "unsafe-client",
                    "redirect_uris": [uri],
                }),
            )
            .await;
        assert_eq!(
            resp.status(),
            axum::http::StatusCode::BAD_REQUEST,
            "expected register to reject {uri}",
        );
        let body: Value = resp.json();
        assert_eq!(body["error"], "invalid_redirect_uri");
    }
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn test_oauth_register_and_metadata() {
    let server = TestServer::new().await;
    // Test dynamic client registration
    let (client_id, client_secret) = register_oauth_client(&server).await;
    assert!(!client_id.is_empty());
    assert!(!client_secret.is_empty());

    // Test server metadata endpoint
    let metadata: Value = server
        .get("/.well-known/oauth-authorization-server")
        .await
        .assert_success()
        .json();
    assert!(
        metadata["authorization_endpoint"]
            .as_str()
            .unwrap()
            .ends_with("/oauth/authorize")
    );
    assert!(
        metadata["token_endpoint"]
            .as_str()
            .unwrap()
            .ends_with("/oauth/token")
    );

    // Test path-specific protected resource metadata endpoint (RFC 9728 §3.1).
    let resource: Value = server
        .get("/.well-known/oauth-protected-resource/mcp")
        .await
        .assert_success()
        .json();
    assert!(resource["resource"].as_str().unwrap().ends_with("/mcp"));

    // Root PRM URL is no longer served — clients must use the path-derived URL.
    server
        .get("/.well-known/oauth-protected-resource")
        .await
        .assert_status(StatusCode::NOT_FOUND);
}
