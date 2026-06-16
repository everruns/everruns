//! Observer CRUD and validation integration tests.
//!
//! End-to-end scoring (match → enqueue → worker → score) is covered by unit
//! tests in `domains::observers::worker`. These tests exercise the public API
//! surface: create/list/get/update/delete, validation, and score listing.

mod test_harness;

use axum::http::StatusCode;
use serde_json::json;
use test_harness::TestServer;

#[tokio::test]
async fn create_and_get_observer_roundtrip() {
    let server = TestServer::in_memory().await;

    let created = server
        .post(
            "/v1/observers",
            json!({
                "name": "Support quality",
                "description": "Watch the support agent",
                "sampling_rate": 0.25,
                "scorers": [
                    { "key": "has_greeting", "method": "rule", "rule": { "type": "contains", "text": "hello" } }
                ]
            }),
        )
        .await
        .assert_status(StatusCode::CREATED)
        .json_value();

    let id = created["id"].as_str().unwrap().to_string();
    assert!(id.starts_with("observer_"));
    assert_eq!(created["name"], "Support quality");
    assert_eq!(created["sampling_rate"], 0.25);
    assert_eq!(created["status"], "active");
    assert_eq!(created["scorers"][0]["key"], "has_greeting");
    assert_eq!(created["scorers"][0]["scope"], "turn");

    let fetched = server
        .get(&format!("/v1/observers/{id}"))
        .await
        .assert_status(StatusCode::OK)
        .json_value();
    assert_eq!(fetched["id"], id);
    assert_eq!(fetched["description"], "Watch the support agent");
}

#[tokio::test]
async fn sampling_rate_defaults_to_ten_percent() {
    let server = TestServer::in_memory().await;
    let created = server
        .post(
            "/v1/observers",
            json!({
                "name": "Default sampling",
                "scorers": [
                    { "key": "k", "method": "rule", "rule": { "type": "contains", "text": "x" } }
                ]
            }),
        )
        .await
        .assert_status(StatusCode::CREATED)
        .json_value();
    assert_eq!(created["sampling_rate"], 0.1);
}

#[tokio::test]
async fn list_observers_excludes_archived_by_default() {
    let server = TestServer::in_memory().await;
    let created = server
        .post(
            "/v1/observers",
            json!({
                "name": "Temp",
                "scorers": [{ "key": "k", "method": "rule", "rule": { "type": "contains", "text": "x" } }]
            }),
        )
        .await
        .assert_status(StatusCode::CREATED)
        .json_value();
    let id = created["id"].as_str().unwrap().to_string();

    let listed = server
        .get("/v1/observers")
        .await
        .assert_status(StatusCode::OK)
        .json_value();
    assert_eq!(listed["data"].as_array().unwrap().len(), 1);

    server
        .delete(&format!("/v1/observers/{id}"))
        .await
        .assert_status(StatusCode::NO_CONTENT);

    let after = server
        .get("/v1/observers")
        .await
        .assert_status(StatusCode::OK)
        .json_value();
    assert_eq!(after["data"].as_array().unwrap().len(), 0);

    let with_archived = server
        .get("/v1/observers?include_archived=true")
        .await
        .assert_status(StatusCode::OK)
        .json_value();
    assert_eq!(with_archived["data"].as_array().unwrap().len(), 1);
    assert_eq!(with_archived["data"][0]["status"], "archived");
}

#[tokio::test]
async fn update_observer_changes_sampling_and_pause() {
    let server = TestServer::in_memory().await;
    let created = server
        .post(
            "/v1/observers",
            json!({
                "name": "Adjustable",
                "scorers": [{ "key": "k", "method": "rule", "rule": { "type": "contains", "text": "x" } }]
            }),
        )
        .await
        .assert_status(StatusCode::CREATED)
        .json_value();
    let id = created["id"].as_str().unwrap().to_string();

    let updated = server
        .patch(
            &format!("/v1/observers/{id}"),
            json!({ "sampling_rate": 0.5, "status": "paused" }),
        )
        .await
        .assert_status(StatusCode::OK)
        .json_value();
    assert_eq!(updated["sampling_rate"], 0.5);
    assert_eq!(updated["status"], "paused");
}

#[tokio::test]
async fn rejects_invalid_sampling_rate() {
    let server = TestServer::in_memory().await;
    server
        .post(
            "/v1/observers",
            json!({
                "name": "Bad",
                "sampling_rate": 1.5,
                "scorers": [{ "key": "k", "method": "rule", "rule": { "type": "contains", "text": "x" } }]
            }),
        )
        .await
        .assert_status(StatusCode::BAD_REQUEST);
}

#[tokio::test]
async fn rejects_empty_scorers() {
    let server = TestServer::in_memory().await;
    server
        .post("/v1/observers", json!({ "name": "Empty", "scorers": [] }))
        .await
        .assert_status(StatusCode::BAD_REQUEST);
}

#[tokio::test]
async fn rejects_duplicate_scorer_keys() {
    let server = TestServer::in_memory().await;
    server
        .post(
            "/v1/observers",
            json!({
                "name": "Dupe",
                "scorers": [
                    { "key": "k", "method": "rule", "rule": { "type": "contains", "text": "a" } },
                    { "key": "k", "method": "rule", "rule": { "type": "contains", "text": "b" } }
                ]
            }),
        )
        .await
        .assert_status(StatusCode::BAD_REQUEST);
}

#[tokio::test]
async fn rejects_file_contains_scorer() {
    let server = TestServer::in_memory().await;
    server
        .post(
            "/v1/observers",
            json!({
                "name": "File",
                "scorers": [
                    { "key": "f", "method": "rule", "rule": { "type": "file_contains", "path": "/x", "text": "y" } }
                ]
            }),
        )
        .await
        .assert_status(StatusCode::BAD_REQUEST);
}

#[tokio::test]
async fn creates_llm_judge_scorer() {
    let server = TestServer::in_memory().await;
    let created = server
        .post(
            "/v1/observers",
            json!({
                "name": "Quality",
                "scorers": [
                    {
                        "key": "completeness",
                        "method": "llm_judge",
                        "rubric": "Score 1.0 if the answer fully addresses the question.",
                        "pass_threshold": 0.7
                    }
                ]
            }),
        )
        .await
        .assert_status(StatusCode::CREATED)
        .json_value();
    assert_eq!(created["scorers"][0]["method"], "llm_judge");
    assert_eq!(created["scorers"][0]["pass_threshold"], 0.7);
    assert_eq!(created["scorers"][0]["key"], "completeness");
}

#[tokio::test]
async fn rejects_llm_judge_with_empty_rubric() {
    let server = TestServer::in_memory().await;
    server
        .post(
            "/v1/observers",
            json!({
                "name": "Bad judge",
                "scorers": [{ "key": "j", "method": "llm_judge", "rubric": "  " }]
            }),
        )
        .await
        .assert_status(StatusCode::BAD_REQUEST);
}

#[tokio::test]
async fn scores_endpoint_returns_empty_for_new_observer() {
    let server = TestServer::in_memory().await;
    let created = server
        .post(
            "/v1/observers",
            json!({
                "name": "Fresh",
                "scorers": [{ "key": "k", "method": "rule", "rule": { "type": "contains", "text": "x" } }]
            }),
        )
        .await
        .assert_status(StatusCode::CREATED)
        .json_value();
    let id = created["id"].as_str().unwrap().to_string();

    let scores = server
        .get(&format!("/v1/observers/{id}/scores"))
        .await
        .assert_status(StatusCode::OK)
        .json_value();
    assert_eq!(scores["data"].as_array().unwrap().len(), 0);
}

#[tokio::test]
async fn get_missing_observer_is_404() {
    let server = TestServer::in_memory().await;
    server
        .get("/v1/observers/observer_01933b5a000070008000000000000999")
        .await
        .assert_status(StatusCode::NOT_FOUND);
}
