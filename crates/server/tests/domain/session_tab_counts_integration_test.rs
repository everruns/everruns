//! Session detail tab counts (EVE-868).
//!
//! The tab bar renders on every session page load, so the counts behind the
//! Work, Events and Workspace badges must be O(1) reads rather than aggregates
//! over history. These tests pin both halves of that contract: the counters
//! stay correct as rows arrive and leave, and reading them costs an index
//! lookup rather than a scan of `events`.

use crate::test_harness;

use everruns_contracts::typed_id::SessionId;
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
            id: everruns_contracts::typed_id::PrincipalId::new(),
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
    (
        event_count(server, session_id).await,
        task_count(server, session_id).await,
        file_count(server, workspace_id).await,
    )
}

/// Read through the session repository: `event_count` is derived from
/// `event_sequences` by the query, not stored on the row.
async fn event_count(server: &TestServer, session_id: Uuid) -> i64 {
    let org_id: i64 = sqlx::query_scalar("SELECT org_id FROM sessions WHERE id = $1")
        .bind(session_id)
        .fetch_one(&server.pool)
        .await
        .expect("load session org");
    server
        .db
        .get_session(org_id, SessionId::from_uuid(session_id))
        .await
        .expect("load session")
        .expect("session exists")
        .event_count
}

async fn task_count(server: &TestServer, session_id: Uuid) -> i64 {
    sqlx::query_scalar("SELECT task_count FROM sessions WHERE id = $1")
        .bind(session_id)
        .fetch_one(&server.pool)
        .await
        .expect("load session task counter")
}

async fn file_count(server: &TestServer, workspace_id: Uuid) -> i64 {
    sqlx::query_scalar("SELECT file_count FROM workspaces WHERE id = $1")
        .bind(workspace_id)
        .fetch_one(&server.pool)
        .await
        .expect("load workspace file counter")
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
        SELECT uuidv7(), $1, allocate_event_sequence($1), 'output.message.delta',
               '{}'::jsonb, '{}'::jsonb, NOW(), NOW()
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

/// Retention deletes leave the derived event count matching the live events
/// (migration 191): deleted events are recorded as removed sequence numbers.
#[tokio::test]
async fn event_count_follows_retention_deletes() {
    let server = TestServer::new().await;
    let org_id = create_org(&server, "Session event count deletes org").await;
    let (session_id, _workspace_id) = create_counted_session(&server, org_id).await;

    sqlx::query(
        r#"
        INSERT INTO events (id, session_id, sequence, event_type, data, context, ts, created_at)
        SELECT uuidv7(), $1, allocate_event_sequence($1), 'output.message.delta',
               '{}'::jsonb, '{}'::jsonb, NOW(), NOW()
          FROM generate_series(1, 5) n
        "#,
    )
    .bind(session_id)
    .execute(&server.pool)
    .await
    .expect("seed events");
    assert_eq!(event_count(&server, session_id).await, 5);

    let mut tx = server.pool.begin().await.expect("begin");
    sqlx::query("SET LOCAL app.archival_bypass = 'true'")
        .execute(&mut *tx)
        .await
        .expect("archival bypass");
    sqlx::query("DELETE FROM events WHERE session_id = $1 AND sequence <= 2")
        .bind(session_id)
        .execute(&mut *tx)
        .await
        .expect("archive two events");
    tx.commit().await.expect("commit");
    assert_eq!(event_count(&server, session_id).await, 3);

    sqlx::query(
        r#"
        INSERT INTO events (id, session_id, sequence, event_type, data, context, ts, created_at)
        VALUES (uuidv7(), $1, allocate_event_sequence($1), 'output.message.delta',
                '{}'::jsonb, '{}'::jsonb, NOW(), NOW())
        "#,
    )
    .bind(session_id)
    .execute(&server.pool)
    .await
    .expect("insert after archival");
    assert_eq!(event_count(&server, session_id).await, 4);
}

/// Turn counters and the last-turn pointer move once per turn, when its
/// terminal event lands (migration 191). Tool calls are counted from the
/// events since the previous terminal event, and the pointer never moves back
/// for an older turn.
#[tokio::test]
async fn turn_counters_move_once_per_turn() {
    let server = TestServer::new().await;
    let org_id = create_org(&server, "Session turn counters org").await;
    let (session_id, _workspace_id) = create_counted_session(&server, org_id).await;

    let insert = |sequence: i64, kind: &'static str| {
        sqlx::query(
            r#"
            INSERT INTO events (id, session_id, sequence, event_type, data, context, ts, created_at)
            VALUES (uuidv7(), $1, $2, $3, '{}'::jsonb, '{}'::jsonb, NOW(), NOW())
            "#,
        )
        .bind(session_id)
        .bind(sequence)
        .bind(kind)
    };
    let read = || {
        sqlx::query_as::<_, (i64, i64, Option<String>, Option<i32>)>(
            "SELECT turn_count, tool_call_count, last_turn_status, last_turn_sequence \
             FROM sessions WHERE id = $1",
        )
        .bind(session_id)
    };

    insert(1, "tool.completed")
        .execute(&server.pool)
        .await
        .expect("tool call");
    assert_eq!(
        read().fetch_one(&server.pool).await.expect("read counters"),
        (0, 0, None, None),
        "events inside a turn leave the session row alone"
    );

    // One statement with two turns: each counts the tool calls since the
    // previous one, and the later one wins the pointer. The trailing tool call
    // belongs to the next turn.
    sqlx::query(
        r#"
        INSERT INTO events (id, session_id, sequence, event_type, data, context, ts, created_at)
        SELECT uuidv7(), $1, n, kind, '{}'::jsonb, '{}'::jsonb, NOW(), NOW()
          FROM unnest(
                 ARRAY[2, 3, 4, 5, 6],
                 ARRAY['tool.completed', 'turn.completed', 'output.message.completed',
                       'turn.failed', 'tool.completed']
               ) AS e(n, kind)
        "#,
    )
    .bind(session_id)
    .execute(&server.pool)
    .await
    .expect("seed a batch of events");
    assert_eq!(
        read().fetch_one(&server.pool).await.expect("read counters"),
        (2, 2, Some("failed".to_string()), Some(5))
    );

    insert(9, "turn.completed")
        .execute(&server.pool)
        .await
        .expect("newer turn");
    // An older turn arriving late counts, but does not move the pointer back.
    insert(7, "turn.cancelled")
        .execute(&server.pool)
        .await
        .expect("older turn");
    assert_eq!(
        read().fetch_one(&server.pool).await.expect("read counters"),
        (4, 3, Some("completed".to_string()), Some(9))
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
        SELECT uuidv7(), $1, allocate_event_sequence($1), 'output.message.delta',
               '{}'::jsonb, '{}'::jsonb, NOW(), NOW()
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
        SELECT s.id,
               COALESCE((SELECT es.next_sequence - 1 - es.removed_count
                           FROM event_sequences es WHERE es.session_id = s.id), 0)::BIGINT AS event_count,
               s.task_count, COALESCE(w.file_count, 0) AS workspace_file_count
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

    // Assert the contract, not the plan shape. Which relations the read touches
    // and whether it aggregates are properties of the query; join strategy and
    // scan type are the planner's call and swing with table statistics — a
    // seeded test database has a handful of sessions and a few dozen
    // workspaces, so it will pick hash joins and sequential scans that say
    // nothing about behaviour at production size. Pinning those makes the test
    // fail on a healthy plan, which is exactly what it did.
    assert!(
        !plan_text.contains("events"),
        "session detail read must not touch the events table:\n{plan_text}"
    );
    assert!(
        !plan_text.contains("Aggregate"),
        "session detail read must read counters, not compute them:\n{plan_text}"
    );
}
