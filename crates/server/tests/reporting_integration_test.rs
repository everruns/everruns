//! Reporting API integration tests.

mod test_harness;

use axum::http::StatusCode;
use chrono::{Duration, Utc};
use serde_json::{Value, json};
use test_harness::TestServer;

fn report_query() -> Value {
    let to = Utc::now();
    json!({
        "dataset": "tool_calls",
        "time_range": {
            "from": (to - Duration::days(1)).to_rfc3339(),
            "to": to.to_rfc3339()
        },
        "dimensions": ["tool"],
        "measures": ["call_count"],
        "limit": 100
    })
}

#[tokio::test]
async fn saved_reports_are_crud_runnable_and_exportable() {
    let server = TestServer::in_memory().await;

    let created: Value = server
        .post(
            "/v1/reports/saved",
            json!({
                "name": "Tool usage",
                "description": "Dashboard report",
                "query": report_query(),
                "dashboard": {
                    "title": "Tool usage",
                    "section": "Operations",
                    "chart_type": "table",
                    "position": 1
                }
            }),
        )
        .await
        .assert_status(StatusCode::CREATED)
        .json();

    let report_id = created["id"].as_str().expect("saved report id");
    assert_eq!(created["name"], "Tool usage");
    assert!(created.get("org_id").is_none());

    let list: Value = server
        .get("/v1/reports/saved")
        .await
        .assert_status(StatusCode::OK)
        .json();
    assert_eq!(list["data"].as_array().expect("saved report list").len(), 1);

    let run_result: Value = server
        .post(
            format!("/v1/reports/saved/{report_id}/run").as_str(),
            json!({}),
        )
        .await
        .assert_status(StatusCode::OK)
        .json();
    assert_eq!(run_result["columns"][0]["name"], "tool");
    assert_eq!(run_result["columns"][1]["name"], "call_count");
    assert!(run_result.get("as_of").is_some());

    let export: Value = server
        .post(
            format!("/v1/reports/saved/{report_id}/export").as_str(),
            json!({ "format": "csv" }),
        )
        .await
        .assert_status(StatusCode::OK)
        .json();
    assert_eq!(export["content_type"], "text/csv; charset=utf-8");
    assert_eq!(export["filename"], "tool-usage.csv");
    assert!(
        export["content"]
            .as_str()
            .unwrap()
            .starts_with("\"tool\",\"call_count\"\n")
    );

    let updated: Value = server
        .patch(
            format!("/v1/reports/saved/{report_id}").as_str(),
            json!({ "description": null }),
        )
        .await
        .assert_status(StatusCode::OK)
        .json();
    assert!(updated.get("description").is_none());

    server
        .patch(
            format!("/v1/reports/saved/{report_id}").as_str(),
            json!({ "query": null }),
        )
        .await
        .assert_status(StatusCode::BAD_REQUEST);

    server
        .delete(format!("/v1/reports/saved/{report_id}").as_str())
        .await
        .assert_status(StatusCode::NO_CONTENT);
}

#[tokio::test]
async fn report_exports_reject_invalid_semantic_queries() {
    let server = TestServer::in_memory().await;

    server
        .post(
            "/v1/reports/query/export",
            json!({
                "format": "json",
                "query": {
                    "dataset": "tool_calls",
                    "time_range": report_query()["time_range"].clone(),
                    "dimensions": ["model"],
                    "measures": ["call_count"],
                    "limit": 100
                }
            }),
        )
        .await
        .assert_status(StatusCode::BAD_REQUEST);
}

#[tokio::test]
async fn reporting_diagnostics_are_admin_scoped() {
    let server = TestServer::in_memory().await;

    let diagnostics: Value = server
        .get("/v1/reports/admin/diagnostics")
        .await
        .assert_status(StatusCode::OK)
        .json();

    assert!(diagnostics["projector_lag"].as_array().unwrap().len() >= 5);
    assert_eq!(diagnostics["outbox"]["failed"], 0);
}
