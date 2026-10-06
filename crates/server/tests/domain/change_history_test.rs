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

#[tokio::test]
async fn rest_routes_that_skip_commands_still_record_history_and_hold_to_context() {
    let server = TestServer::new().await;

    // POST /v1/workspaces writes storage directly; the header reason still lands.
    let created: Value = send(
        &server,
        Method::POST,
        "/v1/workspaces",
        vec![("everruns-change-reason", "rest%20create")],
        json!({ "name": unique("rest") }),
    )
    .await
    .assert_success()
    .json();
    let id = created["id"].as_str().expect("workspace id").to_string();

    server
        .put(
            &format!("/v1/context/{id}"),
            json!({ "content": "Frozen until review.", "expected_revision": 0 }),
        )
        .await
        .assert_success();

    // A stale acknowledgement is refused before the write.
    let refused: Value = send(
        &server,
        Method::PATCH,
        &format!("/v1/workspaces/{id}"),
        vec![("everruns-context-revision", "7")],
        json!({ "name": unique("stale") }),
    )
    .await
    .assert_status(StatusCode::CONFLICT)
    .json();
    assert_eq!(refused["code"], "manager_context_changed");

    send(
        &server,
        Method::PATCH,
        &format!("/v1/workspaces/{id}"),
        vec![
            ("everruns-change-reason", "acked%20rename"),
            ("everruns-context-revision", "1"),
        ],
        json!({ "name": unique("renamed") }),
    )
    .await
    .assert_success();

    let history: Value = server
        .get(&format!("/v1/history/{id}"))
        .await
        .assert_success()
        .json();
    let commands: Vec<&str> = history
        .as_array()
        .expect("history")
        .iter()
        .map(|entry| entry["command"].as_str().unwrap_or_default())
        .collect();
    assert_eq!(
        commands,
        [
            "update_workspace",
            "set_manager_context",
            "create_workspace"
        ]
    );
    assert_eq!(history[0]["reason"], "acked rename");
    assert_eq!(history[0]["changed_fields"], json!(["name"]));
    assert_eq!(history[0]["surface"], "api");
    assert_eq!(history[2]["reason"], "rest create");

    // Archiving drops the workspace's manager context.
    server
        .delete(&format!("/v1/workspaces/{id}"))
        .await
        .assert_status(StatusCode::NO_CONTENT);
}

#[tokio::test]
async fn an_agent_restored_to_an_earlier_revision_runs_as_a_recorded_change() {
    let server = TestServer::new().await;
    let agent: Value = server
        .post(
            "/v1/agents",
            json!({ "name": unique("restore"), "system_prompt": "Answer in English." }),
        )
        .await
        .assert_status(StatusCode::CREATED)
        .json();
    let id = agent["id"].as_str().expect("agent id").to_string();
    server
        .patch(
            &format!("/v1/agents/{id}"),
            json!({ "system_prompt": "Answer in French.", "description": "Paris office" }),
        )
        .await
        .assert_success();

    let diff: Value = server
        .get(&format!("/v1/history/{id}/diff?from=1"))
        .await
        .assert_success()
        .json();
    let fields: Vec<&str> = diff
        .as_array()
        .expect("diff")
        .iter()
        .filter_map(|change| change["field"].as_str())
        .collect();
    assert!(fields.contains(&"system_prompt"), "{diff}");
    assert!(fields.contains(&"description"), "{diff}");

    let restored: Value = send(
        &server,
        Method::POST,
        &format!("/v1/history/{id}/restore"),
        vec![("everruns-change-reason", "French%20was%20a%20mistake")],
        json!({ "revision": 1 }),
    )
    .await
    .assert_success()
    .json();
    assert_eq!(restored["restored_revision"], 1);
    // `update_agent` cannot clear a description, so the restore says so
    // rather than claiming the revision is back in full.
    let warnings = restored["warnings"].to_string();
    assert!(
        warnings.contains("description could not be set back"),
        "{warnings}"
    );

    let now: Value = server
        .get(&format!("/v1/agents/{id}"))
        .await
        .assert_success()
        .json();
    assert_eq!(now["system_prompt"], "Answer in English.");

    let history: Value = server
        .get(&format!("/v1/history/{id}"))
        .await
        .assert_success()
        .json();
    assert_eq!(history[0]["action"], "restored");
    assert_eq!(history[0]["restored_from_revision"], 1);
    assert_eq!(history[0]["revision"], 3);
    assert_eq!(history[0]["reason"], "French was a mistake");

    let shown: Value = server
        .get(&format!("/v1/history/{id}/revisions/2"))
        .await
        .assert_success()
        .json();
    assert_eq!(shown["snapshot"]["system_prompt"], "Answer in French.");
}
