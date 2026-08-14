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
use everruns_core::capability_types::VirtualFileTree;
use everruns_core::{DEFAULT_ORG_ID, DEFAULT_ORG_PUBLIC_ID};
use everruns_provider::typed_id::SessionId;
use everruns_server::storage::models::{CreateOAuthAuthorizationCodeRow, CreateUserRow};
use serde_json::{Value, json};
use sha2::{Digest, Sha256};
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

#[tokio::test]
async fn test_mcp_is_available_when_obsolete_org_flag_is_disabled() {
    let server = TestServer::in_memory().await;
    let disabled_flags = std::collections::HashMap::from([("mcp_endpoint".to_string(), false)]);
    server
        .db
        .replace_org_feature_flags(everruns_core::DEFAULT_ORG_ID, &disabled_flags)
        .await
        .expect("seed obsolete MCP endpoint flag row");

    let resp = mcp_request_raw(&server, "initialize", json!({}), vec![]).await;
    assert_eq!(resp.status(), StatusCode::OK);
}

#[tokio::test]
async fn test_disabled_feature_is_hidden_from_api_platform_and_mcp_catalog() {
    let server = TestServer::in_memory().await;
    let enabled_flags = std::collections::HashMap::from([
        ("evals".to_string(), false),
        ("skills".to_string(), false),
        ("memory".to_string(), false),
        ("knowledge".to_string(), false),
        ("plugins".to_string(), false),
        ("agent_delegation".to_string(), false),
    ]);
    server
        .db
        .replace_org_feature_flags(everruns_core::DEFAULT_ORG_ID, &enabled_flags)
        .await
        .expect("disable gated surfaces while leaving MCP available");

    let api_body: Value = server
        .get("/v1/evals")
        .await
        .assert_status(StatusCode::NOT_FOUND)
        .json();
    assert_eq!(api_body["detail"], "Feature 'evals' is not enabled");
    assert_eq!(api_body["code"], "feature_not_enabled");

    for (path, flag) in [
        ("/v1/skills", "skills"),
        ("/v1/skills/config", "skills"),
        ("/v1/memories", "memory"),
        ("/v1/memories/memory_missing/fs", "memory"),
        ("/v1/knowledge-indexes", "knowledge"),
        ("/v1/knowledge-bases", "knowledge"),
        ("/v1/plugins", "plugins"),
    ] {
        let response = server.get(path).await;
        assert_eq!(
            response.status(),
            StatusCode::NOT_FOUND,
            "unexpected response for {path}: {}",
            response.text()
        );
        assert!(!response.text().is_empty(), "empty response for {path}");
        let body: Value = response.json();
        assert_eq!(body["detail"], format!("Feature '{flag}' is not enabled"));
        assert_eq!(body["code"], "feature_not_enabled");
    }

    let capabilities: Value = server
        .get("/v1/capabilities?limit=200")
        .await
        .assert_status(StatusCode::OK)
        .json();
    assert!(
        capabilities["data"]
            .as_array()
            .expect("capability list")
            .iter()
            .all(|capability| capability["id"] != "agent_handoff"
                && capability["id"] != "a2a_agent_delegation")
    );
    let capability_error: Value = server
        .get("/v1/capabilities/agent_handoff")
        .await
        .assert_status(StatusCode::NOT_FOUND)
        .json();
    assert_eq!(
        capability_error["detail"],
        "Feature 'agent_delegation' is not enabled"
    );

    let discovered = mcp_tool_call(
        &server,
        "discover",
        json!({ "all": true, "include_schemas": false }),
    )
    .await;
    let discovery: Value = serde_json::from_str(&tool_text(&discovered)).expect("discovery JSON");
    assert!(
        discovery["categories"]
            .as_array()
            .expect("categories")
            .iter()
            .all(|category| !matches!(
                category["category"].as_str(),
                Some(
                    "evals"
                        | "skills"
                        | "memories"
                        | "knowledge_indexes"
                        | "knowledge_bases"
                        | "plugins"
                )
            ))
    );

    let invoked = mcp_tool_call(&server, "query", json!({ "commands": "list_evals" })).await;
    assert!(tool_is_error(&invoked));
    assert!(
        tool_text(&invoked).contains("Feature 'evals' is not enabled"),
        "expected feature-gate failure: {}",
        tool_text(&invoked)
    );
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

/// Call a tool while negotiating a protocol version that exposes the richer
/// tool catalog (card tools like `agent_get_card` are only registered for
/// 2025-06-18+; the default fallback protocol omits them).
async fn mcp_tool_call_latest(server: &TestServer, tool: &str, arguments: Value) -> Value {
    mcp_tool_call_with_headers(
        server,
        tool,
        arguments,
        vec![("MCP-Protocol-Version", MCP_PROTOCOL_VERSION_LATEST)],
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

/// Extract the plain-text summary from a card tool result. Card tools return a
/// content array of `[{type:"resource", ...}, {type:"text", text: summary}]`
/// (see `card_tool_content`), so the summary lives on the first `text` item,
/// not at `content[0]`.
fn card_summary_text(resp: &Value) -> String {
    resp["result"]["content"]
        .as_array()
        .and_then(|items| {
            items
                .iter()
                .find(|item| item["type"] == "text")
                .and_then(|item| item["text"].as_str())
        })
        .unwrap_or("")
        .to_string()
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

fn hash_value(value: &str) -> String {
    hex::encode(Sha256::digest(value.as_bytes()))
}

fn pkce_challenge(verifier: &str) -> String {
    use base64::Engine;
    base64::engine::general_purpose::URL_SAFE_NO_PAD.encode(Sha256::digest(verifier.as_bytes()))
}

fn jwt_lifetime_secs(token: &str) -> i64 {
    use base64::Engine;
    let payload = token.split('.').nth(1).expect("JWT must have payload");
    let decoded = base64::engine::general_purpose::URL_SAFE_NO_PAD
        .decode(payload)
        .expect("JWT payload must be base64url");
    let claims: Value = serde_json::from_slice(&decoded).expect("JWT payload must be JSON");
    claims["exp"].as_i64().expect("JWT must contain exp")
        - claims["iat"].as_i64().expect("JWT must contain iat")
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
    // Card tools (knowledge/ui/mcp-cards.md) only exposed under 2025-06-18.
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
    let operation_properties =
        &discover["outputSchema"]["properties"]["operations"]["items"]["properties"];
    assert!(operation_properties["method"].is_null());
    assert!(operation_properties["path"].is_null());
    let operation_required =
        discover["outputSchema"]["properties"]["operations"]["items"]["required"]
            .as_array()
            .expect("discover operations required must be array");
    assert!(
        !operation_required.iter().any(|field| field == "method"),
        "discover output schema should not require method"
    );
    assert!(
        !operation_required.iter().any(|field| field == "path"),
        "discover output schema should not require path"
    );
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
    // under the fallback protocol. See knowledge/ui/mcp-cards.md.
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

/// EVE-492: unknown-tool errors carry the typed `structuredContent`
/// envelope alongside the legacy `content[0].text`. The legacy text path
/// stays the same for back-compat; new SDKs branch on the structured
/// envelope's machine-readable `code` / `category` / `retryable`.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn test_mcp_tools_call_unknown_tool_emits_structured_envelope() {
    let server = TestServer::new().await;
    let resp = mcp_tool_call(&server, "nonexistent_tool", json!({})).await;

    assert!(tool_is_error(&resp));
    let envelope = &resp["result"]["structuredContent"];
    assert!(
        envelope.is_object(),
        "expected structuredContent envelope on error response, got: {resp}"
    );
    assert_eq!(envelope["code"], "tool_not_found");
    assert_eq!(envelope["category"], "permanent");
    assert_eq!(envelope["retryable"], false);
    assert!(
        envelope["message"]
            .as_str()
            .unwrap_or("")
            .contains("Unknown tool"),
        "envelope.message should mirror the legacy text content: {envelope}"
    );
    // Hint must be populated for tool_not_found so the agent has a
    // concrete recovery action without parsing prose.
    assert!(
        envelope["hint"]
            .as_str()
            .unwrap_or("")
            .contains("tools/list"),
        "tool_not_found envelope should hint at tools/list: {envelope}"
    );
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

// Regression test for the `agent_get_card` session-count bug: the count must
// be keyed by the agent's internal id, not its public id. Public and internal
// ids are distinct UUIDs (see `crates/server/src/storage/repositories/agents.rs`
// where `id` and `public_id` are populated independently), and
// `sessions.agent_id` references `agents.id`. If the count keyed by public_id
// instead, the assertion below would observe `0` sessions despite one existing.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn test_mcp_agent_get_card_counts_sessions_by_internal_id() {
    let server = TestServer::new().await;

    // Create an agent via REST (auto-generated public_id != internal_id).
    let agent: Value = server
        .post(
            "/v1/agents",
            json!({
                "name": format!("card-count-agent-{}", unique_suffix()),
                "display_name": "Card Count Agent",
                "system_prompt": "Test"
            }),
        )
        .await
        .assert_success()
        .json();
    let agent_public_id = agent["id"].as_str().unwrap().to_string();

    // Baseline: no sessions yet, card should report 0.
    let resp = mcp_tool_call_latest(
        &server,
        "agent_get_card",
        json!({ "agent_id": agent_public_id }),
    )
    .await;
    assert!(
        !tool_is_error(&resp),
        "agent_get_card failed: {}",
        tool_text(&resp)
    );
    assert!(
        card_summary_text(&resp).contains("0 session(s)"),
        "expected 0 sessions in summary, got: {}",
        card_summary_text(&resp)
    );

    // Create a session bound to this agent (REST resolves public_id ->
    // internal_id and stores the internal id in `sessions.agent_id`).
    let _session: Value = server
        .post(
            "/v1/sessions",
            json!({
                "harness_id": server.seed_base_harness_id,
                "agent_id": agent_public_id,
            }),
        )
        .await
        .assert_success()
        .json();

    // Now the card must reflect 1 session. With the old (buggy) code that
    // counted by public_id, this would still be 0.
    let resp = mcp_tool_call_latest(
        &server,
        "agent_get_card",
        json!({ "agent_id": agent_public_id }),
    )
    .await;
    assert!(
        !tool_is_error(&resp),
        "agent_get_card failed: {}",
        tool_text(&resp)
    );
    assert!(
        card_summary_text(&resp).contains("1 session(s)"),
        "expected 1 session in summary, got: {}",
        card_summary_text(&resp)
    );
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
    let resp = mcp_tool_call(&server, "discover", json!({ "query": "list_agents" })).await;

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
    let list_agents_object = list_agents.as_object().expect("operation object");
    assert!(
        !list_agents_object.contains_key("method"),
        "discover should not expose HTTP methods"
    );
    assert!(
        !list_agents_object.contains_key("path"),
        "discover should not expose HTTP paths"
    );
    assert_eq!(list_agents["output_shape"], "paginated");
    assert!(
        list_agents["output_fields"]
            .as_array()
            .is_some_and(|fields| fields.iter().any(|field| field == "data")),
        "exact discovery should expose compact jq field paths"
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
    let resp = mcp_tool_call(&server, "discover", json!({ "query": "list_harnesses" })).await;

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
    assert!(
        list_harnesses["output_fields"]
            .as_array()
            .is_some_and(|fields| fields.iter().any(|field| field == "id")),
        "exact discovery should expose compact jq field paths"
    );
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
    assert!(
        !first_operation.contains_key("method"),
        "discover --all should not expose HTTP methods"
    );
    assert!(
        !first_operation.contains_key("path"),
        "discover --all should not expose HTTP paths"
    );
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn test_mcp_discover_session_database_schema_omits_http_metadata() {
    let server = TestServer::in_memory().await;
    let resp = mcp_tool_call(
        &server,
        "discover",
        json!({ "query": "session database schema" }),
    )
    .await;

    assert!(
        !tool_is_error(&resp),
        "discover failed: {}",
        tool_text(&resp)
    );
    let payload = tool_json(&resp);
    let operation = payload["operations"]
        .as_array()
        .unwrap()
        .iter()
        .find(|op| op["name"] == "get_session_database_schema")
        .expect("get_session_database_schema operation");
    let operation = operation.as_object().expect("operation object");
    assert!(
        !operation.contains_key("method"),
        "discover should not expose HTTP methods"
    );
    assert!(
        !operation.contains_key("path"),
        "discover should not expose HTTP paths"
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
    assert_eq!(payload["result_scope"], "operation_catalog");
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

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn test_mcp_discover_resource_phrase_ranks_read_paths() {
    let server = TestServer::in_memory().await;
    let resp = mcp_tool_call(
        &server,
        "discover",
        json!({ "query": "resend email capability plugin", "include_schemas": false }),
    )
    .await;
    assert!(
        !tool_is_error(&resp),
        "discover failed: {}",
        tool_text(&resp)
    );
    let payload = tool_json(&resp);
    let names = payload["operations"]
        .as_array()
        .expect("operations")
        .iter()
        .filter_map(|operation| operation["name"].as_str())
        .collect::<Vec<_>>();
    for expected in ["list_plugins", "list_capabilities"] {
        assert!(names.contains(&expected), "missing {expected}: {names:?}");
    }
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
async fn test_mcp_query_reporting_catalog_and_model_usage() {
    let server = TestServer::in_memory().await;

    let catalog = mcp_tool_call(
        &server,
        "query",
        json!({ "commands": "get_report_catalog" }),
    )
    .await;
    assert!(
        !tool_is_error(&catalog),
        "query get_report_catalog failed: {}",
        tool_text(&catalog)
    );
    assert!(
        tool_json(&catalog)["datasets"]
            .as_array()
            .expect("report datasets")
            .iter()
            .any(|dataset| dataset["name"] == "llm_generations")
    );

    let report = mcp_tool_call(
        &server,
        "query",
        json!({
            "commands": "run_report_query --dataset llm_generations --time_range '{\"from\":\"2026-01-01T00:00:00Z\",\"to\":\"2026-08-09T00:00:00Z\"}' --dimensions '[\"provider\",\"model\"]' --measures '[\"input_tokens\",\"output_tokens\"]'"
        }),
    )
    .await;
    assert!(
        !tool_is_error(&report),
        "query run_report_query failed: {}",
        tool_text(&report)
    );
    let result = tool_json(&report);
    assert_eq!(result["columns"][0]["name"], "provider");
    assert_eq!(result["columns"][1]["name"], "model");
    assert_eq!(result["columns"][2]["name"], "input_tokens");
    assert_eq!(result["columns"][3]["name"], "output_tokens");
    assert!(result["rows"].is_array());

    let invalid = mcp_tool_call(
        &server,
        "query",
        json!({
            "commands": "run_report_query --dataset llm_generations --time_range '{\"from\":\"2026-01-01T00:00:00Z\",\"to\":\"2026-08-09T00:00:00Z\"}' --dimensions '[\"not_a_dimension\"]' --measures '[\"input_tokens\"]'"
        }),
    )
    .await;
    assert!(tool_is_error(&invalid));
    assert!(tool_text(&invalid).contains("bad_request:"));
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn test_mcp_query_exposes_connection_provider_reads() {
    let server = TestServer::in_memory().await;
    let resp = mcp_tool_call(
        &server,
        "query",
        json!({ "commands": "list_connection_providers" }),
    )
    .await;
    assert!(!tool_is_error(&resp), "query failed: {}", tool_text(&resp));
    assert!(tool_json(&resp).is_array(), "provider read must be bounded");

    // The shared catalog contains the user-scoped read, but organization API
    // keys intentionally cannot execute it because they have no user identity.
    let discover = mcp_tool_call(
        &server,
        "discover",
        json!({ "query": "list_user_connections" }),
    )
    .await;
    assert_eq!(
        tool_json(&discover)["operations"][0]["name"],
        "list_user_connections"
    );
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
async fn test_mcp_query_allows_empty_jq_result() {
    let server = TestServer::in_memory().await;
    let resp = mcp_tool_call(
        &server,
        "query",
        json!({
            "commands": "list_harnesses --limit 100 | jq -c '.data[] | {id, name, system_prompt}'"
        }),
    )
    .await;

    assert!(!tool_is_error(&resp), "empty jq output is valid");
    assert_eq!(tool_text(&resp), "");
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn test_mcp_query_rejects_preview_commands_with_network_side_effects() {
    // EVE / TM-MCP-002: even though `preview_agent` and `preview_harness`
    // are flagged `read_only()`, they perform scoped MCP discovery which
    // makes outbound HTTP requests. They must not be exposed under the
    // read-only `query` tool, which is the no-side-effects entry point.
    let server = TestServer::in_memory().await;
    let resp = mcp_tool_call(
        &server,
        "query",
        json!({ "commands": "preview_agent --system_prompt 'Preview via query'" }),
    )
    .await;
    assert!(
        tool_is_error(&resp),
        "query preview_agent unexpectedly succeeded: {}",
        tool_text(&resp)
    );

    let resp_harness = mcp_tool_call(
        &server,
        "query",
        json!({ "commands": "preview_harness --system_prompt 'Preview via query'" }),
    )
    .await;
    assert!(
        tool_is_error(&resp_harness),
        "query preview_harness unexpectedly succeeded: {}",
        tool_text(&resp_harness)
    );
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

    // `list_harnesses` dispatches the `ListHarnesses` command whose output is a
    // bare `Vec<Harness>`, so the tool result is a JSON array (not a
    // `{data: [...]}` envelope).
    let result = tool_json(&resp);
    let harnesses = result.as_array().expect("Expected harnesses array");
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

    // Session files were renamed to workspace files; the read tool is now
    // `get_workspace_file` (still keyed by session id + path).
    let resp = mcp_tool_call(
        &server,
        "execute",
        json!({ "commands": format!("get_workspace_file --session_id {} --path '/docs/readme.md'", session_id) }),
    )
    .await;

    assert!(
        !tool_is_error(&resp),
        "execute get_workspace_file failed: {}",
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
async fn test_mcp_execute_app_commands_reject_schedule_and_support_webhook_channels() {
    let server = TestServer::in_memory().await;
    let app_name = format!("repo-checker-{}", unique_suffix());

    let create_agent_resp = mcp_tool_call(
        &server,
        "execute",
        json!({
            "commands": format!(
                "create_agent --name '{app_name}-agent' --display_name 'Repo Checker Agent' --system_prompt 'Test prompt'"
            )
        }),
    )
    .await;
    assert!(
        !tool_is_error(&create_agent_resp),
        "create_agent failed: {}",
        tool_text(&create_agent_resp)
    );
    let agent_id = tool_json(&create_agent_resp)["id"]
        .as_str()
        .expect("agent id")
        .to_string();

    let create_resp = mcp_tool_call(
        &server,
        "execute",
        json!({
            "commands": format!(
                "create_app --name '{app_name}' --description 'repo checks' --harness_id {} --agent_id {agent_id}",
                server.seed_generic_harness_id,
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
                "add_schedule_app_channel --app_id {app_id} --cron_expression '*/10 * * * *' --timezone UTC --session_mode shared_session --message 'run checks'"
            )
        }),
    )
    .await;
    assert!(
        tool_is_error(&add_schedule_resp),
        "add_schedule_app_channel unexpectedly succeeded: {}",
        tool_text(&add_schedule_resp)
    );
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

    let list_channels_resp = mcp_tool_call(
        &server,
        "execute",
        json!({ "commands": format!("list_app_channels {app_id}") }),
    )
    .await;
    assert!(
        !tool_is_error(&list_channels_resp),
        "list_app_channels failed: {}",
        tool_text(&list_channels_resp)
    );
    let channels = tool_json(&list_channels_resp);
    let serialized_channels = serde_json::to_string(&channels).unwrap();
    assert!(
        !serialized_channels.contains("secret-1"),
        "channel listing leaked webhook token: {serialized_channels}"
    );
    assert!(
        channels
            .as_array()
            .unwrap()
            .iter()
            .any(|channel| channel["channel_config"]["token_configured"] == true)
    );

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
    assert!(!channel_types.contains(&"schedule"));
    assert!(channel_types.contains(&"webhook"));
    let serialized_app = serde_json::to_string(&fetched_app).unwrap();
    assert!(
        !serialized_app.contains("secret-1"),
        "get_app leaked webhook token: {serialized_app}"
    );

    let publish_resp = mcp_tool_call(
        &server,
        "execute",
        json!({ "commands": format!("publish_app {app_id}") }),
    )
    .await;
    assert!(
        !tool_is_error(&publish_resp),
        "publish_app failed: {}",
        tool_text(&publish_resp)
    );
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

    // Boolean flags in the execute CLI are presence flags (bashkit sets the
    // flag to `true` when present and does not consume a following value
    // token), so request archived agents with bare `--include_archived` rather
    // than `--include_archived true` (which would leave `true` as a stray arg).
    let resp = execute_http_command(
        &url,
        format!("list_agents --include_archived --search {archived_agent_name} --limit 10"),
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
async fn test_mcp_org_override_is_available_without_target_org_opt_in() {
    let server = TestServer::in_memory().await;

    let org2: Value = server
        .post(
            "/v1/orgs",
            json!({ "name": format!("MCP Disabled Override Target {}", unique_suffix()) }),
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

    let direct_org2_resp = mcp_request_raw(
        &server,
        "initialize",
        json!({}),
        vec![("cookie", org2_cookie.as_str())],
    )
    .await;
    assert_eq!(direct_org2_resp.status(), StatusCode::OK);

    let override_resp = mcp_tool_call(
        &server,
        "execute",
        json!({
            "organization_id": org2_id,
            "commands": "list_agents --limit 5",
        }),
    )
    .await;
    assert!(
        !tool_is_error(&override_resp),
        "target org override failed without feature opt-in: {}",
        tool_text(&override_resp)
    );
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
    assert!(discover_agent_text.contains("list_agents"));
    assert!(discover_agent_text.contains("get_agent"));

    // Broad entity discovery is intentionally bounded and read-first. Resolve
    // the mutation by exact name before exercising its tenant boundary below.
    let discover_update_agent = mcp_tool_call(
        &server,
        "discover",
        json!({ "query": "update_agent", "include_schemas": true }),
    )
    .await;
    assert!(
        !tool_is_error(&discover_update_agent),
        "discover update_agent failed: {}",
        tool_text(&discover_update_agent)
    );
    assert!(tool_text(&discover_update_agent).contains("update_agent"));

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
    assert!(discover_session_text.contains("list_sessions"));
    assert!(discover_session_text.contains("get_session"));

    let discover_create_message = mcp_tool_call(
        &server,
        "discover",
        json!({ "query": "create_message", "include_schemas": true }),
    )
    .await;
    assert!(
        !tool_is_error(&discover_create_message),
        "discover create_message failed: {}",
        tool_text(&discover_create_message)
    );
    assert!(tool_text(&discover_create_message).contains("create_message"));

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

    // Use a pipe to count harnesses (tests bash pipe support in execute).
    // `list_harnesses` outputs a bare JSON array (Vec<Harness>), so count with
    // `jq length` directly rather than indexing a `.data` envelope.
    let resp = mcp_tool_call_http(
        &url,
        "execute",
        json!({ "commands": "list_harnesses | jq 'length'" }),
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

    // `list_orgs` lists the *authenticated user's* org memberships, so it
    // requires a real user. The no-auth MCP path runs as the anonymous caller
    // (no `user_id`, see `resolve_org_for_user`'s `AuthMethod::None` branch), so
    // the command is correctly Forbidden rather than returning a result.
    let resp = mcp_tool_call_http(&url, "execute", json!({ "commands": "list_orgs" })).await;
    assert!(
        tool_is_error(&resp),
        "list_orgs should be forbidden for the anonymous MCP caller, got: {}",
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

    // `discover --all` writes the tool catalog to stdout as plain text (one
    // `name  description` line per command), so assert against the text result
    // directly rather than a JSON `{success, stdout}` envelope.
    let stdout = tool_text(&resp);
    assert!(stdout.contains("agents"), "Should list agents category");
    assert!(stdout.contains("sessions"), "Should list sessions category");
    assert!(
        !stdout.contains("list_schedules"),
        "durable schedule tools should not be exposed through MCP execute"
    );
    assert!(
        !stdout.contains("/v1/durable/"),
        "durable control-plane tools should not be exposed through MCP execute"
    );
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

async fn create_oauth_test_user(server: &TestServer) -> uuid::Uuid {
    let suffix = unique_suffix();
    let user = server
        .db
        .create_user(CreateUserRow {
            email: format!("mcp-oauth-{suffix}@example.com"),
            name: "MCP OAuth Test User".to_string(),
            avatar_url: None,
            roles: vec!["user".to_string()],
            password_hash: None,
            email_verified: true,
            auth_provider: None,
            auth_provider_id: None,
            external_id: None,
        })
        .await
        .expect("failed to create OAuth test user");
    server
        .db
        .add_organization_member(DEFAULT_ORG_ID, user.id, "member")
        .await
        .expect("failed to add OAuth test user to default org");
    user.id
}

async fn issue_mcp_oauth_token_pair(
    server: &TestServer,
    client_id: &str,
    client_secret: &str,
) -> Value {
    let user_id = create_oauth_test_user(server).await;
    let suffix = unique_suffix();
    let code = format!("oauth-code-{suffix}");
    let verifier = format!("oauth-verifier-{suffix}");
    let redirect_uri = "http://localhost:9999/callback";

    server
        .db
        .create_oauth_authorization_code(CreateOAuthAuthorizationCodeRow {
            code_hash: hash_value(&code),
            client_id: client_id.to_string(),
            user_id,
            org_id: DEFAULT_ORG_ID,
            redirect_uri: redirect_uri.to_string(),
            code_challenge: pkce_challenge(&verifier),
            code_challenge_method: "S256".to_string(),
            scope: "mcp".to_string(),
            expires_at: chrono::Utc::now() + chrono::Duration::minutes(5),
        })
        .await
        .expect("failed to create OAuth authorization code");

    server
        .post(
            "/oauth/token",
            json!({
                "grant_type": "authorization_code",
                "code": code,
                "client_id": client_id,
                "client_secret": client_secret,
                "redirect_uri": redirect_uri,
                "code_verifier": verifier,
            }),
        )
        .await
        .assert_success()
        .json()
}

async fn refresh_mcp_oauth_token(
    server: &TestServer,
    client_id: &str,
    client_secret: &str,
    refresh_token: &str,
) -> test_harness::TestResponse {
    server
        .post(
            "/oauth/token",
            json!({
                "grant_type": "refresh_token",
                "refresh_token": refresh_token,
                "client_id": client_id,
                "client_secret": client_secret,
            }),
        )
        .await
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
async fn test_oauth_token_expires_in_matches_jwt_lifetime() {
    let server = TestServer::in_memory().await;
    let (client_id, client_secret) = register_oauth_client(&server).await;
    let token: Value = issue_mcp_oauth_token_pair(&server, &client_id, &client_secret).await;

    let access_token = token["access_token"]
        .as_str()
        .expect("access_token must be present");
    let expires_in = token["expires_in"]
        .as_i64()
        .expect("expires_in must be present");

    assert_eq!(expires_in, jwt_lifetime_secs(access_token));
}

/// Decode a JWT payload to a JSON object without verifying the signature.
fn jwt_claims(token: &str) -> Value {
    use base64::Engine;
    let payload = token.split('.').nth(1).expect("JWT must have payload");
    let decoded = base64::engine::general_purpose::URL_SAFE_NO_PAD
        .decode(payload)
        .expect("JWT payload must be base64url");
    serde_json::from_slice(&decoded).expect("JWT payload must be JSON")
}

/// TM-MCP-006: the MCP OAuth flow must mint a resource-bound `mcp_access`
/// token (distinguishing claim + `aud` audience) rather than a full-API access
/// token. Proves claim (c) end-to-end through the real authorization_code grant.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn test_oauth_token_is_mcp_scoped_with_audience() {
    let server = TestServer::in_memory().await;
    let (client_id, client_secret) = register_oauth_client(&server).await;
    let token: Value = issue_mcp_oauth_token_pair(&server, &client_id, &client_secret).await;
    let access_token = token["access_token"]
        .as_str()
        .expect("access_token present");

    let claims = jwt_claims(access_token);
    assert_eq!(
        claims["token_type"], "mcp_access",
        "MCP OAuth token must carry the distinguishing token_type"
    );
    let aud = claims["aud"].as_str().expect("MCP token must carry aud");
    assert!(
        aud.ends_with("/mcp"),
        "MCP token aud must be the /mcp resource, got {aud}"
    );

    // Same audience binding must hold after refresh-token rotation.
    let refreshed: Value = refresh_mcp_oauth_token(
        &server,
        &client_id,
        &client_secret,
        token["refresh_token"].as_str().unwrap(),
    )
    .await
    .assert_success()
    .json();
    let refreshed_claims = jwt_claims(refreshed["access_token"].as_str().unwrap());
    assert_eq!(refreshed_claims["token_type"], "mcp_access");
    assert!(
        refreshed_claims["aud"].as_str().unwrap().ends_with("/mcp"),
        "refreshed MCP token must remain bound to /mcp"
    );
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn test_oauth_refresh_token_rotates_and_rejects_reuse() {
    let server = TestServer::in_memory().await;
    let (client_id, client_secret) = register_oauth_client(&server).await;
    let initial: Value = issue_mcp_oauth_token_pair(&server, &client_id, &client_secret).await;
    let old_refresh = initial["refresh_token"]
        .as_str()
        .expect("initial refresh_token must be present")
        .to_string();

    let refreshed: Value =
        refresh_mcp_oauth_token(&server, &client_id, &client_secret, &old_refresh)
            .await
            .assert_success()
            .json();
    let new_refresh = refreshed["refresh_token"]
        .as_str()
        .expect("refreshed refresh_token must be present")
        .to_string();
    assert_ne!(new_refresh, old_refresh);
    assert_eq!(
        refreshed["expires_in"].as_i64().unwrap(),
        jwt_lifetime_secs(refreshed["access_token"].as_str().unwrap())
    );

    let reuse = refresh_mcp_oauth_token(&server, &client_id, &client_secret, &old_refresh).await;
    assert_eq!(reuse.status(), StatusCode::BAD_REQUEST);
    let body: Value = reuse.json();
    assert_eq!(body["error"], "invalid_grant");

    refresh_mcp_oauth_token(&server, &client_id, &client_secret, &new_refresh)
        .await
        .assert_success();
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn test_oauth_refresh_token_concurrent_retries_are_single_use() {
    let server = TestServer::in_memory().await;
    let (client_id, client_secret) = register_oauth_client(&server).await;
    let initial: Value = issue_mcp_oauth_token_pair(&server, &client_id, &client_secret).await;
    let refresh = initial["refresh_token"]
        .as_str()
        .expect("initial refresh_token must be present")
        .to_string();

    let (first, second) = tokio::join!(
        refresh_mcp_oauth_token(&server, &client_id, &client_secret, &refresh),
        refresh_mcp_oauth_token(&server, &client_id, &client_secret, &refresh),
    );

    let statuses = [first.status(), second.status()];
    assert_eq!(
        statuses.iter().filter(|status| status.is_success()).count(),
        1,
        "exactly one concurrent refresh should succeed: {statuses:?}",
    );
    assert_eq!(
        statuses
            .iter()
            .filter(|status| **status == StatusCode::BAD_REQUEST)
            .count(),
        1,
        "exactly one concurrent refresh should fail as invalid_grant: {statuses:?}",
    );

    let failed = if first.status().is_success() {
        second
    } else {
        first
    };
    let body: Value = failed.json();
    assert_eq!(body["error"], "invalid_grant");
}

/// Helper: create a single authorization code and return the params needed to
/// redeem it at `/oauth/token` (so a test can redeem the *same* code twice).
async fn create_oauth_authorization_code_params(
    server: &TestServer,
    client_id: &str,
) -> (String, String, String) {
    let user_id = create_oauth_test_user(server).await;
    let suffix = unique_suffix();
    let code = format!("oauth-code-{suffix}");
    let verifier = format!("oauth-verifier-{suffix}");
    let redirect_uri = "http://localhost:9999/callback".to_string();

    server
        .db
        .create_oauth_authorization_code(CreateOAuthAuthorizationCodeRow {
            code_hash: hash_value(&code),
            client_id: client_id.to_string(),
            user_id,
            org_id: DEFAULT_ORG_ID,
            redirect_uri: redirect_uri.clone(),
            code_challenge: pkce_challenge(&verifier),
            code_challenge_method: "S256".to_string(),
            scope: "mcp".to_string(),
            expires_at: chrono::Utc::now() + chrono::Duration::minutes(5),
        })
        .await
        .expect("failed to create OAuth authorization code");

    (code, verifier, redirect_uri)
}

async fn redeem_oauth_authorization_code(
    server: &TestServer,
    client_id: &str,
    client_secret: &str,
    code: &str,
    verifier: &str,
    redirect_uri: &str,
) -> test_harness::TestResponse {
    server
        .post(
            "/oauth/token",
            json!({
                "grant_type": "authorization_code",
                "code": code,
                "client_id": client_id,
                "client_secret": client_secret,
                "redirect_uri": redirect_uri,
                "code_verifier": verifier,
            }),
        )
        .await
}

/// EVE-622: a leaked/intercepted authorization code must be redeemable at most
/// once. The token endpoint reads `consumed` before PKCE validation, then relies
/// on the atomic consume to reject replays.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn test_oauth_authorization_code_rejects_reuse() {
    let server = TestServer::in_memory().await;
    let (client_id, client_secret) = register_oauth_client(&server).await;
    let (code, verifier, redirect_uri) =
        create_oauth_authorization_code_params(&server, &client_id).await;

    redeem_oauth_authorization_code(
        &server,
        &client_id,
        &client_secret,
        &code,
        &verifier,
        &redirect_uri,
    )
    .await
    .assert_success();

    let reuse = redeem_oauth_authorization_code(
        &server,
        &client_id,
        &client_secret,
        &code,
        &verifier,
        &redirect_uri,
    )
    .await;
    assert_eq!(reuse.status(), StatusCode::BAD_REQUEST);
    let body: Value = reuse.json();
    assert_eq!(body["error"], "invalid_grant");
}

/// EVE-622: two concurrent redemptions of the same code (the TOCTOU window) must
/// resolve to exactly one success — without the atomic-consume check both would
/// pass the stale `consumed` read and mint tokens.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn test_oauth_authorization_code_concurrent_retries_are_single_use() {
    let server = TestServer::in_memory().await;
    let (client_id, client_secret) = register_oauth_client(&server).await;
    let (code, verifier, redirect_uri) =
        create_oauth_authorization_code_params(&server, &client_id).await;

    let (first, second) = tokio::join!(
        redeem_oauth_authorization_code(
            &server,
            &client_id,
            &client_secret,
            &code,
            &verifier,
            &redirect_uri,
        ),
        redeem_oauth_authorization_code(
            &server,
            &client_id,
            &client_secret,
            &code,
            &verifier,
            &redirect_uri,
        ),
    );

    let statuses = [first.status(), second.status()];
    assert_eq!(
        statuses.iter().filter(|status| status.is_success()).count(),
        1,
        "exactly one concurrent authorization_code redemption should succeed: {statuses:?}",
    );

    let failed = if first.status().is_success() {
        second
    } else {
        first
    };
    assert_eq!(failed.status(), StatusCode::BAD_REQUEST);
    let body: Value = failed.json();
    assert_eq!(body["error"], "invalid_grant");
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
    // Use a fresh server per URI: the registration endpoint is now per-IP
    // rate-limited (TM-DOS), and this loop has more entries than the register
    // limit, so sharing one server would trip the limiter before all URIs are
    // exercised.
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
        let server = TestServer::in_memory().await;
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

/// Build a `/oauth/authorize` query string for `client_id` with fixed,
/// well-formed PKCE/state values (PKCE is only verified at the token step).
fn authorize_query_string(client_id: &str) -> String {
    format!(
        "/oauth/authorize?client_id={client_id}\
         &redirect_uri=http://localhost:9999/callback\
         &response_type=code\
         &code_challenge=abc123challenge\
         &code_challenge_method=S256\
         &state=xyz-state\
         &scope=mcp"
    )
}

/// Pull the hidden `csrf_token` (the signed consent token) out of the rendered
/// confirmation page.
fn extract_csrf_token(html: &str) -> String {
    let needle = r#"name="csrf_token" value=""#;
    let start = html
        .find(needle)
        .expect("confirm page must embed a csrf_token field")
        + needle.len();
    let end = html[start..].find('"').expect("csrf_token must be closed");
    html[start..start + end].to_string()
}

/// The browser-facing consent step must NOT depend on a second cookie set by
/// the GET round-tripping to the POST: in real MCP-client browser contexts that
/// cookie is not reliably stored even though the session cookie is, which
/// surfaced as a spurious "Missing CSRF cookie" 401. This drives the full
/// GET -> POST confirm flow without threading any cookie and asserts an
/// authorization code is issued. (In test/`AuthMode::None` the session is the
/// anonymous user, so the consent token's `sub` binding still applies.)
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn test_oauth_authorize_confirm_needs_no_csrf_cookie() {
    let server = TestServer::in_memory().await;
    let (client_id, _) = register_oauth_client(&server).await;

    // GET the confirmation page. No cookie is set/threaded by the test client.
    let page = server.get(&authorize_query_string(&client_id)).await;
    assert_eq!(page.status(), StatusCode::OK);
    let csrf_token = extract_csrf_token(&page.text());

    // POST the consent form back with the embedded token but no cookies.
    let body = format!(
        "client_id={client_id}\
         &redirect_uri=http://localhost:9999/callback\
         &response_type=code\
         &code_challenge=abc123challenge\
         &code_challenge_method=S256\
         &state=xyz-state\
         &scope=mcp\
         &csrf_token={csrf_token}"
    );
    let resp = server
        .request_raw(
            Method::POST,
            "/oauth/authorize",
            vec![("content-type", "application/x-www-form-urlencoded")],
            body.into_bytes(),
        )
        .await;

    assert_eq!(
        resp.status(),
        StatusCode::SEE_OTHER,
        "confirm must redirect to the client callback with a code"
    );
    let location = resp
        .headers()
        .get("location")
        .and_then(|v| v.to_str().ok())
        .unwrap_or_default();
    assert!(
        location.starts_with("http://localhost:9999/callback?"),
        "unexpected redirect target: {location}"
    );
    assert!(location.contains("code="), "redirect must carry a code");
    assert!(
        location.contains("state=xyz-state"),
        "state must round-trip"
    );
}

/// The consent page must carry its own CSP whose `form-action` allows the
/// client's redirect origin. Chrome checks `form-action` against every hop of
/// the form-submission redirect chain, so the global `form-action 'self'`
/// policy silently blocks the 302 to the client callback (e.g. a native
/// client's `http://localhost:<port>/callback`) — the click appears to do
/// nothing.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn test_oauth_authorize_page_csp_allows_redirect_origin() {
    let server = TestServer::in_memory().await;
    let (client_id, _) = register_oauth_client(&server).await;

    let page = server.get(&authorize_query_string(&client_id)).await;
    assert_eq!(page.status(), StatusCode::OK);

    let csp = page
        .headers()
        .get("content-security-policy")
        .and_then(|v| v.to_str().ok())
        .expect("consent page must set its own content-security-policy");
    assert!(
        csp.contains("form-action 'self' http://localhost:9999"),
        "form-action must allow the validated redirect origin, got: {csp}"
    );
}

/// A forged/garbage consent token must be rejected — the anti-CSRF guarantee
/// must survive moving from a cookie to a signed session-bound token.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn test_oauth_authorize_confirm_rejects_invalid_consent_token() {
    let server = TestServer::in_memory().await;
    let (client_id, _) = register_oauth_client(&server).await;

    let body = format!(
        "client_id={client_id}\
         &redirect_uri=http://localhost:9999/callback\
         &response_type=code\
         &code_challenge=abc123challenge\
         &code_challenge_method=S256\
         &state=xyz-state\
         &scope=mcp\
         &csrf_token=not-a-valid-token"
    );
    let resp = server
        .request_raw(
            Method::POST,
            "/oauth/authorize",
            vec![("content-type", "application/x-www-form-urlencoded")],
            body.into_bytes(),
        )
        .await;

    assert_eq!(resp.status(), StatusCode::UNAUTHORIZED);
}

/// TM-DOS: the unauthenticated dynamic client registration endpoint must be
/// per-IP rate limited so it cannot be used to create unbounded `oauth_clients`
/// rows. The in-memory register limit is 5/min; the 6th request from the same
/// (loopback) client within the window must be rejected with HTTP 429 and an
/// OAuth-shaped error body.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn test_oauth_register_is_rate_limited() {
    let server = TestServer::in_memory().await;
    let body = json!({
        "client_name": "rl-client",
        "redirect_uris": ["http://localhost:9999/callback"],
    });

    // The first REGISTER_LIMIT (5) requests succeed.
    for _ in 0..5 {
        server
            .post("/oauth/register", body.clone())
            .await
            .assert_status(StatusCode::CREATED);
    }

    // The next request from the same IP is throttled.
    let resp = server.post("/oauth/register", body.clone()).await;
    assert_eq!(resp.status(), StatusCode::TOO_MANY_REQUESTS);
    let payload: Value = resp.json();
    assert_eq!(payload["error"], "too_many_requests");
}

// ============================================================================
// MCP 2026-07-28 Tasks extension (SEP-2663)
// ============================================================================

/// The real 2026-07-28 protocol version. The file-level `..._LATEST` const is
/// `2025-06-18` (the version that first exposed the richer tool shape); the
/// Tasks extension needs the actual negotiated 2026 version.
const MCP_PROTOCOL_VERSION_2026: &str = "2026-07-28";

/// Per-request `_meta` block that advertises the Tasks extension opt-in, as an
/// MCP 2026 client would send it.
fn tasks_opt_in_meta() -> Value {
    json!({
        "io.modelcontextprotocol/clientCapabilities": {
            "extensions": { "io.modelcontextprotocol/tasks": {} }
        }
    })
}

fn assert_final_tasks_duration_fields(result: &Value) {
    assert!(
        result.get("ttlMs").and_then(Value::as_u64).is_some(),
        "final SEP-2663 task shape must include ttlMs, got: {result}"
    );
    assert!(
        result
            .get("pollIntervalMs")
            .and_then(Value::as_u64)
            .is_some(),
        "final SEP-2663 task shape must include pollIntervalMs, got: {result}"
    );
    assert!(
        result.get("ttlSeconds").is_none(),
        "draft ttlSeconds field must not be emitted, got: {result}"
    );
    assert!(
        result.get("pollIntervalMilliseconds").is_none(),
        "draft pollIntervalMilliseconds field must not be emitted, got: {result}"
    );
}

/// tools/call with the 2026 protocol header AND the tasks opt-in `_meta`.
async fn mcp_task_tool_call(server: &TestServer, tool: &str, arguments: Value) -> Value {
    mcp_call_with_headers(
        server,
        "tools/call",
        json!({ "name": tool, "arguments": arguments, "_meta": tasks_opt_in_meta() }),
        vec![("MCP-Protocol-Version", MCP_PROTOCOL_VERSION_2026)],
    )
    .await
}

/// A 2026 `initialize` advertises the tasks extension under
/// `capabilities.extensions`.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn test_mcp_tasks_capability_advertised_for_2026() {
    let server = TestServer::in_memory().await;
    let resp = mcp_call(
        &server,
        "initialize",
        json!({ "protocolVersion": MCP_PROTOCOL_VERSION_2026 }),
    )
    .await;

    assert_eq!(resp["result"]["protocolVersion"], MCP_PROTOCOL_VERSION_2026);
    assert!(
        resp["result"]["capabilities"]["extensions"]["io.modelcontextprotocol/tasks"].is_object(),
        "2026 initialize must advertise the tasks extension, got: {}",
        resp["result"]["capabilities"]
    );
}

/// 2025-* `initialize` responses are unchanged — no `extensions` key.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn test_mcp_tasks_capability_absent_for_2025() {
    let server = TestServer::in_memory().await;
    for version in [MCP_PROTOCOL_VERSION_LATEST, MCP_PROTOCOL_VERSION_FALLBACK] {
        let resp = mcp_call(&server, "initialize", json!({ "protocolVersion": version })).await;
        assert_eq!(resp["result"]["protocolVersion"], version);
        assert!(
            resp["result"]["capabilities"].get("extensions").is_none(),
            "2025 initialize ({version}) must not advertise extensions, got: {}",
            resp["result"]["capabilities"]
        );
    }
}

/// A 2026 tasks-opted-in `agent_run` returns the CreateTaskResult task-handle
/// shape (resultType/taskId/status) alongside the existing session fields.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn test_mcp_agent_run_returns_task_handle_for_2026() {
    let server = TestServer::new().await;

    let resp =
        mcp_task_tool_call(&server, "agent_run", json!({ "message": "Hello via task" })).await;
    assert!(
        !tool_is_error(&resp),
        "agent_run failed: {}",
        tool_text(&resp)
    );

    let result = &resp["result"];
    // Additive task handle.
    assert_eq!(result["resultType"], "task");
    let task_id = result["taskId"].as_str().expect("taskId present");
    assert!(task_id.starts_with("session_"), "taskId is session id");
    assert_eq!(result["status"], "working");
    assert_final_tasks_duration_fields(result);

    // Legacy content preserved: taskId equals the session_id in the body.
    let body = tool_json(&resp);
    assert_eq!(body["session_id"].as_str(), Some(task_id));
}

/// Without the opt-in `_meta`, a 2026 `agent_run` does NOT get task fields —
/// the server must not push tasks onto a client that didn't advertise support.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn test_mcp_agent_run_no_task_handle_without_opt_in() {
    let server = TestServer::new().await;

    let resp = mcp_call_with_headers(
        &server,
        "tools/call",
        json!({ "name": "agent_run", "arguments": { "message": "Hi" } }),
        vec![("MCP-Protocol-Version", MCP_PROTOCOL_VERSION_2026)],
    )
    .await;

    // 2026 marks every result's type, so the absence of a task shows up as
    // `complete` rather than a missing field.
    assert_eq!(resp["result"]["resultType"], "complete");
    assert!(resp["result"].get("taskId").is_none());
}

/// 2025-* clients never see task fields even if they somehow send the `_meta`.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn test_mcp_agent_run_no_task_handle_for_2025() {
    let server = TestServer::new().await;

    let resp = mcp_call_with_headers(
        &server,
        "tools/call",
        json!({ "name": "agent_run", "arguments": { "message": "Hi" }, "_meta": tasks_opt_in_meta() }),
        vec![("MCP-Protocol-Version", MCP_PROTOCOL_VERSION_LATEST)],
    )
    .await;

    assert!(
        !tool_is_error(&resp),
        "agent_run failed: {}",
        tool_text(&resp)
    );
    assert!(resp["result"].get("resultType").is_none());
    assert!(resp["result"].get("taskId").is_none());
}

/// tasks/get resolves a task handle (session_id) to a Task object with a
/// lifecycle status and a result snapshot.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn test_mcp_tasks_get_round_trip() {
    let server = TestServer::new().await;

    let run = mcp_task_tool_call(&server, "agent_run", json!({ "message": "task get" })).await;
    let task_id = run["result"]["taskId"].as_str().unwrap().to_string();

    let resp = mcp_call_with_headers(
        &server,
        "tasks/get",
        json!({ "taskId": task_id, "_meta": tasks_opt_in_meta() }),
        vec![("MCP-Protocol-Version", MCP_PROTOCOL_VERSION_2026)],
    )
    .await;

    let result = &resp["result"];
    assert_eq!(result["taskId"].as_str(), Some(task_id.as_str()));
    assert_final_tasks_duration_fields(result);
    let status = result["status"].as_str().unwrap();
    assert!(
        [
            "working",
            "input_required",
            "completed",
            "failed",
            "cancelled"
        ]
        .contains(&status),
        "unexpected task status: {status}"
    );
    // The session_get_status payload is surfaced as the task result.
    assert_eq!(
        result["result"]["session_id"].as_str(),
        Some(task_id.as_str())
    );
}

/// Seed a deterministic structured result (EVE-678) on `session_id`: create a
/// `result_schema`-bound task owned by the session, point it at a result file,
/// and write that file into the session's workspace VFS.
async fn seed_structured_result(server: &TestServer, session_id: &str, result: Value) {
    use everruns_core::session_task::{
        CreateSessionTask, SessionTaskState, SessionTaskUpdate, new_session_task, task_result_path,
    };
    use everruns_server::storage::models::CreateSessionFileRow;

    let sid = session_id.parse::<SessionId>().expect("valid session id");
    let session = server
        .db
        .get_session(DEFAULT_ORG_ID, sid)
        .await
        .expect("get_session")
        .expect("session exists");

    let task = new_session_task(
        CreateSessionTask {
            session_id: sid,
            id: None,
            kind: "subagent".to_string(),
            display_name: "structured result".to_string(),
            spec: json!({ "result_schema": { "type": "object" } }),
            state: SessionTaskState::Running,
            links: Default::default(),
            wake_policy: Default::default(),
        },
        chrono::Utc::now(),
    );
    server
        .db
        .create_session_task(&task)
        .await
        .expect("create task");

    let path = task_result_path(&task.id);
    server
        .db
        .update_session_task(
            sid,
            &task.id,
            SessionTaskUpdate {
                state: Some(SessionTaskState::Succeeded),
                result_path: Some(path.clone()),
                ..Default::default()
            },
        )
        .await
        .expect("set result_path");

    server
        .db
        .create_session_file(CreateSessionFileRow {
            session_id: SessionId::from_uuid(session.workspace_id),
            path,
            content: Some(serde_json::to_vec(&result).unwrap()),
            is_directory: false,
            is_readonly: false,
        })
        .await
        .expect("write result file");
}

async fn seed_non_schema_result_path(server: &TestServer, session_id: &str, result: Value) {
    use everruns_core::session_task::{
        CreateSessionTask, SessionTaskState, SessionTaskUpdate, new_session_task,
    };
    use everruns_server::storage::models::CreateSessionFileRow;

    let sid = session_id.parse::<SessionId>().expect("valid session id");
    let session = server
        .db
        .get_session(DEFAULT_ORG_ID, sid)
        .await
        .expect("get_session")
        .expect("session exists");

    let task = new_session_task(
        CreateSessionTask {
            session_id: sid,
            id: None,
            kind: "background_tool".to_string(),
            display_name: "background result".to_string(),
            spec: json!({ "tool": "internal_scan" }),
            state: SessionTaskState::Running,
            links: Default::default(),
            wake_policy: Default::default(),
        },
        chrono::Utc::now(),
    );
    server
        .db
        .create_session_task(&task)
        .await
        .expect("create non-schema task");

    let path = format!("/.background/{}/result.json", task.id);
    server
        .db
        .update_session_task(
            sid,
            &task.id,
            SessionTaskUpdate {
                state: Some(SessionTaskState::Succeeded),
                result_path: Some(path.clone()),
                ..Default::default()
            },
        )
        .await
        .expect("set non-schema result_path");

    server
        .db
        .create_session_file(CreateSessionFileRow {
            session_id: SessionId::from_uuid(session.workspace_id),
            path,
            content: Some(serde_json::to_vec(&result).unwrap()),
            is_directory: false,
            is_readonly: false,
        })
        .await
        .expect("write non-schema result file");
}

/// EVE-728: when a task reported a schema-bound `result.json`, `tasks/get`
/// surfaces it under `result.structured_result` for Tasks clients — not just
/// the last-message / status snapshot.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn test_mcp_tasks_get_surfaces_structured_result() {
    let server = TestServer::new().await;

    let run = mcp_task_tool_call(&server, "agent_run", json!({ "message": "task result" })).await;
    let task_id = run["result"]["taskId"].as_str().unwrap().to_string();

    // Before a structured result is reported, only the status snapshot is
    // present under `result`.
    let before = mcp_call_with_headers(
        &server,
        "tasks/get",
        json!({ "taskId": task_id, "_meta": tasks_opt_in_meta() }),
        vec![("MCP-Protocol-Version", MCP_PROTOCOL_VERSION_2026)],
    )
    .await;
    assert!(
        before["result"]["result"]
            .get("structured_result")
            .is_none()
    );

    let expected = json!({ "verdict": "ok", "score": 42 });
    seed_structured_result(&server, &task_id, expected.clone()).await;

    let resp = mcp_call_with_headers(
        &server,
        "tasks/get",
        json!({ "taskId": task_id, "_meta": tasks_opt_in_meta() }),
        vec![("MCP-Protocol-Version", MCP_PROTOCOL_VERSION_2026)],
    )
    .await;

    assert_eq!(resp["result"]["taskId"].as_str(), Some(task_id.as_str()));
    assert_eq!(resp["result"]["result"]["structured_result"], expected);
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn test_mcp_tasks_get_ignores_non_schema_result_path() {
    let server = TestServer::in_memory().await;

    let run = mcp_task_tool_call(&server, "agent_run", json!({ "message": "task result" })).await;
    let task_id = run["result"]["taskId"].as_str().unwrap().to_string();

    seed_non_schema_result_path(&server, &task_id, json!({ "internal": "tool-output" })).await;

    let resp = mcp_call_with_headers(
        &server,
        "tasks/get",
        json!({ "taskId": task_id, "_meta": tasks_opt_in_meta() }),
        vec![("MCP-Protocol-Version", MCP_PROTOCOL_VERSION_2026)],
    )
    .await;

    assert!(
        resp["result"]["result"].get("structured_result").is_none(),
        "non-schema result_path leaked as structured_result: {resp}"
    );
}

/// tasks/get for 2025-* is method_not_found (extension doesn't exist there).
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn test_mcp_tasks_get_rejected_for_2025() {
    let server = TestServer::in_memory().await;
    let resp = mcp_call_with_headers(
        &server,
        "tasks/get",
        json!({ "taskId": "session_00000000000000000000000000000000" }),
        vec![("MCP-Protocol-Version", MCP_PROTOCOL_VERSION_LATEST)],
    )
    .await;
    assert_eq!(resp["error"]["code"], -32601, "expected method not found");
}

/// tasks/* for 2026 clients still require the per-request Tasks opt-in.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn test_mcp_tasks_get_rejected_without_tasks_opt_in() {
    let server = TestServer::in_memory().await;
    let resp = mcp_call_with_headers(
        &server,
        "tasks/get",
        json!({ "taskId": "session_00000000000000000000000000000000" }),
        vec![("MCP-Protocol-Version", MCP_PROTOCOL_VERSION_2026)],
    )
    .await;
    assert_eq!(resp["error"]["code"], -32601, "expected method not found");
}

/// tasks/update sends input to a session and returns an updated Task object.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn test_mcp_tasks_update_round_trip() {
    let server = TestServer::new().await;

    let run = mcp_task_tool_call(&server, "agent_run", json!({ "message": "first" })).await;
    let task_id = run["result"]["taskId"].as_str().unwrap().to_string();

    let resp = mcp_call_with_headers(
        &server,
        "tasks/update",
        json!({ "taskId": task_id, "message": "follow up", "_meta": tasks_opt_in_meta() }),
        vec![("MCP-Protocol-Version", MCP_PROTOCOL_VERSION_2026)],
    )
    .await;

    let result = &resp["result"];
    assert_eq!(result["taskId"].as_str(), Some(task_id.as_str()));
    assert!(result["status"].is_string());
    // Underlying session_send_message result surfaced under `result`.
    assert!(result["result"]["message_id"].as_str().is_some());
}

/// tasks/update accepts the SEP-2663 `inputResponses` map form too.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn test_mcp_tasks_update_accepts_input_responses_map() {
    let server = TestServer::new().await;

    let run = mcp_task_tool_call(&server, "agent_run", json!({ "message": "first" })).await;
    let task_id = run["result"]["taskId"].as_str().unwrap().to_string();

    let resp = mcp_call_with_headers(
        &server,
        "tasks/update",
        json!({ "taskId": task_id, "inputResponses": { "prompt": "map form input" }, "_meta": tasks_opt_in_meta() }),
        vec![("MCP-Protocol-Version", MCP_PROTOCOL_VERSION_2026)],
    )
    .await;

    assert_eq!(resp["result"]["taskId"].as_str(), Some(task_id.as_str()));
    assert!(resp["result"]["result"]["message_id"].as_str().is_some());
}

/// tasks/cancel acknowledges cancellation of a task handle.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn test_mcp_tasks_cancel_round_trip() {
    let server = TestServer::new().await;

    let run = mcp_task_tool_call(&server, "agent_run", json!({ "message": "cancel me" })).await;
    let task_id = run["result"]["taskId"].as_str().unwrap().to_string();

    let resp = mcp_call_with_headers(
        &server,
        "tasks/cancel",
        json!({ "taskId": task_id, "_meta": tasks_opt_in_meta() }),
        vec![("MCP-Protocol-Version", MCP_PROTOCOL_VERSION_2026)],
    )
    .await;

    let result = &resp["result"];
    assert_eq!(result["taskId"].as_str(), Some(task_id.as_str()));
    // Cooperative cancel: status is a valid lifecycle value (often the
    // post-cancel session state rather than literally `cancelled`).
    assert!(result["status"].is_string());
}

/// tasks/* with a missing taskId is an invalid-params error.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn test_mcp_tasks_get_missing_task_id() {
    let server = TestServer::in_memory().await;
    let resp = mcp_call_with_headers(
        &server,
        "tasks/get",
        json!({ "_meta": tasks_opt_in_meta() }),
        vec![("MCP-Protocol-Version", MCP_PROTOCOL_VERSION_2026)],
    )
    .await;
    assert_eq!(resp["error"]["code"], -32602, "expected invalid params");
}

// ===========================================================================
// 2026-07-28 cacheable results (`resultType` / `ttlMs` / `cacheScope`)
// ===========================================================================

/// `tools/list` is identical for every caller, so it is cacheable and public.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn test_mcp_tools_list_carries_public_caching_hints_for_2026() {
    let server = TestServer::new().await;

    let resp = mcp_call_with_headers(
        &server,
        "tools/list",
        json!({}),
        vec![("MCP-Protocol-Version", MCP_PROTOCOL_VERSION_2026)],
    )
    .await;

    assert_eq!(resp["result"]["resultType"], "complete");
    assert_eq!(resp["result"]["cacheScope"], "public");
    let ttl = resp["result"]["ttlMs"]
        .as_i64()
        .expect("ttlMs must be an integer");
    assert!(ttl >= 0, "ttlMs must be non-negative, got {ttl}");
}

/// `resources/read` returns org-scoped data, so it must never be marked public.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn test_mcp_resources_read_is_private_for_2026() {
    let server = TestServer::new().await;

    let resp = mcp_call_with_headers(
        &server,
        "resources/read",
        json!({ "uri": "everruns://agents" }),
        vec![("MCP-Protocol-Version", MCP_PROTOCOL_VERSION_2026)],
    )
    .await;

    assert_eq!(resp["result"]["resultType"], "complete");
    assert_eq!(
        resp["result"]["cacheScope"], "private",
        "org-scoped payloads must not be shareable across authorization contexts"
    );
    assert!(resp["result"]["ttlMs"].as_i64().is_some());
}

/// 2025-era clients see the era they negotiated — no 2026 result metadata.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn test_mcp_tools_list_omits_caching_hints_for_2025() {
    let server = TestServer::new().await;

    let resp = mcp_call_with_headers(
        &server,
        "tools/list",
        json!({}),
        vec![("MCP-Protocol-Version", MCP_PROTOCOL_VERSION_LATEST)],
    )
    .await;

    assert!(resp["result"]["tools"].is_array());
    assert!(resp["result"].get("ttlMs").is_none());
    assert!(resp["result"].get("cacheScope").is_none());
    assert!(resp["result"].get("resultType").is_none());
}
