//! Integration tests for the MCP Apps views on `/mcp` (EVE-1122).
//!
//! Covers discovery (`server/discover`, `tools/list` `_meta`), the templates
//! served through `resources/read`, and the app-only tools the views call:
//! `everruns_home`, `session_view`, `session_answer_question`, and
//! `session_decide_approval`. See `knowledge/ui/mcp-apps.md`.
//!
//! Run with: cargo test -p everruns-server --test domain mcp_apps_test:: -- --test-threads=1

use crate::test_harness::{TestServer, extract_cookie};
use axum::http::{Method, StatusCode};
use everruns_contracts::typed_id::SessionId;
use everruns_core::DEFAULT_ORG_ID;
use everruns_server::storage::{CreateEventRow, UpdateSession};
use serde_json::{Value, json};
use std::sync::Arc;
use std::sync::atomic::{AtomicUsize, Ordering};

const LATEST: &str = "2026-07-28";
const SESSION_VIEW_URI: &str = "ui://everruns/app/session";
const HOME_VIEW_URI: &str = "ui://everruns/app/home";
const APP_MIME: &str = "text/html;profile=mcp-app";

/// Records resumes instead of running a model turn: what matters is that the
/// answered question handed the parked turn back to the runner.
#[derive(Default)]
struct ResumeRecordingRunner {
    resumes: AtomicUsize,
}

#[async_trait::async_trait]
impl everruns_core::host::TurnBackend for ResumeRecordingRunner {
    async fn start_turn(
        &self,
        request: everruns_core::host::TurnRequest,
    ) -> everruns_contracts::error::Result<everruns_core::host::TurnTicket> {
        if matches!(
            request.input,
            everruns_core::host::TurnInput::RecordedToolResults { .. }
        ) {
            self.resumes.fetch_add(1, Ordering::SeqCst);
        }
        // The server drops its tickets; this one never resolves.
        Ok(everruns_core::host::TurnTicket::new(
            request.session_id,
            request.turn_id,
            std::future::pending(),
        ))
    }

    async fn cancel(
        &self,
        _session_id: everruns_contracts::typed_id::SessionId,
    ) -> everruns_contracts::error::Result<bool> {
        Ok(false)
    }

    async fn is_running(&self, _session_id: everruns_contracts::typed_id::SessionId) -> bool {
        false
    }

    async fn active_count(&self) -> usize {
        0
    }
}

static COUNTER: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);

fn unique(prefix: &str) -> String {
    let n = COUNTER.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
    let millis = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap()
        .as_millis();
    format!("{prefix}-{millis}-{n}")
}

async fn rpc(
    server: &TestServer,
    method: &str,
    params: Value,
    version: &str,
    extra: Vec<(&str, &str)>,
) -> Value {
    let mut headers = vec![
        ("content-type", "application/json"),
        ("MCP-Protocol-Version", version),
    ];
    headers.extend(extra);
    server
        .request_raw(
            Method::POST,
            "/mcp",
            headers,
            serde_json::to_vec(&json!({
                "jsonrpc": "2.0", "id": 1, "method": method, "params": params
            }))
            .unwrap(),
        )
        .await
        .assert_success()
        .json()
}

async fn call(server: &TestServer, tool: &str, arguments: Value) -> Value {
    rpc(
        server,
        "tools/call",
        json!({ "name": tool, "arguments": arguments }),
        LATEST,
        vec![],
    )
    .await
}

async fn call_as(server: &TestServer, tool: &str, arguments: Value, cookie: &str) -> Value {
    rpc(
        server,
        "tools/call",
        json!({ "name": tool, "arguments": arguments }),
        LATEST,
        vec![("cookie", cookie)],
    )
    .await
}

fn is_error(resp: &Value) -> bool {
    resp["result"]["isError"].as_bool().unwrap_or(false)
}

fn text(resp: &Value) -> String {
    resp["result"]["content"][0]["text"]
        .as_str()
        .unwrap_or_default()
        .to_string()
}

fn structured(resp: &Value) -> Value {
    assert!(!is_error(resp), "tool failed: {}", text(resp));
    resp["result"]["structuredContent"].clone()
}

async fn create_agent(server: &TestServer, name: &str) -> String {
    let resp = call(
        server,
        "execute",
        json!({ "commands": format!(
            "create_agent --name '{name}' --display_name 'Apps Agent' --system_prompt 'Test prompt'"
        ) }),
    )
    .await;
    assert!(!is_error(&resp), "create_agent failed: {}", text(&resp));
    let payload: Value = serde_json::from_str(&text(&resp)).unwrap();
    payload["id"].as_str().unwrap().to_string()
}

async fn start_session(server: &TestServer, agent_id: &str, message: &str) -> String {
    let resp = call(
        server,
        "agent_run",
        json!({ "agent_id": agent_id, "message": message, "title": unique("apps session") }),
    )
    .await;
    structured(&resp)["session_id"]
        .as_str()
        .expect("session id")
        .to_string()
}

async fn emit(server: &TestServer, session_id: &str, event_type: &str, data: Value) {
    server
        .db
        .create_event(CreateEventRow {
            session_id: session_id.parse::<SessionId>().unwrap(),
            event_type: event_type.to_string(),
            ts: chrono::Utc::now(),
            context: json!({}),
            data,
            metadata: None,
            tags: None,
        })
        .await
        .expect("emit event");
}

async fn set_status(server: &TestServer, session_id: &str, status: &str) {
    server
        .db
        .update_session(
            DEFAULT_ORG_ID,
            session_id.parse::<SessionId>().unwrap(),
            UpdateSession {
                status: Some(status.to_string()),
                ..Default::default()
            },
        )
        .await
        .expect("update session")
        .expect("session exists");
}

async fn park_on_approval(server: &TestServer, session_id: &str, action: &str) {
    let payload = json!({
        "awaiting_approval": true,
        "action": action,
        "question": "Shall I go ahead?",
    });
    emit(
        server,
        session_id,
        "tool.completed",
        json!({
            "tool_call_id": unique("call_approval"),
            "tool_name": "request_approval",
            "success": true,
            "status": "success",
            "result": [{ "type": "text", "text": payload.to_string() }],
        }),
    )
    .await;
    set_status(server, session_id, "idle").await;
}

// ============================================================================
// Discovery
// ============================================================================

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn server_discover_advertises_mcp_apps() {
    let server = TestServer::in_memory().await;
    let resp = rpc(&server, "server/discover", json!({}), LATEST, vec![]).await;
    let result = &resp["result"];
    assert_eq!(result["supportedVersions"][0], LATEST);
    assert_eq!(
        result["capabilities"]["extensions"]["io.modelcontextprotocol/ui"]["mimeTypes"],
        json!([APP_MIME])
    );
    assert_eq!(
        result["_meta"]["io.modelcontextprotocol/serverInfo"]["name"],
        "everruns"
    );
    assert_eq!(
        result["_meta"]["io.modelcontextprotocol/serverInfo"]["version"],
        env!("CARGO_PKG_VERSION")
    );
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn tools_list_links_tools_to_views() {
    let server = TestServer::in_memory().await;
    let resp = rpc(&server, "tools/list", json!({}), LATEST, vec![]).await;
    let tools = resp["result"]["tools"].as_array().unwrap();
    let tool = |name: &str| {
        tools
            .iter()
            .find(|t| t["name"] == name)
            .unwrap_or_else(|| panic!("{name} listed"))
            .clone()
    };

    assert_eq!(
        tool("agent_run")["_meta"]["ui"]["resourceUri"],
        SESSION_VIEW_URI
    );
    let home = tool("everruns_home");
    assert_eq!(home["_meta"]["ui"]["resourceUri"], HOME_VIEW_URI);
    assert_eq!(
        home["_meta"]["openai/ui"]["entrypoints"],
        json!([{ "type": "global" }, { "type": "thread" }])
    );
    assert!(
        home["icons"][0]["src"]
            .as_str()
            .unwrap()
            .starts_with("data:image/svg+xml")
    );
    for app_only in [
        "session_view",
        "session_answer_question",
        "session_decide_approval",
    ] {
        assert_eq!(tool(app_only)["_meta"]["ui"]["visibility"], json!(["app"]));
    }

    // The fallback protocol keeps its old shape: no views, no `_meta`.
    let old = rpc(&server, "tools/list", json!({}), "2025-03-26", vec![]).await;
    for tool in old["result"]["tools"].as_array().unwrap() {
        assert!(tool.get("_meta").is_none(), "{} has _meta", tool["name"]);
        assert_ne!(tool["name"], "everruns_home");
    }
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn templates_are_served_as_mcp_app_resources() {
    let server = TestServer::in_memory().await;
    let list = rpc(&server, "resources/list", json!({}), LATEST, vec![]).await;
    let uris: Vec<&str> = list["result"]["resources"]
        .as_array()
        .unwrap()
        .iter()
        .filter_map(|r| r["uri"].as_str())
        .collect();
    assert!(uris.contains(&SESSION_VIEW_URI) && uris.contains(&HOME_VIEW_URI));

    let read = rpc(
        &server,
        "resources/read",
        json!({ "uri": SESSION_VIEW_URI }),
        LATEST,
        vec![],
    )
    .await;
    let content = &read["result"]["contents"][0];
    assert_eq!(content["mimeType"], APP_MIME);
    assert!(
        content["text"]
            .as_str()
            .unwrap()
            .starts_with("<!doctype html>")
    );
    assert_eq!(content["_meta"]["ui"]["prefersBorder"], true);
}

// ============================================================================
// Views
// ============================================================================

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn home_and_session_view_show_the_run() {
    let server = TestServer::in_memory().await;
    let agent_id = create_agent(&server, &unique("apps-home")).await;
    let session_id = start_session(&server, &agent_id, "hello from the app").await;

    let home = structured(&call(&server, "everruns_home", json!({})).await);
    assert!(
        home["agents"]
            .as_array()
            .unwrap()
            .iter()
            .any(|a| a["id"] == agent_id.as_str()),
        "home lists the agent: {home}"
    );

    let view =
        structured(&call(&server, "session_view", json!({ "session_id": session_id })).await);
    assert_eq!(view["session_id"], session_id.as_str());
    assert_eq!(view["agent_id"], agent_id.as_str());
    assert_eq!(view["messages"][0]["role"], "user");
    assert_eq!(view["messages"][0]["text"], "hello from the app");
    assert!(view["pending"].is_null());
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn approval_is_decided_from_the_view_and_stale_clicks_are_refused() {
    let server = TestServer::in_memory().await;
    let agent_id = create_agent(&server, &unique("apps-approval")).await;
    let session_id = start_session(&server, &agent_id, "ship it").await;
    park_on_approval(&server, &session_id, "Deploy build 42 to production").await;

    let view =
        structured(&call(&server, "session_view", json!({ "session_id": session_id })).await);
    assert_eq!(view["pending"]["kind"], "approval");
    assert_eq!(view["pending"]["action"], "Deploy build 42 to production");

    let home = structured(&call(&server, "everruns_home", json!({})).await);
    let listed = home["sessions"]
        .as_array()
        .unwrap()
        .iter()
        .find(|s| s["session_id"] == session_id.as_str())
        .expect("session on home");
    assert_eq!(listed["pending"]["kind"], "approval");

    // A click on a card for a different pause must not approve this one.
    let stale = call(
        &server,
        "session_decide_approval",
        json!({ "session_id": session_id, "decision": "approve", "action": "Deploy build 41 to production" }),
    )
    .await;
    assert!(is_error(&stale), "stale click accepted");
    assert!(text(&stale).contains("out of date"), "{}", text(&stale));

    let decided = call(
        &server,
        "session_decide_approval",
        json!({ "session_id": session_id, "decision": "approve", "action": "Deploy build 42 to production" }),
    )
    .await;
    let view = structured(&decided);
    assert!(
        view["messages"]
            .as_array()
            .unwrap()
            .iter()
            .any(|m| m["role"] == "user" && m["text"] == "Approved: Deploy build 42 to production"),
        "decision posted as the user's message: {view}"
    );
    assert!(view["pending"].is_null() || view["pending"]["kind"] != "approval");

    // Answered once: a second click finds nothing pending.
    set_status(&server, &session_id, "idle").await;
    let again = call(
        &server,
        "session_decide_approval",
        json!({ "session_id": session_id, "decision": "decline", "action": "Deploy build 42 to production" }),
    )
    .await;
    assert!(is_error(&again));
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn question_is_rendered_and_answers_are_validated() {
    let runner = Arc::new(ResumeRecordingRunner::default());
    let server = TestServer::in_memory_with_runner(runner.clone()).await;
    let agent_id = create_agent(&server, &unique("apps-question")).await;
    let session_id = start_session(&server, &agent_id, "deploy somewhere").await;
    emit(
        &server,
        &session_id,
        "tool.call_requested",
        json!({ "tool_calls": [{
            "id": "call_ask_apps",
            "name": "ask_user",
            "arguments": { "questions": [{
                "kind": "choice", "id": "target", "header": "Target",
                "question": "Which environment?", "multi_select": false, "allow_other": false,
                "options": [
                    { "label": "Staging", "description": "Safe.", "default": true },
                    { "label": "Production", "description": "Live." }
                ]
            }], "timeout_seconds": 300 }
        }] }),
    )
    .await;
    set_status(&server, &session_id, "waiting_for_tool_results").await;

    let view =
        structured(&call(&server, "session_view", json!({ "session_id": session_id })).await);
    assert_eq!(view["pending"]["kind"], "question");
    assert_eq!(view["pending"]["tool_call_id"], "call_ask_apps");
    assert_eq!(
        view["pending"]["questions"][0]["options"][1]["label"],
        "Production"
    );

    // A label that was never offered is refused and the question stays open.
    let invalid = call(
        &server,
        "session_answer_question",
        json!({ "session_id": session_id, "tool_call_id": "call_ask_apps", "status": "answered",
                "answers": [{ "id": "target", "selected": ["Moon"] }] }),
    )
    .await;
    assert!(is_error(&invalid), "invalid label accepted");

    let answered = call(
        &server,
        "session_answer_question",
        json!({ "session_id": session_id, "tool_call_id": "call_ask_apps", "status": "answered",
                "answers": [{ "id": "target", "selected": ["Staging"] }] }),
    )
    .await;
    let view = structured(&answered);
    assert_ne!(view["pending"]["kind"], "question", "still pending: {view}");
    assert_eq!(runner.resumes.load(Ordering::SeqCst), 1, "turn not resumed");
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn view_tools_cannot_reach_another_org() {
    let server = TestServer::in_memory().await;
    let org2: Value = server
        .post("/v1/orgs", json!({ "name": unique("Apps Other Org") }))
        .await
        .assert_status(StatusCode::CREATED)
        .json();
    let switch = server
        .post(
            "/v1/users/me/switch-org",
            json!({ "org_id": org2["id"].as_str().unwrap() }),
        )
        .await
        .assert_status(StatusCode::OK);
    let org2_cookie = extract_cookie(switch.headers(), "everruns_org");

    let create = call_as(
        &server,
        "execute",
        json!({ "commands": format!(
            "create_agent --name '{}' --display_name 'Other' --system_prompt 'x'",
            unique("apps-org2")
        ) }),
        &org2_cookie,
    )
    .await;
    let org2_agent: Value = serde_json::from_str(&text(&create)).unwrap();
    let run = call_as(
        &server,
        "agent_run",
        json!({ "agent_id": org2_agent["id"], "message": "org2 secret work" }),
        &org2_cookie,
    )
    .await;
    let org2_session = structured(&run)["session_id"].as_str().unwrap().to_string();

    // Default org caller.
    let view = call(
        &server,
        "session_view",
        json!({ "session_id": org2_session }),
    )
    .await;
    assert!(is_error(&view), "cross-org view leaked: {view}");
    assert!(!view.to_string().contains("org2 secret work"));

    let decide = call(
        &server,
        "session_decide_approval",
        json!({ "session_id": org2_session, "decision": "approve", "action": "anything" }),
    )
    .await;
    assert!(is_error(&decide));

    let home = structured(&call(&server, "everruns_home", json!({})).await);
    assert!(!home.to_string().contains(org2_session.as_str()));
}
