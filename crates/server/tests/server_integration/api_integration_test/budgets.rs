//! Budgets are served by the generic `#[command(http = ..)]` handler. These
//! pin the response shapes the hand-written handlers had: 201 on create, bare
//! JSON arrays (not `{"data": [...]}`) on list routes, URL-decorated budgets,
//! and 204 on delete.

use crate::test_harness::TestServer;
use axum::http::StatusCode;
use serde_json::{Value, json};

#[tokio::test]
async fn budget_routes_keep_their_response_shapes() {
    let server = TestServer::new().await;
    // A real session: top-ups journal against the budget's session.
    let session: Value = server
        .post(
            "/v1/sessions",
            json!({"harness_id": server.seed_base_harness_id, "title": "Budget routes"}),
        )
        .await
        .assert_status(StatusCode::CREATED)
        .json();
    let session_id = session["id"].as_str().unwrap().to_string();

    let budget: Value = server
        .post(
            "/v1/budgets",
            json!({
                "subject_type": "session", "subject_id": session_id, "limit": 10.0,
                "soft_limit": 5.0, "currency": "usd"
            }),
        )
        .await
        .assert_status(StatusCode::CREATED)
        .json();
    let id = budget["id"].as_str().unwrap().to_string();
    assert!(budget["self_url"].as_str().is_some(), "{budget}");

    let listed: Vec<Value> = server
        .get(&format!(
            "/v1/budgets?subject_type=session&subject_id={session_id}"
        ))
        .await
        .assert_status(StatusCode::OK)
        .json();
    assert_eq!(listed.len(), 1);
    assert_eq!(listed[0]["id"], id.as_str());
    assert!(listed[0]["self_url"].as_str().is_some());

    let fetched: Value = server
        .get(&format!("/v1/budgets/{id}"))
        .await
        .assert_status(StatusCode::OK)
        .json();
    assert_eq!(fetched["id"], id.as_str());

    let updated: Value = server
        .patch(&format!("/v1/budgets/{id}"), json!({"limit": 20.0}))
        .await
        .assert_status(StatusCode::OK)
        .json();
    assert_eq!(updated["limit"], 20.0);

    let topped: Value = server
        .post(&format!("/v1/budgets/{id}/top-up"), json!({"amount": 1.5}))
        .await
        .assert_status(StatusCode::OK)
        .json();
    assert_eq!(topped["id"], id.as_str());

    let ledger: Vec<Value> = server
        .get(&format!("/v1/budgets/{id}/ledger?limit=10"))
        .await
        .assert_status(StatusCode::OK)
        .json();
    assert!(!ledger.is_empty(), "top-up should be in the ledger");

    let check: Value = server
        .get(&format!("/v1/budgets/{id}/check"))
        .await
        .assert_status(StatusCode::OK)
        .json();
    assert!(check.is_object(), "{check}");

    let session_budgets: Vec<Value> = server
        .get(&format!("/v1/sessions/{session_id}/budgets"))
        .await
        .assert_status(StatusCode::OK)
        .json();
    assert_eq!(session_budgets.len(), 1);
    assert!(session_budgets[0]["self_url"].as_str().is_some());

    server
        .get(&format!("/v1/sessions/{session_id}/budget-check"))
        .await
        .assert_status(StatusCode::OK);
    let resumed: Value = server
        .post(&format!("/v1/sessions/{session_id}/resume"), json!({}))
        .await
        .assert_status(StatusCode::OK)
        .json();
    assert_eq!(resumed["session_id"], session_id.as_str());

    server
        .delete(&format!("/v1/budgets/{id}"))
        .await
        .assert_status(StatusCode::NO_CONTENT);
}

#[tokio::test]
async fn budget_routes_keep_their_error_statuses() {
    let server = TestServer::new().await;

    server
        .post(
            "/v1/budgets",
            json!({"subject_type": "planet", "subject_id": "x", "limit": 1.0, "currency": "usd"}),
        )
        .await
        .assert_status(StatusCode::BAD_REQUEST);
    server
        .get(&format!("/v1/budgets/{}", uuid::Uuid::new_v4()))
        .await
        .assert_status(StatusCode::NOT_FOUND);
}
