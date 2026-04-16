//! AG-UI app channel integration tests.
//!
//! These tests cover the app-scoped AG-UI endpoint at the route boundary:
//! - published app gating
//! - request validation for the AG-UI contract

mod test_harness;

use axum::http::{Method, StatusCode};
use serde_json::{Value, json};
use test_harness::TestServer;

use everruns_core::App;

/// Seed harness ID (BASE_HARNESS)
const SEED_BASE_HARNESS_ID: &str = "harness_01933b5a000070008000000000000601";

fn unique_id(prefix: &str) -> String {
    use std::sync::atomic::{AtomicU64, Ordering};
    static COUNTER: AtomicU64 = AtomicU64::new(0);
    let now = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap()
        .as_millis();
    let seq = COUNTER.fetch_add(1, Ordering::Relaxed);
    format!("{prefix}_{now}_{seq}")
}

fn unique_slug(prefix: &str) -> String {
    use std::sync::atomic::{AtomicU64, Ordering};
    static COUNTER: AtomicU64 = AtomicU64::new(0);
    let now = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap()
        .as_millis();
    let seq = COUNTER.fetch_add(1, Ordering::Relaxed);
    format!("{prefix}-{now}-{seq}")
}

fn raw_uuid() -> String {
    uuid::Uuid::new_v4().to_string()
}

async fn create_llmsim_agent(server: &TestServer) -> String {
    let provider: Value = server
        .post(
            "/v1/llm-providers",
            json!({
                "name": unique_id("AG-UI Test Provider"),
                "provider_type": "llmsim"
            }),
        )
        .await
        .assert_status(StatusCode::CREATED)
        .json();

    let model: Value = server
        .post(
            &format!(
                "/v1/llm-providers/{}/models",
                provider["id"].as_str().unwrap()
            ),
            json!({
                "model_id": unique_id("llmsim-streaming"),
                "display_name": unique_id("AG-UI Streaming Model")
            }),
        )
        .await
        .assert_status(StatusCode::CREATED)
        .json();

    let agent: Value = server
        .post(
            "/v1/agents",
            json!({
                "name": unique_slug("ag-ui-test-agent"),
                "display_name": unique_id("AG-UI Test Agent"),
                "system_prompt": "You are a brief test agent.",
                "default_model_id": model["id"]
            }),
        )
        .await
        .assert_status(StatusCode::CREATED)
        .json();

    agent["id"].as_str().unwrap().to_string()
}

async fn create_published_ag_ui_app(server: &TestServer) -> App {
    let agent_id = create_llmsim_agent(server).await;

    let app: App = server
        .post(
            "/v1/apps",
            json!({
                "name": unique_id("AG-UI App"),
                "harness_id": SEED_BASE_HARNESS_ID,
                "agent_id": agent_id,
                "channel_type": "ag_ui",
                "channel_config": {
                    "anonymous": true
                }
            }),
        )
        .await
        .assert_status(StatusCode::CREATED)
        .json();

    server
        .post(&format!("/v1/apps/{}/publish", app.public_id), json!({}))
        .await
        .assert_success();

    server
        .get(&format!("/v1/apps/{}", app.public_id))
        .await
        .assert_success()
        .json()
}

async fn send_ag_ui_run(
    server: &TestServer,
    app_id: impl std::fmt::Display,
    payload: &Value,
) -> test_harness::TestResponse {
    server
        .request_raw(
            Method::POST,
            &format!("/v1/apps/{}/ag-ui", app_id),
            vec![
                ("content-type", "application/json"),
                ("accept", "text/event-stream"),
            ],
            serde_json::to_vec(payload).unwrap(),
        )
        .await
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn test_ag_ui_rejects_missing_messages() {
    let server = TestServer::in_memory().await;
    let app = create_published_ag_ui_app(&server).await;

    let payload = json!({
        "threadId": raw_uuid(),
        "runId": raw_uuid(),
        "state": {},
        "messages": []
        ,
        "tools": [],
        "context": [],
        "forwardedProps": {}
    });

    send_ag_ui_run(&server, &app.public_id, &payload)
        .await
        .assert_status(StatusCode::BAD_REQUEST);
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn test_ag_ui_rejects_non_user_final_message() {
    let server = TestServer::in_memory().await;
    let app = create_published_ag_ui_app(&server).await;
    let payload = json!({
        "threadId": raw_uuid(),
        "runId": raw_uuid(),
        "state": {},
        "messages": [
            {
                "id": raw_uuid(),
                "role": "assistant",
                "content": "I should not be accepted as the trigger message"
            }
        ],
        "tools": [],
        "context": [],
        "forwardedProps": {}
    });

    send_ag_ui_run(&server, &app.public_id, &payload)
        .await
        .assert_status(StatusCode::BAD_REQUEST);
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn test_ag_ui_unpublished_app_rejected() {
    let server = TestServer::in_memory().await;
    let agent_id = create_llmsim_agent(&server).await;

    let app: App = server
        .post(
            "/v1/apps",
            json!({
                "name": unique_id("Draft AG-UI App"),
                "harness_id": SEED_BASE_HARNESS_ID,
                "agent_id": agent_id,
                "channel_type": "ag_ui",
                "channel_config": { "anonymous": true }
            }),
        )
        .await
        .assert_status(StatusCode::CREATED)
        .json();

    let payload = json!({
        "threadId": raw_uuid(),
        "runId": raw_uuid(),
        "state": {},
        "messages": [{ "id": raw_uuid(), "role": "user", "content": "Hello" }],
        "tools": [],
        "context": [],
        "forwardedProps": {}
    });

    send_ag_ui_run(&server, &app.public_id, &payload)
        .await
        .assert_status(StatusCode::FORBIDDEN);
}
