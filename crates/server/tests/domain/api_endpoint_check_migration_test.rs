//! Migration `20261012191427_tighten_channel_type_drop_api_endpoint`: the
//! channel_type CHECK stops admitting the retired `api_endpoint` type, on each
//! table that holds no such row, and stays as it was on one that still does
//! (the startup conversion has not reached it yet). Runs the migration SQL
//! against temporary stand-ins for `agent_channels` and `apps` that carry the
//! pre-migration CHECK.

use crate::test_harness;

use sqlx::{Connection, Executor, PgConnection, Postgres, Transaction};

const MIGRATION: &str =
    include_str!("../../migrations/20261012191427_tighten_channel_type_drop_api_endpoint.sql");

/// Temporary `agent_channels` and `apps` with the CHECK the baseline created.
async fn legacy_tables(tx: &mut Transaction<'_, Postgres>) {
    tx.execute("SET LOCAL search_path TO pg_temp, public")
        .await
        .expect("isolate the migration's tables");
    let types = "'slack', 'ag_ui', 'schedule', 'webhook', 'a2a', 'fcp', 'api_endpoint', \
                 'public_chat', 'voice', 'api', 'poppy'";
    for table in ["agent_channels", "apps"] {
        let ddl = format!(
            "CREATE TEMPORARY TABLE {table} (id SERIAL PRIMARY KEY, channel_type VARCHAR(50), \
             CONSTRAINT {table}_channel_type_check CHECK (channel_type IN ({types})))"
        );
        sqlx::raw_sql(sqlx::AssertSqlSafe(ddl))
            .execute(&mut **tx)
            .await
            .expect("create legacy table");
    }
}

/// Whether `table` accepts a new row of `channel_type`, tried in a savepoint.
async fn accepts(tx: &mut Transaction<'_, Postgres>, table: &str, channel_type: &str) -> bool {
    let mut savepoint = tx.begin().await.expect("savepoint");
    let insert = format!("INSERT INTO {table} (channel_type) VALUES ($1)");
    let result = sqlx::query(sqlx::AssertSqlSafe(insert))
        .bind(channel_type)
        .execute(&mut *savepoint)
        .await;
    savepoint.rollback().await.expect("roll back savepoint");
    match result {
        Ok(_) => true,
        Err(error) => {
            assert!(
                error
                    .to_string()
                    .contains(&format!("{table}_channel_type_check")),
                "{error}"
            );
            false
        }
    }
}

#[tokio::test]
async fn without_leftovers_both_tables_refuse_api_endpoint() {
    let mut connection = PgConnection::connect(&test_harness::get_database_url())
        .await
        .expect("connect to PostgreSQL");
    let mut tx = connection.begin().await.expect("begin");
    legacy_tables(&mut tx).await;
    sqlx::raw_sql(MIGRATION)
        .execute(&mut *tx)
        .await
        .expect("run migration");
    for table in ["agent_channels", "apps"] {
        assert!(!accepts(&mut tx, table, "api_endpoint").await, "{table}");
        for kept in ["api", "a2a", "slack", "poppy"] {
            assert!(accepts(&mut tx, table, kept).await, "{table} {kept}");
        }
    }
    // Idempotent: a second run leaves the same constraint.
    sqlx::raw_sql(MIGRATION)
        .execute(&mut *tx)
        .await
        .expect("re-run migration");
    assert!(!accepts(&mut tx, "agent_channels", "api_endpoint").await);
    tx.rollback().await.expect("roll back");
}

#[tokio::test]
async fn an_unconverted_channel_keeps_its_table_loose_until_converted() {
    let mut connection = PgConnection::connect(&test_harness::get_database_url())
        .await
        .expect("connect to PostgreSQL");
    let mut tx = connection.begin().await.expect("begin");
    legacy_tables(&mut tx).await;
    sqlx::query("INSERT INTO agent_channels (channel_type) VALUES ('api_endpoint')")
        .execute(&mut *tx)
        .await
        .expect("seed an unconverted channel");

    // Does not fail the deploy, and leaves the row and its CHECK alone so the
    // startup conversion can still read and rewrite it.
    sqlx::raw_sql(MIGRATION)
        .execute(&mut *tx)
        .await
        .expect("run migration with a leftover");
    assert!(accepts(&mut tx, "agent_channels", "api_endpoint").await);
    let left: i64 = sqlx::query_scalar(
        "SELECT count(*) FROM agent_channels WHERE channel_type = 'api_endpoint'",
    )
    .fetch_one(&mut *tx)
    .await
    .unwrap();
    assert_eq!(left, 1);
    // An archiving UPDATE that keeps the type still works on it.
    sqlx::query("UPDATE agent_channels SET channel_type = channel_type")
        .execute(&mut *tx)
        .await
        .expect("update the leftover");
    // `apps` holds no leftover, so it is tightened on its own.
    assert!(!accepts(&mut tx, "apps", "api_endpoint").await);

    // Re-run after the conversion (as a later migration settling leftovers
    // would), it tightens the table.
    sqlx::query("UPDATE agent_channels SET channel_type = 'api'")
        .execute(&mut *tx)
        .await
        .unwrap();
    sqlx::raw_sql(MIGRATION)
        .execute(&mut *tx)
        .await
        .expect("re-run migration");
    assert!(!accepts(&mut tx, "agent_channels", "api_endpoint").await);
    tx.rollback().await.expect("roll back");
}
