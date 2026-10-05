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
    assert!(
        environment["sandbox_id"]
            .as_str()
            .unwrap()
            .starts_with("sandbox_")
    );
    assert_eq!(environment["id"], environment["sandbox_id"]);
    assert_eq!(environment["role"], "primary");
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

#[tokio::test]
async fn execution_harness_pins_managed_bashkit_environment() {
    let server = TestServer::in_memory().await;

    let session: Value = server
        .post(
            "/v1/sessions",
            json!({
                "harness_id": server.seed_generic_harness_id,
                "title": "Environment smoke",
            }),
        )
        .await
        .assert_status(StatusCode::CREATED)
        .json();
    let session_id = session["id"].as_str().expect("session id");

    let environment: Value = server
        .get(&format!("/v1/sessions/{session_id}/environment"))
        .await
        .assert_status(StatusCode::OK)
        .json();

    // Execution-capable Harnesses without an Agent or Session override pin the
    // managed Bashkit Environment into a durable primary Sandbox.
    assert!(
        environment["sandbox_id"]
            .as_str()
            .unwrap()
            .starts_with("sandbox_")
    );
    assert_eq!(environment["id"], environment["sandbox_id"]);
    assert!(
        environment["environment_revision_id"]
            .as_str()
            .unwrap()
            .starts_with("envrev_")
    );
    assert_eq!(environment["role"], "primary");
    assert_eq!(environment["name"], "bashkit-virtual-workspace");
    assert_eq!(environment["target"]["kind"], "vfs");
    assert_eq!(environment["target"]["provider"], "bashkit");

    // The load-bearing claim: a caller learns a build cannot run here before
    // running one, rather than from a confusing tool error afterwards.
    assert_eq!(environment["capabilities"]["native_processes"], false);
    assert_eq!(environment["capabilities"]["portable_checkpoint"], true);
    assert_eq!(environment["containment"]["level"], "isolated");
    assert_eq!(environment["durability"], "checkpointed");

    assert_eq!(environment["resolved_from"], "profile");
    assert_eq!(environment["desired_state"], "ready");
    assert_eq!(environment["observed_state"], "absent");
    assert_eq!(environment["generation"], 1);
}
