//! Entity history and manager context end to end, over REST and
//! `/v1/commands`, on PostgreSQL: the reason header and envelope field reach
//! history, and context writes hold to their revision.
//!
//! Run with: cargo test -p everruns-server --test domain change_history_test::

use crate::test_harness;

use axum::http::{Method, StatusCode};
use serde_json::{Value, json};
use test_harness::TestServer;

fn unique(prefix: &str) -> String {
    format!(
        "{prefix}-{}",
        uuid::Uuid::now_v7().simple().to_string()[20..].to_owned()
    )
}

async fn send(
    server: &TestServer,
    method: Method,
    uri: &str,
    headers: Vec<(&str, &str)>,
    body: Value,
) -> test_harness::TestResponse {
    let mut headers = headers;
    headers.push(("content-type", "application/json"));
    server
        .request_raw(method, uri, headers, serde_json::to_vec(&body).unwrap())
        .await
}

#[tokio::test]
async fn reasons_from_rest_and_commands_land_in_history_and_context_holds_its_revision() {
    let server = TestServer::new().await;

    // /v1/commands: the reason rides in the envelope.
    let created: Value = server
        .post(
            "/v1/commands/create_workspace",
            json!({ "params": { "name": unique("history") }, "reason": "a home for support" }),
        )
        .await
        .assert_success()
        .json();
    let id = created["output"]["id"]
        .as_str()
        .expect("workspace id")
        .to_string();

    let history: Value = server
        .get(&format!("/v1/history/{id}"))
        .await
        .assert_success()
        .json();
    assert_eq!(history[0]["action"], "created");
    assert_eq!(history[0]["reason"], "a home for support");
    assert_eq!(history[0]["surface"], "commands");

    // Context: write, append, refuse a stale write.
    // REST: the reason travels percent-encoded in a header.
    let set: Value = send(
        &server,
        Method::PUT,
        &format!("/v1/context/{id}"),
        vec![("everruns-change-reason", "decided%20in%20review")],
        json!({ "content": "Owned by support.", "expected_revision": 0 }),
    )
    .await
    .assert_success()
    .json();
    assert_eq!(set["revision"], 1);
    let appended: Value = server
        .post(
            &format!("/v1/context/{id}/append"),
            json!({ "text": "Do not rename." }),
        )
        .await
        .assert_success()
        .json();
    assert_eq!(appended["revision"], 2);
    let stale: Value = server
        .put(
            &format!("/v1/context/{id}"),
            json!({ "content": "x", "expected_revision": 1 }),
        )
        .await
        .assert_status(StatusCode::CONFLICT)
        .json();
    assert_eq!(stale["code"], "manager_context_changed");

    // /v1/commands: a stale acknowledgement is refused, a missing one warns.
    let refused: Value = server
        .post(
            "/v1/commands/update_workspace",
            json!({
                "params": { "workspace_id": id, "name": unique("renamed") },
                "context_revision": 1,
                "reason": "rename",
            }),
        )
        .await
        .assert_status(StatusCode::CONFLICT)
        .json();
    assert_eq!(refused["code"], "manager_context_changed");
    let warned: Value = server
        .post(
            "/v1/commands/update_workspace",
            json!({
                "params": { "workspace_id": id, "name": unique("renamed") },
                "reason": "rename as agreed",
            }),
        )
        .await
        .assert_success()
        .json();
    let warnings = warned["warnings"].as_array().expect("warnings");
    assert!(
        warnings.iter().any(|warning| warning
            .as_str()
            .unwrap_or_default()
            .contains("--context-revision 2")),
        "{warnings:?}"
    );
    server
        .post(
            "/v1/commands/update_workspace",
            json!({
                "params": { "workspace_id": id, "name": unique("renamed") },
                "context_revision": 2,
            }),
        )
        .await
        .assert_success();

    let history: Value = server
        .get(&format!("/v1/history/{id}"))
        .await
        .assert_success()
        .json();
    let actions: Vec<&str> = history
        .as_array()
        .unwrap()
        .iter()
        .map(|entry| entry["action"].as_str().unwrap())
        .collect();
    assert_eq!(
        actions,
        [
            "updated",
            "updated",
            "context_updated",
            "context_updated",
            "created"
        ]
    );
    assert_eq!(history[1]["reason"], "rename as agreed");
    assert_eq!(history[1]["surface"], "commands");
    assert_eq!(history[3]["reason"], "decided in review");
    assert_eq!(history[3]["surface"], "api");

    // Deleting the entity removes its context; history stays.
    server
        .post(
            "/v1/commands/delete_workspace",
            json!({ "params": { "workspace_id": id } }),
        )
        .await
        .assert_success();
    let org: Value = server
        .get("/v1/history?kind=workspace&action=deleted")
        .await
        .assert_success()
        .json();
    assert!(
        org.as_array()
            .unwrap()
            .iter()
            .any(|entry| entry["entity_ref"] == id.as_str())
    );
}
