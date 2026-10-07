//! A mutating command, its entity history entry and its idempotency record
//! commit together on PostgreSQL, or none of them does
//! (`everruns_server::storage::transaction`).
//!
//! A trigger on `entity_changes` fails the history write for one marker
//! reason, so a test can make exactly its own command's history write fail
//! while other tests share the database.
//!
//! Run with: cargo test -p everruns-server --test domain command_transaction_test::

use crate::test_harness;

use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};

use axum::http::{Method, StatusCode};
use everruns_server::storage::transaction;
use serde_json::{Value, json};
use test_harness::TestServer;

fn unique(prefix: &str) -> String {
    format!(
        "{prefix}-{}",
        uuid::Uuid::now_v7().simple().to_string()[20..].to_owned()
    )
}

/// Makes every history write whose reason is the returned marker fail.
async fn fail_history_for_marker(server: &TestServer) -> String {
    let marker = unique("fail-history");
    let function = format!("test_fail_history_{}", marker.replace('-', "_"));
    sqlx::raw_sql(sqlx::AssertSqlSafe(format!(
        "CREATE FUNCTION {function}() RETURNS trigger LANGUAGE plpgsql AS $$ \
         BEGIN IF NEW.reason = '{marker}' THEN RAISE EXCEPTION 'forced history failure'; END IF; \
         RETURN NEW; END $$; \
         CREATE TRIGGER {function} BEFORE INSERT ON entity_changes \
         FOR EACH ROW EXECUTE FUNCTION {function}();"
    )))
    .execute(&server.pool)
    .await
    .expect("install the history failure trigger");
    marker
}

async fn command(
    server: &TestServer,
    name: &str,
    headers: Vec<(&str, &str)>,
    body: Value,
) -> test_harness::TestResponse {
    let mut headers = headers;
    headers.push(("content-type", "application/json"));
    server
        .request_raw(
            Method::POST,
            &format!("/v1/commands/{name}"),
            headers,
            serde_json::to_vec(&body).unwrap(),
        )
        .await
}

/// Records the top-level transaction id (`txid_current()`, which a savepoint
/// shares with its transaction, unlike `xmin`) of every write to the given
/// rows: `(table, operation, key column, key)`. Returns the log table's name.
async fn log_writing_transactions(
    server: &TestServer,
    rows: &[(&str, &str, &str, &str)],
) -> String {
    let suffix = uuid::Uuid::now_v7().simple().to_string();
    let log = format!("test_txids_{suffix}");
    let function = format!("test_log_txid_{suffix}");
    let mut sql = format!(
        "CREATE TABLE {log} (tbl text NOT NULL, txid bigint NOT NULL); \
         CREATE FUNCTION {function}() RETURNS trigger LANGUAGE plpgsql AS $$ \
         BEGIN INSERT INTO {log} VALUES (TG_TABLE_NAME, txid_current()); RETURN NULL; END $$;"
    );
    for (index, (table, operation, column, key)) in rows.iter().enumerate() {
        sql.push_str(&format!(
            " CREATE TRIGGER {function}_{index} AFTER {operation} ON {table} FOR EACH ROW \
             WHEN (NEW.{column} = '{key}') EXECUTE FUNCTION {function}();"
        ));
    }
    sqlx::raw_sql(sqlx::AssertSqlSafe(sql))
        .execute(&server.pool)
        .await
        .expect("install the transaction log triggers");
    log
}

/// The distinct transactions that wrote each logged table.
async fn writing_transactions(server: &TestServer, log: &str) -> Vec<(String, i64)> {
    sqlx::query_as(sqlx::AssertSqlSafe(format!(
        "SELECT DISTINCT tbl, txid FROM {log} ORDER BY tbl"
    )))
    .fetch_all(&server.pool)
    .await
    .unwrap()
}

async fn history_count(server: &TestServer, entity_ref: &str) -> i64 {
    sqlx::query_scalar("SELECT COUNT(*) FROM entity_changes WHERE entity_ref = $1")
        .bind(entity_ref)
        .fetch_one(&server.pool)
        .await
        .unwrap()
}

async fn workspace_name(server: &TestServer, public_id: &str) -> Option<String> {
    sqlx::query_scalar("SELECT name FROM workspaces WHERE public_id = $1")
        .bind(public_id)
        .fetch_optional(&server.pool)
        .await
        .unwrap()
}

async fn create_workspace(server: &TestServer, name: &str) -> String {
    let created: Value = server
        .post(
            "/v1/commands/create_workspace",
            json!({ "params": { "name": name }, "reason": "a place to test in" }),
        )
        .await
        .assert_success()
        .json();
    created["output"]["id"].as_str().unwrap().to_string()
}

#[tokio::test]
async fn a_change_and_its_history_entry_commit_in_one_transaction() {
    let server = TestServer::new().await;
    let id = create_workspace(&server, &unique("atomic")).await;
    let log = log_writing_transactions(
        &server,
        &[
            ("workspaces", "UPDATE", "public_id", &id),
            ("entity_changes", "INSERT", "entity_ref", &id),
        ],
    )
    .await;
    server
        .post(
            "/v1/commands/update_workspace",
            json!({ "params": { "workspace_id": id, "name": unique("renamed") }, "reason": "rename" }),
        )
        .await
        .assert_success();

    // The row and the entry that records it were written by one transaction.
    let writes = writing_transactions(&server, &log).await;
    assert_eq!(writes.len(), 2, "{writes:?}");
    assert_eq!(writes[0].1, writes[1].1, "{writes:?}");
}

#[tokio::test]
async fn a_failed_history_write_rolls_the_change_back() {
    let server = TestServer::new().await;
    let marker = fail_history_for_marker(&server).await;
    let original = unique("kept");
    let id = create_workspace(&server, &original).await;
    let entries = history_count(&server, &id).await;

    let response = command(
        &server,
        "update_workspace",
        vec![],
        json!({ "params": { "workspace_id": id, "name": unique("lost") }, "reason": marker }),
    )
    .await;
    assert_eq!(response.status(), StatusCode::INTERNAL_SERVER_ERROR);
    assert_eq!(
        workspace_name(&server, &id).await.as_deref(),
        Some(original.as_str())
    );
    assert_eq!(history_count(&server, &id).await, entries);

    // A create leaves nothing behind either.
    let name = unique("never");
    let response = command(
        &server,
        "create_workspace",
        vec![],
        json!({ "params": { "name": name }, "reason": marker }),
    )
    .await;
    assert_eq!(response.status(), StatusCode::INTERNAL_SERVER_ERROR);
    let created: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM workspaces WHERE name = $1")
        .bind(&name)
        .fetch_one(&server.pool)
        .await
        .unwrap();
    assert_eq!(created, 0);
}

#[tokio::test]
async fn the_idempotency_record_commits_with_the_change() {
    let server = TestServer::new().await;
    let marker = fail_history_for_marker(&server).await;
    let id = create_workspace(&server, &unique("idem")).await;

    // Rolled back: the key is released, so the same key may be retried.
    let key = unique("key");
    let response = command(
        &server,
        "update_workspace",
        vec![("idempotency-key", key.as_str())],
        json!({ "params": { "workspace_id": id, "name": unique("lost") }, "reason": marker }),
    )
    .await;
    assert_eq!(response.status(), StatusCode::INTERNAL_SERVER_ERROR);
    let stored: i64 = sqlx::query_scalar(
        "SELECT COUNT(*) FROM command_idempotency_keys WHERE idempotency_key = $1",
    )
    .bind(&key)
    .fetch_one(&server.pool)
    .await
    .unwrap();
    assert_eq!(stored, 0, "a rolled back request releases its key");

    // Committed: the change, its entry and the stored response together.
    let renamed = unique("renamed");
    let body = json!({ "params": { "workspace_id": id, "name": renamed }, "reason": "rename" });
    let log = log_writing_transactions(
        &server,
        &[
            ("workspaces", "UPDATE", "public_id", &id),
            ("entity_changes", "INSERT", "entity_ref", &id),
            (
                "command_idempotency_keys",
                "UPDATE",
                "idempotency_key",
                &key,
            ),
        ],
    )
    .await;
    let first: Value = command(
        &server,
        "update_workspace",
        vec![("idempotency-key", key.as_str())],
        body.clone(),
    )
    .await
    .assert_success()
    .json();
    let writes = writing_transactions(&server, &log).await;
    let tables: Vec<&str> = writes.iter().map(|(table, _)| table.as_str()).collect();
    assert_eq!(
        tables,
        ["command_idempotency_keys", "entity_changes", "workspaces"],
        "each written once: {writes:?}"
    );
    assert!(
        writes.iter().all(|(_, txid)| *txid == writes[0].1),
        "the stored response commits with the change and its entry: {writes:?}"
    );
    let entries = history_count(&server, &id).await;

    let replay = command(
        &server,
        "update_workspace",
        vec![("idempotency-key", key.as_str())],
        body,
    )
    .await;
    assert_eq!(replay.status(), StatusCode::OK);
    assert_eq!(
        replay
            .headers()
            .get("idempotent-replayed")
            .and_then(|v| v.to_str().ok()),
        Some("true")
    );
    let replayed: Value = replay.json();
    assert_eq!(replayed["output"], first["output"]);
    assert_eq!(history_count(&server, &id).await, entries);
    assert_eq!(
        workspace_name(&server, &id).await.as_deref(),
        Some(renamed.as_str())
    );
}

#[tokio::test]
async fn an_event_of_a_rolled_back_command_is_never_stored() {
    let server = TestServer::new().await;
    let marker = fail_history_for_marker(&server).await;
    let session: Value = server
        .post(
            "/v1/sessions",
            json!({ "harness_id": server.seed_base_harness_id, "title": "before" }),
        )
        .await
        .assert_status(StatusCode::CREATED)
        .json();
    let session_id = session["id"].as_str().unwrap().to_string();
    let uuid = uuid::Uuid::parse_str(session_id.trim_start_matches("session_")).unwrap();
    let events = || async {
        sqlx::query_scalar::<_, i64>("SELECT COUNT(*) FROM events WHERE session_id = $1")
            .bind(uuid)
            .fetch_one(&server.pool)
            .await
            .unwrap()
    };
    let before = events().await;

    // The title change emits an event before its history write fails.
    let response = command(
        &server,
        "update_session",
        vec![],
        json!({ "params": { "session_id": session_id, "title": "after" }, "reason": marker }),
    )
    .await;
    assert_eq!(response.status(), StatusCode::INTERNAL_SERVER_ERROR);
    assert_eq!(events().await, before);
    let title: Option<String> = sqlx::query_scalar("SELECT title FROM sessions WHERE id = $1")
        .bind(uuid)
        .fetch_one(&server.pool)
        .await
        .unwrap();
    assert_eq!(title.as_deref(), Some("before"));

    // Without the failure the same change commits its event.
    server
        .post(
            "/v1/commands/update_session",
            json!({ "params": { "session_id": session_id, "title": "after" }, "reason": "ok" }),
        )
        .await
        .assert_success();
    assert!(events().await > before);
}

#[tokio::test]
async fn after_commit_effects_run_on_commit_and_are_dropped_on_rollback() {
    let server = TestServer::new().await;
    let ran = Arc::new(AtomicBool::new(false));

    let flag = ran.clone();
    let result: Result<(), &str> = transaction::scope(
        &server.db,
        async {
            transaction::after_commit(async move { flag.store(true, Ordering::SeqCst) }).await;
            Err("rolled back")
        },
        |_| "begin or commit failed",
    )
    .await;
    assert!(result.is_err());
    assert!(
        !ran.load(Ordering::SeqCst),
        "a rolled back effect never runs"
    );

    let flag = ran.clone();
    let result: Result<(), &str> = transaction::scope(
        &server.db,
        async {
            transaction::after_commit(async move { flag.store(true, Ordering::SeqCst) }).await;
            assert!(!ran.load(Ordering::SeqCst), "deferred until commit");
            Ok(())
        },
        |_| "begin or commit failed",
    )
    .await;
    assert!(result.is_ok());
    assert!(ran.load(Ordering::SeqCst));
}

#[tokio::test]
async fn nested_scopes_and_repository_transactions_share_the_outer_transaction() {
    let server = TestServer::new().await;
    let db = server.db.database();
    let txid = || async {
        sqlx::query_scalar::<_, i64>("SELECT txid_current()")
            .fetch_one(db.tx_pool())
            .await
            .unwrap()
    };

    let result: Result<(), String> = transaction::scope(
        &server.db,
        async {
            let outer = txid().await;
            let nested =
                transaction::scope(&server.db, async { Ok(txid().await) }, |e| e.to_string())
                    .await?;
            assert_eq!(outer, nested, "a nested scope reuses the transaction");

            // A repository transaction becomes a savepoint of the same one.
            let mut savepoint = db.tx_pool().begin().await.unwrap();
            let inside: i64 = sqlx::query_scalar("SELECT txid_current()")
                .fetch_one(&mut *savepoint)
                .await
                .unwrap();
            savepoint.commit().await.unwrap();
            assert_eq!(outer, inside);
            Ok(())
        },
        |e| e.to_string(),
    )
    .await;
    result.unwrap();

    // Outside any scope every query is its own transaction.
    assert_ne!(txid().await, txid().await);
}
