use crate::test_harness::TestServer;
use axum::http::StatusCode;
use serde_json::{Value, json};

#[tokio::test]
async fn canonical_channels_and_endpoint_aliases_share_records_and_liveness() {
    let server = TestServer::new().await;
    let agent: Value = server
        .post(
            "/v1/agents",
            json!({
                "name": format!("channel-rename-{}", uuid::Uuid::new_v4().simple()),
                "system_prompt": "Test channel routing"
            }),
        )
        .await
        .assert_status(StatusCode::CREATED)
        .json();
    let agent_id = agent["id"].as_str().unwrap();
    let canonical = format!("/v1/agents/{agent_id}/channels");
    let legacy = format!("/v1/agents/{agent_id}/endpoints");
    let channel: Value = server
        .post(
            &canonical,
            json!({
                "channel_type": "fcp", "channel_config": {"anonymous": true}
            }),
        )
        .await
        .assert_status(StatusCode::CREATED)
        .json();
    let id = channel["id"].as_str().unwrap();
    assert!(id.starts_with("appchan_"));
    let historical: Value = server
        .get(&format!("{legacy}/{id}"))
        .await
        .assert_status(StatusCode::OK)
        .json();
    assert_eq!(historical, channel);
    let listed: Vec<Value> = server
        .get(&canonical)
        .await
        .assert_status(StatusCode::OK)
        .json();
    assert_eq!(listed, vec![channel.clone()]);

    server
        .post(&format!("{legacy}/{id}/publish"), json!({}))
        .await
        .assert_status(StatusCode::OK);
    for base in ["/v1/channels", "/v1/e"] {
        server
            .get(&format!("{base}/{id}/fcp"))
            .await
            .assert_status(StatusCode::OK);
    }
    // Existing callers may keep their budget subject spelling; storage and
    // enforcement always use the channel subject after migration.
    let budget: Value = server
        .post(
            "/v1/budgets",
            json!({
                "subject_type": "agent_endpoint", "subject_id": id, "limit": 23.0,
                "currency": "usd"
            }),
        )
        .await
        .assert_status(StatusCode::CREATED)
        .json();
    assert_eq!(budget["subject_type"], "agent_channel");
    let stored: String =
        sqlx::query_scalar("SELECT subject_type FROM budgets WHERE subject_id = $1")
            .bind(id)
            .fetch_one(&server.pool)
            .await
            .unwrap();
    assert_eq!(stored, "agent_channel");

    server
        .post(&format!("{canonical}/{id}/unpublish"), json!({}))
        .await
        .assert_status(StatusCode::OK);
    for base in ["/v1/channels", "/v1/e"] {
        server
            .get(&format!("{base}/{id}/fcp"))
            .await
            .assert_status(StatusCode::NOT_FOUND);
    }
    server
        .delete(&format!("{legacy}/{id}"))
        .await
        .assert_status(StatusCode::OK);
    server
        .get(&format!("{canonical}/{id}"))
        .await
        .assert_status(StatusCode::NOT_FOUND);
}
