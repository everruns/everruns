//! Session detail tab counts (EVE-868).
//!
//! The tab bar renders on every session page load, so the counts behind the
//! Work, Events and Workspace badges must be O(1) reads rather than aggregates
//! over history. These tests pin both halves of that contract: the counters
//! stay correct as rows arrive and leave, and reading them costs an index
//! lookup rather than a scan of `events`.

mod test_harness;

use everruns_server::storage::{CreateOrganizationRow, CreatePrincipalRow};
use serde_json::json;
use test_harness::TestServer;
use uuid::Uuid;

/// A session with its own workspace, created through SQL so the test observes
/// exactly the trigger behaviour and nothing the service layer adds.
async fn create_counted_session(server: &TestServer, org_id: i64) -> (Uuid, Uuid) {
    let principal = server
        .db
        .create_principal(CreatePrincipalRow {
            id: everruns_provider::typed_id::PrincipalId::new(),
            org_id,
            kind: "system".to_string(),
            subject_id: Some(Uuid::now_v7()),
            parent_principal_id: None,
            resolved_user_id: None,
            metadata: json!({ "source": "session_tab_counts_test" }),
        })
        .await
        .expect("create tab-count test principal");

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
    .bind(format!("tab-counts-{workspace_id}"))
    .execute(&server.pool)
    .await
    .expect("insert tab-count test workspace");

    let session_id: Uuid = sqlx::query_scalar(
        r#"
        INSERT INTO sessions (org_id, title, status, owner_principal_id, workspace_id)
        VALUES ($1, $2, 'started', $3, $4)
        RETURNING id
        "#,
    )
    .bind(org_id)
    .bind(format!("tab-counts-{}", Uuid::now_v7()))
    .bind(principal.id.uuid())
    .bind(workspace_id)
    .fetch_one(&server.pool)
    .await
    .expect("insert tab-count test session");

    (session_id, workspace_id)
}

async fn create_org(server: &TestServer, label: &str) -> i64 {
    server
        .db
        .create_organization(CreateOrganizationRow {
            public_id: format!("org_{}", Uuid::now_v7().simple()),
            name: format!("{label} {}", Uuid::now_v7()),
            created_by: None,
        })
        .await
        .expect("create tab-count test org")
        .org_id
}

async fn counts(server: &TestServer, session_id: Uuid, workspace_id: Uuid) -> (i64, i64, i64) {
    let (event_count, task_count): (i64, i64) =
        sqlx::query_as("SELECT event_count, task_count FROM sessions WHERE id = $1")
            .bind(session_id)
            .fetch_one(&server.pool)
            .await
            .expect("load session tab counters");
    let file_count: i64 = sqlx::query_scalar("SELECT file_count FROM workspaces WHERE id = $1")
        .bind(workspace_id)
        .fetch_one(&server.pool)
        .await
        .expect("load workspace file counter");
    (event_count, task_count, file_count)
}

#[tokio::test]
async fn tab_counters_track_events_tasks_and_files() {
    let server = TestServer::new().await;
    let org_id = create_org(&server, "Session tab counts org").await;
    let (session_id, workspace_id) = create_counted_session(&server, org_id).await;

    assert_eq!(counts(&server, session_id, workspace_id).await, (0, 0, 0));

    sqlx::query(
        r#"
        INSERT INTO events (id, session_id, sequence, event_type, data, context, ts, created_at)
        SELECT uuidv7(), $1, n, 'output.message.delta', '{}'::jsonb, '{}'::jsonb, NOW(), NOW()
          FROM generate_series(1, 250) n
        "#,
    )
    .bind(session_id)
    .execute(&server.pool)
    .await
    .expect("seed events");

    for kind in ["subagent", "background_tool"] {
        sqlx::query(
            r#"
            INSERT INTO session_tasks (id, session_id, kind, display_name)
            VALUES ($1, $2, $3, $4)
            "#,
        )
        .bind(format!("task_{}", Uuid::now_v7().simple()))
        .bind(session_id)
        .bind(kind)
        .bind(kind)
        .execute(&server.pool)
        .await
        .expect("seed session task");
    }

    // Two files and one directory: the badge answers "is there anything to look
    // at", so directories are not part of the count.
    sqlx::query(
        r#"
        INSERT INTO workspace_files (workspace_id, path, content, is_directory)
        VALUES ($1, '/notes', NULL, TRUE),
               ($1, '/notes/a.txt', '\x61'::bytea, FALSE),
               ($1, '/notes/b.txt', '\x62'::bytea, FALSE)
        "#,
    )
    .bind(workspace_id)
    .execute(&server.pool)
    .await
    .expect("seed workspace files");

    assert_eq!(
        counts(&server, session_id, workspace_id).await,
        (250, 2, 2),
        "counters follow inserts"
    );

    sqlx::query("DELETE FROM session_tasks WHERE session_id = $1")
        .bind(session_id)
        .execute(&server.pool)
        .await
        .expect("delete session tasks");
    sqlx::query("DELETE FROM workspace_files WHERE workspace_id = $1 AND path = '/notes/a.txt'")
        .bind(workspace_id)
        .execute(&server.pool)
        .await
        .expect("delete one workspace file");

    assert_eq!(
        counts(&server, session_id, workspace_id).await,
        (250, 0, 1),
        "counters follow deletes"
    );

    // A directory flipped into a file has to move the counter too, otherwise it
    // drifts permanently.
    sqlx::query("UPDATE workspace_files SET is_directory = FALSE WHERE workspace_id = $1 AND path = '/notes'")
        .bind(workspace_id)
        .execute(&server.pool)
        .await
        .expect("flip directory into file");

    assert_eq!(
        counts(&server, session_id, workspace_id).await,
        (250, 0, 2),
        "counters follow is_directory updates"
    );
}

#[tokio::test]
async fn session_response_carries_the_counts_and_omits_the_empty_ones() {
    use axum::http::StatusCode;

    let server = TestServer::in_memory().await;
    let session: serde_json::Value = server
        .post(
            "/v1/sessions",
            json!({
                "harness_id": server.seed_base_harness_id,
                "title": "tab-counts"
            }),
        )
        .await
        .assert_status(StatusCode::CREATED)
        .json_value();

    let session_id = session["id"].as_str().expect("session id").to_string();
    let workspace_id = session["workspace_id"]
        .as_str()
        .expect("workspace id")
        .to_string();

    // A brand new session has nothing behind any tab, and the payload says so
    // by omission rather than by a run of zeroes.
    let fresh: serde_json::Value = server
        .get(&format!("/v1/sessions/{session_id}"))
        .await
        .assert_status(StatusCode::OK)
        .json_value();
    for field in ["event_count", "task_count", "file_count"] {
        assert!(
            fresh.get(field).is_none_or(serde_json::Value::is_null),
            "empty session must not report {field}: {fresh}"
        );
    }

    for path in ["notes/a.txt", "notes/b.txt"] {
        server
            .post(
                &format!("/v1/workspaces/{workspace_id}/fs/{path}"),
                json!({ "content": "hello", "encoding": "text" }),
            )
            .await
            .assert_status(StatusCode::CREATED);
    }

    let after: serde_json::Value = server
        .get(&format!("/v1/sessions/{session_id}"))
        .await
        .assert_status(StatusCode::OK)
        .json_value();
    assert_eq!(
        after["file_count"].as_u64(),
        Some(2),
        "workspace files must reach the session payload: {after}"
    );
}

#[tokio::test]
async fn session_detail_read_never_scans_events() {
    let server = TestServer::new().await;
    let org_id = create_org(&server, "Session tab counts plan org").await;
    let (session_id, _workspace_id) = create_counted_session(&server, org_id).await;

    sqlx::query(
        r#"
        INSERT INTO events (id, session_id, sequence, event_type, data, context, ts, created_at)
        SELECT uuidv7(), $1, n, 'output.message.delta', '{}'::jsonb, '{}'::jsonb, NOW(), NOW()
          FROM generate_series(1, 2000) n
        "#,
    )
    .bind(session_id)
    .execute(&server.pool)
    .await
    .expect("seed events");
    sqlx::query("ANALYZE events")
        .execute(&server.pool)
        .await
        .expect("analyze events");
    sqlx::query("ANALYZE sessions")
        .execute(&server.pool)
        .await
        .expect("analyze sessions");

    // The plan for the session-detail read, which is the query the tab bar's
    // counts ride along on.
    let plan: Vec<(String,)> = sqlx::query_as(
        r#"
        EXPLAIN
        SELECT s.id, s.event_count, s.task_count, COALESCE(w.file_count, 0) AS workspace_file_count
          FROM sessions s
          LEFT JOIN workspaces w ON w.id = s.workspace_id
         WHERE s.org_id = $1 AND s.id = $2
        "#,
    )
    .bind(org_id)
    .bind(session_id)
    .fetch_all(&server.pool)
    .await
    .expect("explain session detail read");

    let plan_text = plan
        .into_iter()
        .map(|(line,)| line)
        .collect::<Vec<_>>()
        .join("\n");

    // The two properties that matter, and the only two that are stable: the
    // read never touches `events`, and it never aggregates. How PostgreSQL
    // reaches the `sessions` row is its call — on a table this small it will
    // pick a sequential scan over an index every time, which says nothing about
    // behaviour at production size.
    assert!(
        !plan_text.contains("events"),
        "session detail read must not touch the events table:\n{plan_text}"
    );
    assert!(
        !plan_text.contains("Aggregate"),
        "session detail read must read counters, not compute them:\n{plan_text}"
    );
    assert!(
        plan_text.contains("workspaces_pkey"),
        "the workspace file counter must be reached by primary key:\n{plan_text}"
    );
}
