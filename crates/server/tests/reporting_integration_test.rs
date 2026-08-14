//! Reporting API integration tests.

mod test_harness;

use axum::http::StatusCode;
use chrono::{Duration, Utc};
use everruns_server::storage::{
    CreateOrganizationRow, CreatePrincipalRow, reporting::PostgresReportingProjector,
};
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
            id: everruns_provider::typed_id::PrincipalId::new(),
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

async fn create_reporting_repair_session(server: &TestServer, org_id: i64, suffix: &str) -> Uuid {
    let principal = server
        .db
        .create_principal(CreatePrincipalRow {
            id: everruns_provider::typed_id::PrincipalId::new(),
            org_id,
            kind: "system".to_string(),
            subject_id: Some(Uuid::now_v7()),
            parent_principal_id: None,
            resolved_user_id: None,
            metadata: json!({ "source": "reporting_repair_test" }),
        })
        .await
        .expect("create repair test principal");
    let workspace_id = Uuid::now_v7();
    sqlx::query(
        r#"
        INSERT INTO workspaces (id, org_id, public_id, name, status)
        VALUES ($1, $2, $3, $4, 'active')
        "#,
    )
    .bind(workspace_id)
    .bind(org_id)
    .bind(format!("wsp_{}", workspace_id.simple()))
    .bind(format!("reporting-repair-{suffix}-{workspace_id}"))
    .execute(&server.pool)
    .await
    .expect("insert repair test workspace");

    sqlx::query_scalar(
        r#"
        INSERT INTO sessions (org_id, title, status, owner_principal_id, workspace_id)
        VALUES ($1, $2, 'started', $3, $4)
        RETURNING id
        "#,
    )
    .bind(org_id)
    .bind(format!("reporting-repair-{suffix}-{}", Uuid::now_v7()))
    .bind(principal.id.uuid())
    .bind(workspace_id)
    .fetch_one(&server.pool)
    .await
    .expect("insert repair test session")
}

#[tokio::test]
async fn reporting_backfill_skips_malformed_capability_usage_records() {
    let server = TestServer::new().await;
    let session_id = create_reporting_repair_session(&server, 1, "malformed-backfill").await;
    let malformed = Uuid::now_v7();
    let missing_turn = Uuid::now_v7();
    let base_time = Utc::now() + Duration::days(45);

    for (id, sequence, event_type, data) in [
        (
            malformed,
            1,
            "capability.usage",
            json!({ "records": { "capability_id": "cap-object", "usage_kind": "invoked" } }),
        ),
        (
            missing_turn,
            2,
            "turn.completed",
            json!({ "turn_id": "turn-after-malformed" }),
        ),
    ] {
        sqlx::query(
            r#"
            INSERT INTO events (
                id, session_id, sequence, event_type, data, context, ts, created_at
            )
            VALUES ($1, $2, $3, $4, $5, '{}'::jsonb, $6, $6)
            "#,
        )
        .bind(id)
        .bind(session_id)
        .bind(sequence)
        .bind(event_type)
        .bind(data)
        .bind(base_time + Duration::seconds(sequence.into()))
        .execute(&server.pool)
        .await
        .expect("insert malformed backfill fixture");
    }

    let result: Value = server
        .post("/v1/reports/admin/backfill", json!({ "limit": 100 }))
        .await
        .assert_status(StatusCode::OK)
        .json();
    assert!(result["events"].as_i64().unwrap() >= 1);

    let rows: Vec<(String,)> = sqlx::query_as(
        r#"
        SELECT source_id
          FROM reporting_outbox
         WHERE source_type = 'event'
           AND source_id = ANY($1)
         ORDER BY source_id
        "#,
    )
    .bind(vec![malformed.to_string(), missing_turn.to_string()])
    .fetch_all(&server.pool)
    .await
    .expect("load backfilled malformed rows");

    assert_eq!(rows, vec![(missing_turn.to_string(),)]);
}

#[tokio::test]
async fn bounded_event_repair_enqueues_only_missing_projections() {
    let server = TestServer::new().await;
    let other_org = server
        .db
        .create_organization(CreateOrganizationRow {
            public_id: format!("org_{}", Uuid::now_v7().simple()),
            name: format!("Reporting repair org {}", Uuid::now_v7()),
            created_by: None,
        })
        .await
        .expect("create second repair org");
    let primary_session = create_reporting_repair_session(&server, 1, "primary").await;
    let other_session = create_reporting_repair_session(&server, other_org.org_id, "other").await;

    let base_time = Utc::now() + Duration::days(30);
    let tool = Uuid::now_v7();
    let tool_missing = Uuid::now_v7();
    let malformed_capability = Uuid::now_v7();
    let capability_partial = Uuid::now_v7();
    let capability_complete = Uuid::now_v7();
    let guardrail = Uuid::now_v7();
    let guardrail_missing = Uuid::now_v7();
    let turn_completed = Uuid::now_v7();
    let turn_failed = Uuid::now_v7();
    let turn_cancelled = Uuid::now_v7();
    let ignored = Uuid::now_v7();
    let events = [
        (
            tool,
            primary_session,
            1,
            "tool.completed",
            json!({ "capability_id": "cap-tool", "tool_name": "search" }),
        ),
        (
            tool_missing,
            primary_session,
            2,
            "tool.completed",
            json!({ "capability_id": "cap-tool-missing", "tool_name": "fetch" }),
        ),
        (
            malformed_capability,
            primary_session,
            3,
            "capability.usage",
            json!({ "records": { "capability_id": "cap-malformed", "usage_kind": "invoked" } }),
        ),
        (
            capability_partial,
            primary_session,
            4,
            "capability.usage",
            json!({ "records": [
                { "capability_id": "cap-a", "usage_kind": "resolved" },
                { "capability_id": "cap-b", "usage_kind": "exposed" }
            ] }),
        ),
        (
            guardrail,
            primary_session,
            5,
            "output.message.replaced",
            json!({ "guardrail_capability_id": "guardrail-a" }),
        ),
        (
            guardrail_missing,
            primary_session,
            6,
            "output.message.replaced",
            json!({ "guardrail_capability_id": "guardrail-missing" }),
        ),
        (
            turn_completed,
            primary_session,
            7,
            "turn.completed",
            json!({ "turn_id": "turn-completed" }),
        ),
        (
            turn_failed,
            primary_session,
            8,
            "turn.failed",
            json!({ "turn_id": "turn-failed" }),
        ),
        (
            capability_complete,
            other_session,
            1,
            "capability.usage",
            json!({ "records": [
                { "capability_id": "cap-c", "usage_kind": "configured" },
                { "capability_id": "cap-d", "usage_kind": "invoked" }
            ] }),
        ),
        (
            turn_cancelled,
            other_session,
            2,
            "turn.cancelled",
            json!({ "turn_id": "turn-cancelled" }),
        ),
        (
            ignored,
            other_session,
            3,
            "output.message.delta",
            json!({ "delta": "ignored" }),
        ),
    ];
    for (id, session_id, sequence, event_type, data) in &events {
        sqlx::query(
            r#"
            INSERT INTO events (
                id, session_id, sequence, event_type, data, context, ts, created_at
            )
            VALUES ($1, $2, $3, $4, $5, '{}'::jsonb, $6, $6)
            "#,
        )
        .bind(id)
        .bind(session_id)
        .bind(sequence)
        .bind(event_type)
        .bind(data)
        .bind(base_time)
        .execute(&server.pool)
        .await
        .expect("insert repair test event");
    }

    for (org_id, id) in [
        (1, tool),
        (1, tool_missing),
        (1, capability_partial),
        (1, guardrail),
        (1, guardrail_missing),
        (1, turn_completed),
        (1, turn_failed),
        (other_org.org_id, capability_complete),
    ] {
        sqlx::query(
            r#"
            INSERT INTO reporting_outbox (
                org_id, source_type, source_id, source_version, reason, status, next_attempt_at
            )
            VALUES ($1, 'event', $2, $2, 'event_projection', 'completed', NOW())
            "#,
        )
        .bind(org_id)
        .bind(id.to_string())
        .execute(&server.pool)
        .await
        .expect("insert completed repair outbox row");
    }

    sqlx::query(
        r#"
        INSERT INTO fact_tool_call (org_id, source_key, created_at, time_bucket_day)
        VALUES (1, $1, $2, ($2 AT TIME ZONE 'UTC')::DATE)
        "#,
    )
    .bind(format!("event:{tool}"))
    .bind(base_time)
    .execute(&server.pool)
    .await
    .expect("insert completed tool fact");

    for (org_id, source_key, capability_id, usage_kind) in [
        (1, format!("event:{tool}:invoked"), "cap-tool", "invoked"),
        (
            1,
            format!("event:{capability_partial}:capability_usage:1"),
            "cap-a",
            "resolved",
        ),
        (
            1,
            format!("event:{guardrail}:effect_ran"),
            "guardrail-a",
            "effect_ran",
        ),
        (
            other_org.org_id,
            format!("event:{capability_complete}:capability_usage:1"),
            "cap-c",
            "configured",
        ),
        (
            other_org.org_id,
            format!("event:{capability_complete}:capability_usage:2"),
            "cap-d",
            "invoked",
        ),
    ] {
        sqlx::query(
            r#"
            INSERT INTO fact_capability_usage (
                org_id, source_key, created_at, time_bucket_day,
                capability_id, capability_usage_kind
            )
            VALUES ($1, $2, $3, ($3 AT TIME ZONE 'UTC')::DATE, $4, $5)
            "#,
        )
        .bind(org_id)
        .bind(source_key)
        .bind(base_time)
        .bind(capability_id)
        .bind(usage_kind)
        .execute(&server.pool)
        .await
        .expect("insert completed capability fact");
    }

    sqlx::query(
        r#"
        INSERT INTO fact_turn (
            org_id, source_key, created_at, time_bucket_day, turn_status
        )
        VALUES (1, $1, $2, ($2 AT TIME ZONE 'UTC')::DATE, 'failed')
        "#,
    )
    .bind(format!("event:{turn_failed}"))
    .bind(base_time)
    .execute(&server.pool)
    .await
    .expect("insert completed turn fact");

    sqlx::query(
        r#"
        UPDATE reporting_event_repair_cursor
           SET last_event_created_at = $1,
               last_event_id = '00000000-0000-0000-0000-000000000000'
         WHERE singleton
        "#,
    )
    .bind(base_time - Duration::seconds(1))
    .execute(&server.pool)
    .await
    .expect("position repair cursor before fixture");

    let first = PostgresReportingProjector::new(server.pool.clone());
    let second = PostgresReportingProjector::new(server.pool.clone());
    let (first_count, second_count) = tokio::join!(
        first.repair_missing_event_projections(100),
        second.repair_missing_event_projections(100)
    );
    assert_eq!(first_count.unwrap() + second_count.unwrap(), 5);

    let rows: Vec<(i64, String, String)> = sqlx::query_as(
        r#"
        SELECT org_id, source_id, status
          FROM reporting_outbox
         WHERE source_type = 'event'
           AND source_id = ANY($1)
         ORDER BY org_id, source_id
        "#,
    )
    .bind(vec![
        tool.to_string(),
        tool_missing.to_string(),
        malformed_capability.to_string(),
        capability_partial.to_string(),
        capability_complete.to_string(),
        guardrail.to_string(),
        guardrail_missing.to_string(),
        turn_completed.to_string(),
        turn_failed.to_string(),
        turn_cancelled.to_string(),
        ignored.to_string(),
    ])
    .fetch_all(&server.pool)
    .await
    .expect("load repaired outbox rows");
    let pending: Vec<_> = rows
        .iter()
        .filter(|(_, _, status)| status == "pending")
        .map(|(org_id, source_id, _)| (*org_id, source_id.clone()))
        .collect();
    let mut expected_pending = vec![
        (1, capability_partial.to_string()),
        (1, guardrail_missing.to_string()),
        (1, tool_missing.to_string()),
        (1, turn_completed.to_string()),
        (other_org.org_id, turn_cancelled.to_string()),
    ];
    expected_pending.sort();
    assert_eq!(pending, expected_pending);
    assert_eq!(
        rows.len(),
        9,
        "unsupported and malformed events must not be enqueued"
    );

    sqlx::query(
        r#"
        UPDATE reporting_event_repair_cursor
           SET last_event_created_at = $1,
               last_event_id = '00000000-0000-0000-0000-000000000000'
         WHERE singleton
        "#,
    )
    .bind(base_time - Duration::seconds(1))
    .execute(&server.pool)
    .await
    .expect("rewind repair cursor for idempotence check");
    assert_eq!(
        PostgresReportingProjector::new(server.pool.clone())
            .repair_missing_event_projections(100)
            .await
            .unwrap(),
        0
    );
}

#[tokio::test]
#[ignore = "benchmark: seeds 60k events and 180k capability facts"]
async fn reporting_event_repair_benchmark() {
    let server = TestServer::new().await;
    let session_id = create_reporting_repair_session(&server, 1, "benchmark").await;
    let base_time = Utc::now() + Duration::days(60);

    let (event_count, fact_count, outbox_count): (i64, i64, i64) = sqlx::query_as(
        r#"
        WITH inserted_events AS (
            INSERT INTO events (
                id, session_id, sequence, event_type, data, context, ts, created_at
            )
            SELECT uuidv7(), $1, n, 'capability.usage',
                   jsonb_build_object('records', jsonb_build_array(
                       jsonb_build_object(
                           'capability_id', 'cap-a', 'usage_kind', 'configured'
                       ),
                       jsonb_build_object(
                           'capability_id', 'cap-b', 'usage_kind', 'resolved'
                       ),
                       jsonb_build_object(
                           'capability_id', 'cap-c', 'usage_kind', 'exposed'
                       )
                   )),
                   '{}'::jsonb,
                   $2 + n * INTERVAL '1 microsecond',
                   $2 + n * INTERVAL '1 microsecond'
              FROM generate_series(1, 60000) n
            RETURNING id, session_id, created_at
        ),
        inserted_facts AS (
            INSERT INTO fact_capability_usage (
                org_id, source_key, session_id, created_at, time_bucket_day,
                capability_id, capability_usage_kind
            )
            SELECT 1,
                   'event:' || inserted_events.id::TEXT
                       || ':capability_usage:' || ordinal::TEXT,
                   inserted_events.session_id,
                   inserted_events.created_at,
                   (inserted_events.created_at AT TIME ZONE 'UTC')::DATE,
                   'cap-' || ordinal::TEXT,
                   CASE ordinal
                       WHEN 1 THEN 'configured'
                       WHEN 2 THEN 'resolved'
                       ELSE 'exposed'
                   END
              FROM inserted_events
             CROSS JOIN generate_series(1, 3) ordinal
            RETURNING 1
        ),
        inserted_outbox AS (
            INSERT INTO reporting_outbox (
                org_id, source_type, source_id, source_version,
                reason, status, next_attempt_at
            )
            SELECT 1, 'event', id::TEXT, id::TEXT,
                   'event_projection', 'completed', NOW()
              FROM inserted_events
            RETURNING 1
        )
        SELECT (SELECT COUNT(*) FROM inserted_events),
               (SELECT COUNT(*) FROM inserted_facts),
               (SELECT COUNT(*) FROM inserted_outbox)
        "#,
    )
    .bind(session_id)
    .bind(base_time)
    .fetch_one(&server.pool)
    .await
    .expect("seed reporting repair benchmark");
    assert_eq!(
        (event_count, fact_count, outbox_count),
        (60_000, 180_000, 60_000)
    );

    sqlx::query(
        r#"
        UPDATE reporting_event_repair_cursor
           SET last_event_created_at = $1,
               last_event_id = '00000000-0000-0000-0000-000000000000'
         WHERE singleton
        "#,
    )
    .bind(base_time)
    .execute(&server.pool)
    .await
    .expect("position benchmark cursor");
    sqlx::query("ANALYZE events")
        .execute(&server.pool)
        .await
        .expect("analyze benchmark events");
    sqlx::query("ANALYZE fact_capability_usage")
        .execute(&server.pool)
        .await
        .expect("analyze benchmark facts");

    let started = std::time::Instant::now();
    let enqueued = PostgresReportingProjector::new(server.pool.clone())
        .repair_missing_event_projections(1_000)
        .await
        .expect("run reporting repair benchmark");
    let elapsed = started.elapsed();

    assert_eq!(enqueued, 0);
    assert!(
        elapsed < std::time::Duration::from_secs(1),
        "no-work repair took {elapsed:?}"
    );
    eprintln!(
        "reporting repair benchmark: {event_count} events, {fact_count} facts, {} ms",
        elapsed.as_millis()
    );
}

#[tokio::test]
async fn fact_session_projection_uses_denormalized_counts_for_large_histories() {
    let server = TestServer::new().await;
    let org = server
        .db
        .create_organization(CreateOrganizationRow {
            public_id: format!("org_{}", Uuid::now_v7().simple()),
            name: format!("Reporting fact session org {}", Uuid::now_v7()),
            created_by: None,
        })
        .await
        .expect("create fact session projection org");
    let session_id =
        create_reporting_repair_session(&server, org.org_id, "fact-session-large").await;
    let base_time = Utc::now() - Duration::days(90);

    let (event_count, turn_events, tool_events): (i64, i64, i64) = sqlx::query_as(
        r#"
        WITH inserted AS (
            INSERT INTO events (
                id, session_id, sequence, event_type, data, context, ts, created_at
            )
            SELECT uuidv7(),
                   $1,
                   n,
                   CASE (n % 3)
                       WHEN 0 THEN 'turn.completed'
                       WHEN 1 THEN 'tool.completed'
                       ELSE 'output.message.delta'
                   END,
                   '{}'::jsonb,
                   '{}'::jsonb,
                   $2 + n * INTERVAL '1 microsecond',
                   $2 + n * INTERVAL '1 microsecond'
              FROM generate_series(1, 3000) n
            RETURNING event_type
        )
        SELECT COUNT(*)::BIGINT,
               COUNT(*) FILTER (
                   WHERE event_type IN ('turn.completed', 'turn.failed', 'turn.cancelled')
               )::BIGINT,
               COUNT(*) FILTER (WHERE event_type = 'tool.completed')::BIGINT
          FROM inserted
        "#,
    )
    .bind(session_id)
    .bind(base_time)
    .fetch_one(&server.pool)
    .await
    .expect("seed large session history");

    assert_eq!(event_count, 3000);
    assert_eq!(turn_events, 1000);
    assert_eq!(tool_events, 1000);

    let (session_turn_count, session_tool_count): (i64, i64) =
        sqlx::query_as("SELECT turn_count, tool_call_count FROM sessions WHERE id = $1")
            .bind(session_id)
            .fetch_one(&server.pool)
            .await
            .expect("load denormalized session counters");
    assert_eq!((session_turn_count, session_tool_count), (1000, 1000));

    let plan: Vec<(String,)> = sqlx::query_as(
        r#"
        EXPLAIN
        INSERT INTO fact_session (
            org_id, source_key, session_id, user_id, principal_id, agent_id,
            agent_name_snapshot, harness_id, harness_name_snapshot, app_id,
            blueprint_id, created_at, time_bucket_day, session_status, started_at,
            finished_at, last_active_at, session_count, duration_ms, turn_count,
            input_tokens, output_tokens, cache_read_tokens, cache_creation_tokens,
            total_tokens, tool_call_count, projected_at
        )
        SELECT
            s.org_id,
            'session:' || s.id::TEXT,
            s.id,
            s.resolved_owner_user_id,
            s.owner_principal_id,
            s.agent_id,
            a.name,
            s.harness_id,
            h.name,
            s.app_id,
            s.blueprint_id,
            s.created_at,
            (s.created_at AT TIME ZONE 'UTC')::DATE,
            s.status,
            s.started_at,
            s.finished_at,
            s.updated_at,
            1,
            CASE
                WHEN s.started_at IS NOT NULL AND s.finished_at IS NOT NULL
                THEN GREATEST(EXTRACT(EPOCH FROM (s.finished_at - s.started_at)) * 1000, 0)::BIGINT
                ELSE 0
            END,
            s.turn_count,
            s.total_input_tokens,
            s.total_output_tokens,
            s.total_cache_read_tokens,
            s.total_cache_creation_tokens,
            s.total_input_tokens + s.total_output_tokens
                + s.total_cache_read_tokens + s.total_cache_creation_tokens,
            s.tool_call_count,
            NOW()
        FROM sessions s
        LEFT JOIN agents a ON a.id = s.agent_id AND a.org_id = s.org_id
        LEFT JOIN harnesses h ON h.id = s.harness_id AND h.org_id = s.org_id
        WHERE s.id = $1 AND s.org_id = $2
        ON CONFLICT (org_id, source_key) DO UPDATE SET
            turn_count = EXCLUDED.turn_count,
            tool_call_count = EXCLUDED.tool_call_count,
            projected_at = NOW()
        "#,
    )
    .bind(session_id)
    .bind(org.org_id)
    .fetch_all(&server.pool)
    .await
    .expect("explain fact_session upsert");
    let plan_text = plan
        .iter()
        .map(|(line,)| line.as_str())
        .collect::<Vec<_>>()
        .join("\n");
    assert!(
        !plan_text.contains("events"),
        "fact_session upsert must not scan events; plan:\n{plan_text}"
    );

    async fn enqueue_session_snapshot(pool: &sqlx::PgPool, session_id: Uuid) {
        sqlx::query(
            r#"
            INSERT INTO reporting_outbox (
                org_id, source_type, source_id, source_version, reason, status, next_attempt_at
            )
            SELECT org_id, 'session', id::TEXT, updated_at::TEXT, 'session_snapshot', 'pending', NOW()
              FROM sessions
             WHERE id = $1
            ON CONFLICT (org_id, source_type, source_id, source_version, reason)
            DO UPDATE SET
                status = 'pending',
                next_attempt_at = NOW(),
                updated_at = NOW()
            "#,
        )
        .bind(session_id)
        .execute(pool)
        .await
        .expect("enqueue session snapshot");
    }

    enqueue_session_snapshot(&server.pool, session_id).await;
    let projector = PostgresReportingProjector::new(server.pool.clone());

    let first_started = std::time::Instant::now();
    projector
        .run_once(org.org_id, 1)
        .await
        .expect("first session projection");
    let first_elapsed = first_started.elapsed();

    let (fact_turn_count, fact_tool_count): (i64, i64) = sqlx::query_as(
        "SELECT turn_count, tool_call_count FROM fact_session WHERE session_id = $1",
    )
    .bind(session_id)
    .fetch_one(&server.pool)
    .await
    .expect("load projected session fact");
    assert_eq!((fact_turn_count, fact_tool_count), (1000, 1000));

    sqlx::query("UPDATE sessions SET updated_at = NOW() WHERE id = $1")
        .bind(session_id)
        .execute(&server.pool)
        .await
        .expect("touch session for repeated projection");
    enqueue_session_snapshot(&server.pool, session_id).await;

    let repeat_started = std::time::Instant::now();
    projector
        .run_once(org.org_id, 1)
        .await
        .expect("repeated session projection");
    let repeat_elapsed = repeat_started.elapsed();

    let (fact_turn_count, fact_tool_count): (i64, i64) = sqlx::query_as(
        "SELECT turn_count, tool_call_count FROM fact_session WHERE session_id = $1",
    )
    .bind(session_id)
    .fetch_one(&server.pool)
    .await
    .expect("reload projected session fact after repeat");
    assert_eq!((fact_turn_count, fact_tool_count), (1000, 1000));

    assert!(
        first_elapsed < std::time::Duration::from_millis(500),
        "first projection took {first_elapsed:?} with {event_count} events"
    );
    assert!(
        repeat_elapsed < std::time::Duration::from_millis(500),
        "repeated projection took {repeat_elapsed:?} with {event_count} events"
    );
    eprintln!(
        "fact_session projection benchmark: {event_count} events, first={}ms repeat={}ms",
        first_elapsed.as_millis(),
        repeat_elapsed.as_millis()
    );
}
