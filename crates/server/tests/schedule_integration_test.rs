//! Integration tests for durable scheduled tasks API
//!
//! Tests the full schedule lifecycle: CRUD, manual trigger, pause/resume,
//! execution history, and the scheduler background loop processing due schedules.
//!
//! Run with: cargo test -p everruns-server --test schedule_integration_test -- --test-threads=1

mod test_harness;

use axum::http::StatusCode;
use serde_json::{Value, json};
use test_harness::TestServer;

// --------------------------------------------
// Schedule CRUD Tests
// --------------------------------------------

#[tokio::test]
async fn test_create_schedule() {
    let server = TestServer::new().await;

    let body: Value = server
        .post(
            "/v1/durable/schedules",
            json!({
                "name": "test-create",
                "cron_expression": "0 * * * * * *",
                "target": {
                    "type": "workflow",
                    "name": "test-workflow",
                    "input": {"key": "value"}
                }
            }),
        )
        .await
        .assert_status(StatusCode::CREATED)
        .json();

    assert_eq!(body["name"], "test-create");
    assert_eq!(body["cron_expression"], "0 * * * * * *");
    assert_eq!(body["target"]["type"], "workflow");
    assert_eq!(body["target"]["name"], "test-workflow");
    assert!(body["enabled"].as_bool().unwrap());
    assert!(body["id"].as_str().is_some());
    assert!(body["next_trigger_at"].as_str().is_some());
}

#[tokio::test]
async fn test_create_schedule_invalid_cron() {
    let server = TestServer::new().await;

    server
        .post(
            "/v1/durable/schedules",
            json!({
                "name": "bad-cron",
                "cron_expression": "not-valid",
                "target": {
                    "type": "workflow",
                    "name": "test-workflow"
                }
            }),
        )
        .await
        .assert_status(StatusCode::BAD_REQUEST);
}

#[tokio::test]
async fn test_create_schedule_invalid_target_type() {
    let server = TestServer::new().await;

    server
        .post(
            "/v1/durable/schedules",
            json!({
                "name": "bad-target",
                "cron_expression": "0 * * * * * *",
                "target": {
                    "type": "invalid",
                    "name": "test-workflow"
                }
            }),
        )
        .await
        .assert_status(StatusCode::BAD_REQUEST);
}

#[tokio::test]
async fn test_list_schedules() {
    let server = TestServer::new().await;

    // Create two schedules
    server
        .post(
            "/v1/durable/schedules",
            json!({
                "name": "list-test-1",
                "cron_expression": "0 * * * * * *",
                "target": { "type": "workflow", "name": "wf1" }
            }),
        )
        .await
        .assert_status(StatusCode::CREATED);

    server
        .post(
            "/v1/durable/schedules",
            json!({
                "name": "list-test-2",
                "cron_expression": "0 0 * * * * *",
                "target": { "type": "activity", "name": "act1" }
            }),
        )
        .await
        .assert_status(StatusCode::CREATED);

    let body: Value = server
        .get("/v1/durable/schedules")
        .await
        .assert_status(StatusCode::OK)
        .json();

    let data = body["data"].as_array().unwrap();
    assert!(data.len() >= 2);
}

#[tokio::test]
async fn test_get_schedule() {
    let server = TestServer::new().await;

    let created: Value = server
        .post(
            "/v1/durable/schedules",
            json!({
                "name": "get-test",
                "cron_expression": "0 * * * * * *",
                "target": { "type": "workflow", "name": "wf1" }
            }),
        )
        .await
        .assert_status(StatusCode::CREATED)
        .json();

    let id = created["id"].as_str().unwrap();

    let fetched: Value = server
        .get(&format!("/v1/durable/schedules/{}", id))
        .await
        .assert_status(StatusCode::OK)
        .json();

    assert_eq!(fetched["name"], "get-test");
    assert_eq!(fetched["id"], id);
}

#[tokio::test]
async fn test_get_schedule_not_found() {
    let server = TestServer::new().await;

    server
        .get("/v1/durable/schedules/00000000-0000-0000-0000-000000000000")
        .await
        .assert_status(StatusCode::NOT_FOUND);
}

#[tokio::test]
async fn test_update_schedule() {
    let server = TestServer::new().await;

    let created: Value = server
        .post(
            "/v1/durable/schedules",
            json!({
                "name": "update-test",
                "cron_expression": "0 * * * * * *",
                "target": { "type": "workflow", "name": "wf1" }
            }),
        )
        .await
        .assert_status(StatusCode::CREATED)
        .json();

    let id = created["id"].as_str().unwrap();

    let updated: Value = server
        .patch(
            &format!("/v1/durable/schedules/{}", id),
            json!({
                "description": "Updated description",
                "max_concurrent": 3
            }),
        )
        .await
        .assert_status(StatusCode::OK)
        .json();

    assert_eq!(updated["description"], "Updated description");
    assert_eq!(updated["max_concurrent"], 3);
}

#[tokio::test]
async fn test_delete_schedule() {
    let server = TestServer::new().await;

    let created: Value = server
        .post(
            "/v1/durable/schedules",
            json!({
                "name": "delete-test",
                "cron_expression": "0 * * * * * *",
                "target": { "type": "workflow", "name": "wf1" }
            }),
        )
        .await
        .assert_status(StatusCode::CREATED)
        .json();

    let id = created["id"].as_str().unwrap();

    server
        .delete(&format!("/v1/durable/schedules/{}", id))
        .await
        .assert_status(StatusCode::NO_CONTENT);

    // Verify it's gone
    server
        .get(&format!("/v1/durable/schedules/{}", id))
        .await
        .assert_status(StatusCode::NOT_FOUND);
}

// --------------------------------------------
// Pause / Resume Tests
// --------------------------------------------

#[tokio::test]
async fn test_pause_and_resume_schedule() {
    let server = TestServer::new().await;

    let created: Value = server
        .post(
            "/v1/durable/schedules",
            json!({
                "name": "pause-resume-test",
                "cron_expression": "0 * * * * * *",
                "target": { "type": "workflow", "name": "wf1" }
            }),
        )
        .await
        .assert_status(StatusCode::CREATED)
        .json();

    let id = created["id"].as_str().unwrap();
    assert!(created["enabled"].as_bool().unwrap());

    // Pause
    let paused: Value = server
        .post(&format!("/v1/durable/schedules/{}/pause", id), json!({}))
        .await
        .assert_status(StatusCode::OK)
        .json();
    assert!(!paused["enabled"].as_bool().unwrap());

    // Resume
    let resumed: Value = server
        .post(&format!("/v1/durable/schedules/{}/resume", id), json!({}))
        .await
        .assert_status(StatusCode::OK)
        .json();
    assert!(resumed["enabled"].as_bool().unwrap());
}

// --------------------------------------------
// Manual Trigger Tests
// --------------------------------------------

#[tokio::test]
async fn test_manual_trigger() {
    let server = TestServer::new().await;

    let created: Value = server
        .post(
            "/v1/durable/schedules",
            json!({
                "name": "trigger-test",
                "cron_expression": "0 * * * * * *",
                "target": { "type": "workflow", "name": "wf1" }
            }),
        )
        .await
        .assert_status(StatusCode::CREATED)
        .json();

    let id = created["id"].as_str().unwrap();

    // Manual trigger
    let triggered: Value = server
        .post(&format!("/v1/durable/schedules/{}/trigger", id), json!({}))
        .await
        .assert_status(StatusCode::OK)
        .json();

    assert!(triggered["execution_id"].as_str().is_some());

    // Check execution exists
    let exec_id = triggered["execution_id"].as_str().unwrap();
    let exec: Value = server
        .get(&format!("/v1/durable/executions/{}", exec_id))
        .await
        .assert_status(StatusCode::OK)
        .json();

    assert_eq!(exec["schedule_id"].as_str().unwrap(), id);
}

// --------------------------------------------
// Execution History Tests
// --------------------------------------------

#[tokio::test]
async fn test_execution_history() {
    let server = TestServer::new().await;

    let created: Value = server
        .post(
            "/v1/durable/schedules",
            json!({
                "name": "history-test",
                "cron_expression": "0 * * * * * *",
                "target": { "type": "workflow", "name": "wf1" }
            }),
        )
        .await
        .assert_status(StatusCode::CREATED)
        .json();

    let id = created["id"].as_str().unwrap();

    // Trigger twice
    server
        .post(&format!("/v1/durable/schedules/{}/trigger", id), json!({}))
        .await
        .assert_status(StatusCode::OK);
    server
        .post(&format!("/v1/durable/schedules/{}/trigger", id), json!({}))
        .await
        .assert_status(StatusCode::OK);

    // Check execution list
    let execs: Value = server
        .get(&format!("/v1/durable/schedules/{}/executions", id))
        .await
        .assert_status(StatusCode::OK)
        .json();

    assert_eq!(execs["total"], 2);
    assert_eq!(execs["data"].as_array().unwrap().len(), 2);
}

// --------------------------------------------
// Schedule Stats Tests
// --------------------------------------------

#[tokio::test]
async fn test_schedule_stats() {
    let server = TestServer::new().await;

    let created: Value = server
        .post(
            "/v1/durable/schedules",
            json!({
                "name": "stats-test",
                "cron_expression": "0 * * * * * *",
                "target": { "type": "workflow", "name": "wf1" }
            }),
        )
        .await
        .assert_status(StatusCode::CREATED)
        .json();

    let id = created["id"].as_str().unwrap();

    // Stats before any executions
    let stats: Value = server
        .get(&format!("/v1/durable/schedules/{}/stats", id))
        .await
        .assert_status(StatusCode::OK)
        .json();

    assert_eq!(stats["total_executions"], 0);

    // Trigger and check stats
    server
        .post(&format!("/v1/durable/schedules/{}/trigger", id), json!({}))
        .await
        .assert_status(StatusCode::OK);

    let stats: Value = server
        .get(&format!("/v1/durable/schedules/{}/stats", id))
        .await
        .assert_status(StatusCode::OK)
        .json();

    assert_eq!(stats["total_executions"], 1);
}

// --------------------------------------------
// Activity Target Tests
// --------------------------------------------

#[tokio::test]
async fn test_create_activity_schedule() {
    let server = TestServer::new().await;

    let body: Value = server
        .post(
            "/v1/durable/schedules",
            json!({
                "name": "activity-test",
                "cron_expression": "0 * * * * * *",
                "target": {
                    "type": "activity",
                    "name": "cleanup-task",
                    "input": {"cleanup": true}
                },
                "max_concurrent": 2,
                "catch_up_missed": true
            }),
        )
        .await
        .assert_status(StatusCode::CREATED)
        .json();

    assert_eq!(body["target"]["type"], "activity");
    assert_eq!(body["target"]["name"], "cleanup-task");
    assert_eq!(body["max_concurrent"], 2);
    assert!(body["catch_up_missed"].as_bool().unwrap());
}

// --------------------------------------------
// Disabled Schedule Tests
// --------------------------------------------

#[tokio::test]
async fn test_create_disabled_schedule() {
    let server = TestServer::new().await;

    let body: Value = server
        .post(
            "/v1/durable/schedules",
            json!({
                "name": "disabled-test",
                "cron_expression": "0 * * * * * *",
                "target": { "type": "workflow", "name": "wf1" },
                "enabled": false
            }),
        )
        .await
        .assert_status(StatusCode::CREATED)
        .json();

    assert!(!body["enabled"].as_bool().unwrap());
    // Disabled schedules should not have next_trigger_at
    assert!(body["next_trigger_at"].is_null());
}

// --------------------------------------------
// List Filtering Tests
// --------------------------------------------

#[tokio::test]
async fn test_list_schedules_filter_by_target_type() {
    let server = TestServer::new().await;

    server
        .post(
            "/v1/durable/schedules",
            json!({
                "name": "wf-filter",
                "cron_expression": "0 * * * * * *",
                "target": { "type": "workflow", "name": "wf1" }
            }),
        )
        .await
        .assert_status(StatusCode::CREATED);

    server
        .post(
            "/v1/durable/schedules",
            json!({
                "name": "act-filter",
                "cron_expression": "0 * * * * * *",
                "target": { "type": "activity", "name": "act1" }
            }),
        )
        .await
        .assert_status(StatusCode::CREATED);

    let wf_only: Value = server
        .get("/v1/durable/schedules?target_type=workflow")
        .await
        .assert_status(StatusCode::OK)
        .json();

    for schedule in wf_only["data"].as_array().unwrap() {
        assert_eq!(schedule["target"]["type"], "workflow");
    }
}
