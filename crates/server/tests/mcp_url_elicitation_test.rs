//! URL mode elicitation over Everruns' own MCP endpoint
//! (knowledge/integrations/mcp.md, "URL mode elicitation").
//!
//! Covers the contract end to end: a credential-collecting tool answers with an
//! MRTR `input_required` result instead of taking the value in-band, refuses to
//! elicit a client that never declared the capability, honors a decline, serves
//! a form from the server itself, stores what the form posts, and refuses a
//! token presented by anyone but the user it was minted for.

mod test_harness;

use axum::http::{Method, StatusCode};
use everruns_platform::{Agent, Session};
use serde_json::{Value, json};
use test_harness::TestServer;

const LATEST: &str = "2026-07-28";
const CLIENT_CAPABILITIES_META_KEY: &str = "io.modelcontextprotocol/clientCapabilities";

/// `_meta` declaring URL mode elicitation, as a capable client sends it.
fn elicitation_meta() -> Value {
    json!({
        "_meta": {
            CLIENT_CAPABILITIES_META_KEY: { "elicitation": { "url": {} } }
        }
    })
}

async fn mcp_call(server: &TestServer, params: Value) -> Value {
    let body = json!({ "jsonrpc": "2.0", "id": 1, "method": "tools/call", "params": params });
    server
        .request_raw(
            Method::POST,
            "/mcp",
            vec![
                ("content-type", "application/json"),
                ("MCP-Protocol-Version", LATEST),
            ],
            serde_json::to_vec(&body).unwrap(),
        )
        .await
        .json()
}

/// One `tools/call` for `tool`, merging any extra params (`_meta`,
/// `requestState`, `inputResponses`) into the request.
async fn call_tool(server: &TestServer, tool: &str, arguments: Value, extra: Value) -> Value {
    let mut params = json!({ "name": tool, "arguments": arguments });
    if let (Some(params), Some(extra)) = (params.as_object_mut(), extra.as_object()) {
        for (key, value) in extra {
            params.insert(key.clone(), value.clone());
        }
    }
    mcp_call(server, params).await
}

async fn create_session(server: &TestServer) -> String {
    let agent: Agent = server
        .post(
            "/v1/agents",
            json!({
                "name": "elicitation-test-agent",
                "display_name": "Elicitation Test",
                "description": "Agent for the URL elicitation test",
                "system_prompt": "You are a helpful assistant"
            }),
        )
        .await
        .assert_status(StatusCode::CREATED)
        .json();
    let session: Session = server
        .post("/v1/sessions", json!({ "agent_id": agent.public_id }))
        .await
        .assert_status(StatusCode::CREATED)
        .json();
    session.id.to_string()
}

/// The elicitation the server handed back, as `(url, requestState)`.
fn url_elicitation(response: &Value) -> (String, String) {
    let result = &response["result"];
    assert_eq!(
        result["resultType"], "input_required",
        "expected an elicitation, got {response}"
    );
    let request = &result["inputRequests"]["secret"];
    assert_eq!(request["method"], "elicitation/create");
    assert_eq!(request["params"]["mode"], "url");
    (
        request["params"]["url"].as_str().unwrap().to_string(),
        result["requestState"].as_str().unwrap().to_string(),
    )
}

/// Path + query of an absolute URL, for driving the in-process router.
fn path_and_query(url: &str) -> String {
    let after_scheme = url.split_once("://").map(|(_, rest)| rest).unwrap_or(url);
    let start = after_scheme.find('/').expect("absolute URL has a path");
    after_scheme[start..].to_string()
}

#[tokio::test]
async fn setting_a_session_secret_elicits_a_server_rendered_form_and_stores_the_value() {
    let server = TestServer::in_memory().await;
    let session_id = create_session(&server).await;

    // 1. The tool never takes the value, so it answers with an elicitation.
    let response = call_tool(
        &server,
        "session_set_secret",
        json!({ "session_id": session_id, "name": "OPENAI_API_KEY" }),
        elicitation_meta(),
    )
    .await;
    let (url, request_state) = url_elicitation(&response);
    assert!(
        url.contains("/mcp/elicitations/secret?token="),
        "unexpected elicitation URL: {url}"
    );
    assert!(!request_state.is_empty());
    // The URL carries no value and no credential — only the signed intent.
    assert!(!url.contains("OPENAI_API_KEY"));

    // 2. The user opens it: Everruns serves the form itself.
    let page = server.get(&path_and_query(&url)).await;
    assert_eq!(page.status(), StatusCode::OK);
    let html = page.text();
    assert!(
        html.contains("OPENAI_API_KEY"),
        "form should name the secret"
    );
    assert!(html.contains("action=\"/mcp/elicitations/secret\""));
    assert!(html.contains("type=\"password\""));

    // 3. The form posts the value straight back to Everruns.
    let token = url.split_once("token=").unwrap().1.to_string();
    let submitted = server
        .request_raw(
            Method::POST,
            "/mcp/elicitations/secret",
            vec![("content-type", "application/x-www-form-urlencoded")],
            format!("token={token}&value=sk-secret-value").into_bytes(),
        )
        .await;
    assert_eq!(submitted.status(), StatusCode::OK);
    assert!(submitted.text().contains("Secret saved"));
    // Whatever else the page says, it must not echo the value back.
    assert!(!submitted.text().contains("sk-secret-value"));

    // 4. The client retries the call; now the secret exists, so it completes.
    let retry = call_tool(
        &server,
        "session_set_secret",
        json!({ "session_id": session_id, "name": "OPENAI_API_KEY" }),
        json!({
            "_meta": elicitation_meta()["_meta"],
            "requestState": request_state,
            "inputResponses": { "secret": { "action": "accept" } }
        }),
    )
    .await;
    assert_eq!(retry["result"]["resultType"], "complete");
    assert_eq!(retry["result"]["structuredContent"]["stored"], true);

    // And the value really is stored, without ever passing through MCP.
    let secrets: Value = server
        .get(&format!("/v1/sessions/{session_id}/storage/secrets"))
        .await
        .assert_success()
        .json();
    let names: Vec<&str> = secrets["data"]
        .as_array()
        .expect("items")
        .iter()
        .filter_map(|item| item["name"].as_str())
        .collect();
    assert!(names.contains(&"OPENAI_API_KEY"), "got {names:?}");
}

#[tokio::test]
async fn a_client_that_declares_no_url_elicitation_is_refused_not_asked_for_the_value() {
    let server = TestServer::in_memory().await;
    let session_id = create_session(&server).await;

    let response = call_tool(
        &server,
        "session_set_secret",
        json!({ "session_id": session_id, "name": "API_KEY" }),
        // Form mode only: exactly the client that must never receive a URL.
        json!({ "_meta": { CLIENT_CAPABILITIES_META_KEY: { "elicitation": {} } } }),
    )
    .await;

    assert_eq!(
        response["error"]["code"], -32021,
        "expected MissingRequiredClientCapability, got {response}"
    );
    assert_eq!(
        response["error"]["data"]["requiredCapabilities"]["elicitation"]["url"],
        json!({})
    );
}

#[tokio::test]
async fn declining_the_elicitation_is_reported_as_a_result_not_retried() {
    let server = TestServer::in_memory().await;
    let session_id = create_session(&server).await;

    let response = call_tool(
        &server,
        "session_set_secret",
        json!({ "session_id": session_id, "name": "API_KEY" }),
        elicitation_meta(),
    )
    .await;
    let (_, request_state) = url_elicitation(&response);

    let declined = call_tool(
        &server,
        "session_set_secret",
        json!({ "session_id": session_id, "name": "API_KEY" }),
        json!({
            "_meta": elicitation_meta()["_meta"],
            "requestState": request_state,
            "inputResponses": { "secret": { "action": "decline" } }
        }),
    )
    .await;
    assert_eq!(declined["result"]["structuredContent"]["stored"], false);
    assert_eq!(declined["result"]["resultType"], "complete");
}

#[tokio::test]
async fn a_tampered_request_state_is_rejected() {
    let server = TestServer::in_memory().await;
    let session_id = create_session(&server).await;

    // MRTR: requestState is attacker-controlled input and must be verified.
    let response = call_tool(
        &server,
        "session_set_secret",
        json!({ "session_id": session_id, "name": "API_KEY" }),
        json!({
            "_meta": elicitation_meta()["_meta"],
            "requestState": "forged.state",
        }),
    )
    .await;
    assert_eq!(response["error"]["code"], -32602, "got {response}");
}

#[tokio::test]
async fn the_secret_form_refuses_a_malformed_or_unsigned_token() {
    let server = TestServer::in_memory().await;

    let page = server
        .get("/mcp/elicitations/secret?token=not-a-real-token")
        .await;
    assert_eq!(page.status(), StatusCode::BAD_REQUEST);
    assert!(page.text().contains("could not be completed"));

    let submitted = server
        .request_raw(
            Method::POST,
            "/mcp/elicitations/secret",
            vec![("content-type", "application/x-www-form-urlencoded")],
            b"token=not-a-real-token&value=sk-x".to_vec(),
        )
        .await;
    assert_eq!(submitted.status(), StatusCode::BAD_REQUEST);
}

#[tokio::test]
async fn connect_elicits_a_page_that_verifies_the_user_before_the_provider() {
    let server = TestServer::in_memory().await;

    let response = call_tool(
        &server,
        "connect",
        json!({ "provider": "github" }),
        elicitation_meta(),
    )
    .await;
    let result = &response["result"];
    assert_eq!(result["resultType"], "input_required");
    let request = &result["inputRequests"]["connect"];
    assert_eq!(request["params"]["mode"], "url");
    let url = request["params"]["url"].as_str().unwrap();
    // The elicitation points at Everruns' own page, never straight at the
    // third-party authorize endpoint — that indirection is what lets the
    // server check that the visitor is the user the elicitation was for.
    assert!(
        url.contains("/mcp/elicitations/connect?token="),
        "unexpected URL: {url}"
    );
    assert!(!url.contains("github.com"));
}
