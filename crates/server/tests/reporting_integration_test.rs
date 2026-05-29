//! Reporting API integration tests.

mod test_harness;

use axum::http::StatusCode;
use chrono::{Duration, SecondsFormat, Utc};
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

    let owner_principal_id: Uuid =
        sqlx::query_scalar("SELECT id FROM principals WHERE org_id = 1 LIMIT 1")
            .fetch_one(&server.pool)
            .await
            .expect("seeded org principal");

    let (session_id, updated_at): (Uuid, chrono::DateTime<Utc>) = sqlx::query_as(
        r#"
        INSERT INTO sessions (org_id, owner_principal_id, title, status)
        VALUES (1, $1, $2, 'started')
        RETURNING id, updated_at
        "#,
    )
    .bind(owner_principal_id)
    .bind(&title)
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

    // The backfill formats `source_version` from `updated_at` as UTC with a
    // `Z` suffix and auto-scaled sub-second precision (see the projector SQL),
    // which matches chrono's `AutoSi` + `use_z` rendering — not the default
    // `+00:00` of `to_rfc3339()`.
    assert_eq!(
        row.0,
        updated_at.to_rfc3339_opts(SecondsFormat::AutoSi, true)
    );
}

#[tokio::test]
async fn reporting_projects_llm_generation_into_queryable_fact() {
    // End-to-end pipeline check: canonical llm_generations row -> backfill
    // enqueue -> projector run -> fact_llm_generation -> semantic query.
    let server = TestServer::new().await;
    let model = format!("e2e-model-{}", Uuid::now_v7());

    let owner_principal_id: Uuid =
        sqlx::query_scalar("SELECT id FROM principals WHERE org_id = 1 LIMIT 1")
            .fetch_one(&server.pool)
            .await
            .expect("seeded org principal");

    let session_id: Uuid = sqlx::query_scalar(
        r#"
        INSERT INTO sessions (org_id, owner_principal_id, title, status)
        VALUES (1, $1, $2, 'started')
        RETURNING id
        "#,
    )
    .bind(owner_principal_id)
    .bind(format!("reporting-e2e-{}", Uuid::now_v7()))
    .fetch_one(&server.pool)
    .await
    .expect("insert test session");

    sqlx::query(
        r#"
        INSERT INTO llm_generations
            (org_id, session_id, model, provider, input_tokens, output_tokens, duration_ms)
        VALUES (1, $1, $2, 'test-provider', 100, 50, 1200)
        "#,
    )
    .bind(session_id)
    .bind(&model)
    .execute(&server.pool)
    .await
    .expect("insert canonical llm generation");

    let backfill: Value = server
        .post("/v1/reports/admin/backfill", json!({ "limit": 100 }))
        .await
        .assert_status(StatusCode::OK)
        .json();
    assert!(backfill["llm_generations"].as_i64().unwrap() >= 1);

    server
        .post("/v1/reports/projector/run?limit=100", json!({}))
        .await
        .assert_status(StatusCode::OK);

    let to = Utc::now() + Duration::hours(1);
    let result: Value = server
        .post(
            "/v1/reports/query",
            json!({
                "dataset": "llm_generations",
                "time_range": {
                    "from": (to - Duration::days(2)).to_rfc3339(),
                    "to": to.to_rfc3339()
                },
                "dimensions": ["model"],
                "measures": ["total_tokens", "input_tokens", "output_tokens"],
                "filters": [{ "field": "model", "op": "eq", "value": model }],
                "limit": 100
            }),
        )
        .await
        .assert_status(StatusCode::OK)
        .json();

    let rows = result["rows"].as_array().expect("query rows");
    let row = rows
        .iter()
        .find(|r| r["model"] == json!(model))
        .expect("projected llm generation fact should be queryable");
    assert_eq!(row["input_tokens"].as_i64(), Some(100));
    assert_eq!(row["output_tokens"].as_i64(), Some(50));
    assert_eq!(row["total_tokens"].as_i64(), Some(150));
}
