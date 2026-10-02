//! `GET /v1/e/{endpoint_id}/ag-ui/capabilities`: the AG-UI 1.0
//! `AgentCapabilities` an endpoint declares, behind the run route's auth.

use crate::test_harness;

use axum::http::{Method, StatusCode};
use serde_json::{Value, json};
use test_harness::TestServer;

async fn published_endpoint(server: &TestServer, channel_config: Value) -> String {
    let agent: Value = server
        .post(
            "/v1/agents",
            json!({
                "name": format!("ag-ui-caps-{}", uuid::Uuid::new_v4().simple()),
                "system_prompt": "Test",
            }),
        )
        .await
        .assert_status(StatusCode::CREATED)
        .json();
    let agent_id = agent["id"].as_str().unwrap();
    let endpoint: Value = server
        .post(
            &format!("/v1/agents/{agent_id}/endpoints"),
            json!({ "channel_type": "ag_ui", "channel_config": channel_config }),
        )
        .await
        .assert_status(StatusCode::CREATED)
        .json();
    let endpoint_id = endpoint["id"].as_str().unwrap().to_string();
    server
        .post(
            &format!("/v1/agents/{agent_id}/endpoints/{endpoint_id}/publish"),
            json!({}),
        )
        .await
        .assert_success();
    endpoint_id
}

async fn get_capabilities(
    server: &TestServer,
    endpoint_id: &str,
    headers: Vec<(&str, &str)>,
) -> test_harness::TestResponse {
    server
        .request_raw(
            Method::GET,
            &format!("/v1/e/{endpoint_id}/ag-ui/capabilities"),
            headers,
            Vec::new(),
        )
        .await
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn ag_ui_capabilities_follow_the_endpoint_config() {
    let server = TestServer::in_memory().await;
    let defaults = published_endpoint(&server, json!({ "anonymous": true })).await;
    let caps: Value = get_capabilities(&server, &defaults, vec![])
        .await
        .assert_status(StatusCode::OK)
        .json();
    assert_eq!(caps["identity"]["type"], "everruns");
    assert_eq!(caps["transport"]["streaming"], true);
    assert_eq!(caps["tools"]["clientProvided"], true);
    assert_eq!(caps["humanInTheLoop"]["interrupts"], true);
    assert_eq!(caps["humanInTheLoop"]["approvals"], false);
    assert_eq!(caps["reasoning"]["supported"], false);
    assert!(
        caps.get("multiAgent").is_none(),
        "subagents are off by default"
    );
    assert_eq!(caps["custom"]["everruns"]["usage"], false);

    let opted_in = published_endpoint(
        &server,
        json!({
            "anonymous": true,
            "subagents_visible": true,
            "usage_visible": true,
            "tool_approval_interrupts": true,
        }),
    )
    .await;
    let caps: Value = get_capabilities(&server, &opted_in, vec![])
        .await
        .assert_status(StatusCode::OK)
        .json();
    assert_eq!(caps["multiAgent"]["supported"], true);
    assert_eq!(caps["humanInTheLoop"]["approvals"], true);
    assert_eq!(caps["custom"]["everruns"]["usage"], true);
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn ag_ui_capabilities_use_the_run_routes_auth() {
    let server = TestServer::in_memory().await;
    let gated =
        published_endpoint(&server, json!({ "anonymous": true, "token": "caps-token" })).await;
    get_capabilities(&server, &gated, vec![])
        .await
        .assert_status(StatusCode::UNAUTHORIZED);
    get_capabilities(&server, &gated, vec![("authorization", "Bearer wrong")])
        .await
        .assert_status(StatusCode::UNAUTHORIZED);
    get_capabilities(
        &server,
        &gated,
        vec![("authorization", "Bearer caps-token")],
    )
    .await
    .assert_status(StatusCode::OK);

    let closed = published_endpoint(&server, json!({ "anonymous": false })).await;
    get_capabilities(&server, &closed, vec![])
        .await
        .assert_status(StatusCode::NOT_FOUND);
    get_capabilities(&server, "ep_doesnotexist", vec![])
        .await
        .assert_status(StatusCode::NOT_FOUND);
}
