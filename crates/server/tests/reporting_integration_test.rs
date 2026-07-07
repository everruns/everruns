//! Reporting API integration tests.

mod test_harness;

use axum::http::StatusCode;
use chrono::{Duration, Utc};
use serde_json::{Value, json};
use test_harness::TestServer;
use uuid::Uuid;

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

#[tokio::test]
async fn reporting_backfill_endpoint_is_admin_scoped() {
    let server = TestServer::in_memory().await;

    let result: Value = server
        .post("/v1/reports/admin/backfill", json!({ "limit": 100 }))
        .await
        .assert_status(StatusCode::OK)
        .json();

    assert_eq!(result["enqueued"], 0);
    assert_eq!(result["events"], 0);
    assert_eq!(result["sessions"], 0);
    assert_eq!(result["llm_generations"], 0);
    assert_eq!(result["usage_ledger"], 0);
}

#[tokio::test]
async fn reporting_backfill_enqueues_missing_postgres_session_fact() {
    let server = TestServer::new().await;
    let title = format!("reporting-backfill-{}", Uuid::now_v7());

    // `sessions.owner_principal_id` is NOT NULL (migration 018), so seed a
    // principal for the default org and own the raw-inserted session with it.
    let owner_principal_id = server
        .db
        .create_principal(everruns_server::storage::CreatePrincipalRow {
            id: everruns_core::PrincipalId::new(),
            org_id: everruns_core::DEFAULT_ORG_ID,
            kind: "system".to_string(),
            subject_id: Some(Uuid::now_v7()),
            parent_principal_id: None,
            resolved_user_id: None,
            metadata: json!({ "source": "reporting_integration_test" }),
        })
        .await
        .expect("create owner principal")
        .id;

    // `sessions.workspace_id` is also NOT NULL (migration 056) and FK-references
    // `workspaces`, so create a workspace for the default org and own the
    // session with it (mirrors the per-session default workspace in production).
    let workspace_id = Uuid::now_v7();
    sqlx::query(
        r#"
        INSERT INTO workspaces (id, org_id, public_id, name, status)
        VALUES ($1, 1, $2, $3, 'active')
        "#,
    )
    .bind(workspace_id)
    .bind(format!("wsp_{}", workspace_id.simple()))
    .bind(format!("reporting-backfill-ws-{workspace_id}"))
    .execute(&server.pool)
    .await
    .expect("insert test workspace");

    let (session_id, updated_at): (Uuid, chrono::DateTime<Utc>) = sqlx::query_as(
        r#"
        INSERT INTO sessions (org_id, title, status, owner_principal_id, workspace_id)
        VALUES (1, $1, 'started', $2, $3)
        RETURNING id, updated_at
        "#,
    )
    .bind(&title)
    .bind(owner_principal_id.uuid())
    .bind(workspace_id)
    .fetch_one(&server.pool)
    .await
    .expect("insert test session");

    let result: Value = server
        .post("/v1/reports/admin/backfill", json!({ "limit": 100 }))
        .await
        .assert_status(StatusCode::OK)
        .json();

    assert!(result["sessions"].as_i64().unwrap() >= 1);

    let row: (String,) = sqlx::query_as(
        r#"
        SELECT source_version
          FROM reporting_outbox
         WHERE org_id = 1
           AND source_type = 'session'
           AND source_id = $1
           AND reason = 'session_snapshot'
        "#,
    )
    .bind(session_id.to_string())
    .fetch_one(&server.pool)
    .await
    .expect("backfill should enqueue session outbox row");

    // The backfill stores the snapshot version as a `Z`-suffixed RFC3339 string
    // with auto-trimmed sub-second precision (see `backfill_sessions` in
    // crates/server/src/storage/reporting/outbox.rs). Match that formatting
    // rather than `to_rfc3339()`, which renders the offset as `+00:00`.
    assert_eq!(
        row.0,
        updated_at.to_rfc3339_opts(chrono::SecondsFormat::AutoSi, true)
    );
}
