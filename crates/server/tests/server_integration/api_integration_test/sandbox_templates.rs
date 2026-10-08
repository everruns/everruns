//! API integration tests for first-class Sandbox Templates.

use crate::test_harness::TestServer;
use axum::http::StatusCode;
use serde_json::{Value, json};

#[tokio::test]
async fn raw_sandbox_bindings_are_rejected_on_agent_harness_and_session_writes() {
    let server = TestServer::in_memory().await;
    let agent: Value = server
        .post(
            "/v1/agents",
            json!({
                "name": "sandbox-boundary", "system_prompt": "Test",
            }),
        )
        .await
        .assert_status(StatusCode::CREATED)
        .json();
    let harness: Value = server
        .post(
            "/v1/harnesses",
            json!({
                "name": "sandbox-boundary", "system_prompt": "Test",
            }),
        )
        .await
        .assert_status(StatusCode::CREATED)
        .json();
    for config in [
        json!({"provider": "daytona", "credential": {
            "source": "session_user", "virtual_user_id": uuid::Uuid::new_v4()
        }}),
        json!({"provider": "daytona", "provider_config": {
            "api_base": "https://attacker.invalid", "toolbox_base": "https://attacker.invalid"
        }}),
    ] {
        let capabilities = json!([{"ref": "session_sandbox", "config": config}]);
        server
            .post(
                "/v1/agents",
                json!({
                    "name": "forged", "system_prompt": "Test", "capabilities": capabilities,
                }),
            )
            .await
            .assert_status(StatusCode::BAD_REQUEST);
        server
            .patch(
                &format!("/v1/agents/{}", agent["id"].as_str().unwrap()),
                json!({
                    "capabilities": capabilities,
                }),
            )
            .await
            .assert_status(StatusCode::BAD_REQUEST);
        server
            .post(
                "/v1/harnesses",
                json!({
                    "name": "forged", "system_prompt": "Test", "capabilities": capabilities,
                }),
            )
            .await
            .assert_status(StatusCode::BAD_REQUEST);
        server
            .patch(
                &format!("/v1/harnesses/{}", harness["id"].as_str().unwrap()),
                json!({
                    "capabilities": capabilities,
                }),
            )
            .await
            .assert_status(StatusCode::BAD_REQUEST);
        server
            .post(
                "/v1/sessions",
                json!({
                    "harness_id": server.seed_base_harness_id,
                    "agent_id": agent["id"], "capabilities": capabilities,
                }),
            )
            .await
            .assert_status(StatusCode::BAD_REQUEST);
    }
}

#[tokio::test]
async fn selected_template_is_pinned_and_reported() {
    let server = TestServer::in_memory().await;
    let agent: Value = server
        .post(
            "/v1/agents",
            json!({
                "name": "sandbox-template-agent",
                "system_prompt": "Test Sandbox Templates.",
                "sandbox_policy": {
                    "mode": "selectable",
                    "default": "scratch",
                    "templates": {
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
                "title": "Inherited Sandbox"
            }),
        )
        .await
        .assert_status(StatusCode::CREATED)
        .json();
    let inherited_id = inherited["id"].as_str().expect("session id");
    let sandbox: Value = server
        .get(&format!("/v1/sessions/{inherited_id}/sandbox"))
        .await
        .assert_status(StatusCode::OK)
        .json();
    assert!(
        sandbox["sandbox_id"]
            .as_str()
            .unwrap()
            .starts_with("sandbox_")
    );
    assert!(sandbox.get("id").is_none());
    assert_eq!(sandbox["role"], "primary");
    assert_eq!(sandbox["name"], "scratch");
    assert_eq!(sandbox["resolved_from"], "spec");
    assert_eq!(sandbox["target"]["kind"], "vfs");
    assert_eq!(sandbox["observed_state"], "absent");
    assert_eq!(sandbox["generation"], 1);

    // Editing the Agent changes only future Sessions. The first Session keeps
    // its immutable snapshot rather than following the editable Agent head.
    server
        .patch(
            &format!("/v1/agents/{}", agent["id"].as_str().unwrap()),
            json!({
                "sandbox_policy": {
                    "mode": "fixed",
                    "default": "build",
                    "templates": {
                        "build": {"target": {"kind": "managed", "provider": "daytona"}, "durability": "checkpointed"}
                    }
                }
            }),
        )
        .await
        .assert_status(StatusCode::OK);
    let still_pinned: Value = server
        .get(&format!("/v1/sessions/{inherited_id}/sandbox"))
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
                "sandbox": {"use": "missing"},
                "title": "Invalid Sandbox binding"
            }),
        )
        .await
        .assert_status(StatusCode::BAD_REQUEST);
}

#[tokio::test]
async fn execution_harness_pins_managed_bashkit_sandbox() {
    let server = TestServer::in_memory().await;

    let session: Value = server
        .post(
            "/v1/sessions",
            json!({
                "harness_id": server.seed_generic_harness_id,
                "title": "Sandbox smoke",
            }),
        )
        .await
        .assert_status(StatusCode::CREATED)
        .json();
    let session_id = session["id"].as_str().expect("session id");

    let sandbox: Value = server
        .get(&format!("/v1/sessions/{session_id}/sandbox"))
        .await
        .assert_status(StatusCode::OK)
        .json();

    // Execution-capable Harnesses without an Agent or Session override pin the
    // managed Bashkit template into a durable primary Sandbox.
    assert!(
        sandbox["sandbox_id"]
            .as_str()
            .unwrap()
            .starts_with("sandbox_")
    );
    assert!(sandbox.get("id").is_none());
    assert!(
        sandbox["sandbox_template_revision_id"]
            .as_str()
            .unwrap()
            .starts_with("sbxtplrev_")
    );
    assert_eq!(sandbox["role"], "primary");
    assert_eq!(sandbox["name"], "bashkit-virtual-workspace");
    assert_eq!(sandbox["target"]["kind"], "vfs");
    assert_eq!(sandbox["target"]["provider"], "bashkit");

    // The load-bearing claim: a caller learns a build cannot run here before
    // running one, rather than from a confusing tool error afterwards.
    assert_eq!(sandbox["capabilities"]["native_processes"], false);
    assert_eq!(sandbox["capabilities"]["portable_checkpoint"], true);
    assert_eq!(sandbox["containment"]["level"], "isolated");
    assert_eq!(sandbox["durability"], "checkpointed");

    assert_eq!(sandbox["resolved_from"], "spec");
    assert_eq!(sandbox["desired_state"], "ready");
    assert_eq!(sandbox["observed_state"], "absent");
    assert_eq!(sandbox["generation"], 1);
}

#[tokio::test]
async fn legacy_environment_authoring_is_accepted_but_response_is_canonical() {
    let server = TestServer::in_memory().await;

    let template: Value = server
        .post(
            "/v1/environments",
            json!({
                "name": "legacy-bashkit",
                "display_name": "Legacy Bashkit",
                "profile": {"target": {"kind": "vfs", "provider": "bashkit"}}
            }),
        )
        .await
        .assert_status(StatusCode::OK)
        .json();

    assert!(template["id"].as_str().unwrap().starts_with("sbxtpl_"));
    assert!(
        template["current_revision"]["id"]
            .as_str()
            .unwrap()
            .starts_with("sbxtplrev_")
    );
    assert!(template["current_revision"].get("spec").is_some());
    assert!(template["current_revision"].get("profile").is_none());
}

/// The fleet endpoints list a Session's managed Sandbox with its owner, roll it
/// up, and show its lifecycle; in-process Sandboxes stay out by default.
#[tokio::test]
async fn sandbox_fleet_lists_rolls_up_and_shows_history() {
    let server = TestServer::new().await;
    let agent: Value = server
        .post(
            "/v1/agents",
            json!({
                "name": "fleet-agent",
                "system_prompt": "Test the Sandbox fleet.",
                "sandbox_policy": {
                    "mode": "fixed",
                    "default": "build",
                    "templates": {
                        "build": {"target": {"kind": "managed", "provider": "daytona"}, "durability": "checkpointed"}
                    }
                }
            }),
        )
        .await
        .assert_status(StatusCode::CREATED)
        .json();
    let title = format!("Fleet {}", uuid::Uuid::now_v7());
    let session: Value = server
        .post(
            "/v1/sessions",
            json!({
                "harness_id": server.seed_generic_harness_id,
                "agent_id": agent["id"],
                "title": title,
            }),
        )
        .await
        .assert_status(StatusCode::CREATED)
        .json();
    let pinned: Value = server
        .get(&format!(
            "/v1/sessions/{}/sandbox",
            session["id"].as_str().unwrap()
        ))
        .await
        .assert_status(StatusCode::OK)
        .json();
    let sandbox_id = pinned["sandbox_id"].as_str().expect("sandbox id");

    let page: Value = server
        .get(&format!(
            "/v1/sandboxes?search={}",
            title.replace(' ', "%20")
        ))
        .await
        .assert_status(StatusCode::OK)
        .json();
    assert_eq!(page["total"], 1, "{page}");
    let item = &page["items"][0];
    assert_eq!(item["id"], sandbox_id);
    assert_eq!(item["provider"], "daytona");
    assert_eq!(item["state"], "not_started");
    assert_eq!(item["session_id"], session["id"]);
    assert_eq!(item["agent_id"], agent["id"]);
    assert_eq!(item["agent_name"], "fleet-agent");

    let stats: Value = server
        .get(&format!(
            "/v1/sandboxes/stats?search={}",
            title.replace(' ', "%20")
        ))
        .await
        .assert_status(StatusCode::OK)
        .json();
    assert_eq!(stats["window_days"], 7);
    assert_eq!(
        stats["by_state"],
        json!([{"key": "not_started", "count": 1}])
    );
    assert_eq!(stats["created_in_window"], 1);

    let detail: Value = server
        .get(&format!("/v1/sandboxes/{sandbox_id}"))
        .await
        .assert_status(StatusCode::OK)
        .json();
    assert_eq!(detail["id"], sandbox_id);
    assert_eq!(detail["history"][0]["state"], "not_started");
    assert_eq!(detail["incarnations"], json!([]));

    // Never started, so nothing to draw.
    let timeline: Value = server
        .get(&format!(
            "/v1/sandboxes/timeline?search={}",
            title.replace(' ', "%20")
        ))
        .await
        .assert_status(StatusCode::OK)
        .json();
    assert_eq!(timeline["lanes"], json!([]));

    server
        .get("/v1/sandboxes?state=runing")
        .await
        .assert_status(StatusCode::BAD_REQUEST);
    server
        .get("/v1/sandboxes/sandbox_01933b5a000070008000000000000099")
        .await
        .assert_status(StatusCode::NOT_FOUND);
}
