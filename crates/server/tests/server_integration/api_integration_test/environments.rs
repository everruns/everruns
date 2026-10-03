//! API integration tests for first-class execution environments.

use crate::test_harness::TestServer;
use axum::http::StatusCode;
use serde_json::{Value, json};

#[tokio::test]
async fn selected_profile_is_pinned_and_reported() {
    let server = TestServer::in_memory().await;
    let agent: Value = server
        .post(
            "/v1/agents",
            json!({
                "name": "profiled-environment-agent",
                "system_prompt": "Test environment profiles.",
                "environments": {
                    "default": "scratch",
                    "profiles": {
                        "scratch": {"target": {"kind": "vfs", "provider": "bashkit"}},
                        "build": {"target": {"kind": "managed", "provider": "daytona"}, "durability": "checkpointed"}
                    }
                }
            }),
        )
        .await
        .assert_status(StatusCode::CREATED)
        .json();
    let inherited: Value = server
        .post(
            "/v1/sessions",
            json!({
                "harness_id": server.seed_generic_harness_id,
                "agent_id": agent["id"],
                "title": "Inherited profile"
            }),
        )
        .await
        .assert_status(StatusCode::CREATED)
        .json();
    let inherited_id = inherited["id"].as_str().expect("session id");
    let environment: Value = server
        .get(&format!("/v1/sessions/{inherited_id}/environment"))
        .await
        .assert_status(StatusCode::OK)
        .json();
    assert!(environment["id"].as_str().unwrap().starts_with("env_"));
    assert_eq!(environment["name"], "scratch");
    assert_eq!(environment["resolved_from"], "profile");
    assert_eq!(environment["target"]["kind"], "vfs");
    assert_eq!(environment["observed_state"], "absent");
    assert_eq!(environment["generation"], 1);

    // Editing the Agent changes only future Sessions. The first Session keeps
    // its immutable snapshot rather than following the editable Agent head.
    server
        .patch(
            &format!("/v1/agents/{}", agent["id"].as_str().unwrap()),
            json!({
                "environments": {
                    "default": "build",
                    "profiles": {
                        "build": {"target": {"kind": "managed", "provider": "daytona"}, "durability": "checkpointed"}
                    }
                }
            }),
        )
        .await
        .assert_status(StatusCode::OK);
    let still_pinned: Value = server
        .get(&format!("/v1/sessions/{inherited_id}/environment"))
        .await
        .assert_status(StatusCode::OK)
        .json();
    assert_eq!(still_pinned["name"], "scratch");
    assert_eq!(still_pinned["target"]["kind"], "vfs");

    server
        .post(
            "/v1/sessions",
            json!({
                "harness_id": server.seed_generic_harness_id,
                "agent_id": agent["id"],
                "environment": {"use": "missing"},
                "title": "Invalid profile"
            }),
        )
        .await
        .assert_status(StatusCode::BAD_REQUEST);
}
