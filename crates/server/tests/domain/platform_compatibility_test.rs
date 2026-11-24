use axum::http::StatusCode;
use serde_json::{Value, json};

#[tokio::test]
async fn platform_legacy_and_canonical_management_share_resources() {
    let server = crate::test_harness::TestServer::in_memory().await;
    let response = server
        .post(
            "/v1/agents",
            json!({"name":"legacy-platform", "system_prompt":"test"}),
        )
        .await;
    assert_eq!(
        response
            .headers()
            .get("deprecation")
            .and_then(|h| h.to_str().ok()),
        Some(everruns_contracts::platform_api::LEGACY_DEPRECATION),
        "legacy agent create: {} {}",
        response.status(),
        response.text()
    );
    let agent: Value = response.assert_status(StatusCode::CREATED).json();
    let id = agent["id"].as_str().unwrap();
    let current: Value = server
        .get(&format!("/v1/platform/agents/{id}"))
        .await
        .assert_status(StatusCode::OK)
        .json();
    assert_eq!(current["id"], agent["id"]);
    server
        .patch(
            &format!("/v1/agents/{id}"),
            json!({"display_name":"Updated through legacy"}),
        )
        .await
        .assert_status(StatusCode::OK);
    let current: Value = server
        .get(&format!("/v1/platform/agents/{id}"))
        .await
        .assert_status(StatusCode::OK)
        .json();
    assert_eq!(current["display_name"], "Updated through legacy");
    let session: Value = server
        .post("/v1/sessions", json!({"agent_id": id}))
        .await
        .assert_status(StatusCode::CREATED)
        .json();
    let sid = session["id"].as_str().unwrap();
    server
        .get(&format!("/v1/platform/sessions/{sid}"))
        .await
        .assert_status(StatusCode::OK);
    server
        .get(&format!("/v1/sessions/{sid}/events"))
        .await
        .assert_status(StatusCode::OK);
}
