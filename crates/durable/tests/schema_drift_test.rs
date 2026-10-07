//! Drift guard for the crate's standalone PostgreSQL schema.
//!
//! `PostgresWorkflowEventStore::migrate` applies `schema/postgres.sql`, the
//! crate's own copy of the durable tables. The Everruns server creates the same
//! tables with its versioned migrations in `crates/server/migrations/`. These
//! tests fail when the two diverge, so a server migration that touches a
//! durable table must update the crate schema in the same change.
//!
//! The reference is the database at `DATABASE_URL`, which CI migrates with the
//! server migrations before running this suite. The crate schema is applied to
//! a fresh, uniquely named PostgreSQL schema in the same database (the pool's
//! `search_path` points only there), so no extra database or privilege is
//! needed. Both sides are read back from the system catalogs and compared.
//!
//! Requires: `--features postgres-tests` and a server-migrated `DATABASE_URL`.

#![cfg(feature = "postgres-tests")]

use std::collections::BTreeSet;
use std::str::FromStr;

use everruns_durable::persistence::{
    DurableAdmin, EventLog, PostgresWorkflowEventStore, TaskDefinition, TaskQueue, WorkerInfo,
    WorkerRegistry, WorkflowStatus,
};
use everruns_durable::workflow::ActivityOptions;
use serde_json::json;
use sqlx::postgres::{PgConnectOptions, PgPoolOptions};
use sqlx::{AssertSqlSafe, PgPool};
use uuid::Uuid;

/// Durable tables the server creates that are deliberately not part of the
/// crate schema. `durable_tool_results` backs the server's tool-call
/// idempotency storage; nothing in this crate reads or writes it.
const SERVER_ONLY_TABLES: &[&str] = &["durable_tool_results"];

fn database_url() -> String {
    std::env::var("DATABASE_URL").unwrap_or_else(|_| {
        let port = std::env::var("DB_PORT").unwrap_or_else(|_| "9332".to_string());
        format!("postgres://postgres:postgres@localhost:{port}/everruns_test")
    })
}

async fn reference_pool() -> PgPool {
    PgPool::connect(&database_url())
        .await
        .expect("connect to DATABASE_URL")
}

/// A fresh PostgreSQL schema, plus a pool whose `search_path` is only that
/// schema. Dropped with [`ScratchSchema::drop`].
struct ScratchSchema {
    name: String,
    admin: PgPool,
    pool: PgPool,
}

impl ScratchSchema {
    async fn create(admin: &PgPool) -> Self {
        let name = format!("durable_schema_test_{}", Uuid::now_v7().simple());
        // `name` is generated above from a UUID, never external input.
        sqlx::raw_sql(AssertSqlSafe(format!("CREATE SCHEMA {name}")))
            .execute(admin)
            .await
            .expect("create scratch schema");
        let options = PgConnectOptions::from_str(&database_url())
            .expect("parse DATABASE_URL")
            .options([("search_path", name.as_str())]);
        let pool = PgPoolOptions::new()
            .max_connections(4)
            .connect_with(options)
            .await
            .expect("connect scratch pool");
        Self {
            name,
            admin: admin.clone(),
            pool,
        }
    }

    async fn drop(self) {
        self.pool.close().await;
        sqlx::raw_sql(AssertSqlSafe(format!("DROP SCHEMA {} CASCADE", self.name)))
            .execute(&self.admin)
            .await
            .expect("drop scratch schema");
    }
}

/// Everything about the durable tables in the pool's current schema that the
/// store depends on, one normalized line per object, in a stable order.
async fn snapshot(pool: &PgPool) -> BTreeSet<String> {
    let schema: String = sqlx::query_scalar("SELECT current_schema()")
        .fetch_one(pool)
        .await
        .expect("current_schema");
    let excluded: Vec<String> = SERVER_ONLY_TABLES.iter().map(|t| t.to_string()).collect();

    let lines: Vec<String> = sqlx::query_scalar(
        r#"
        WITH tables AS (
            SELECT c.oid, c.relname
            FROM pg_class c
            WHERE c.relnamespace = current_schema()::regnamespace
              AND c.relkind IN ('r', 'p')
              AND c.relname LIKE 'durable\_%'
              AND NOT (c.relname = ANY($1))
        ),
        triggers AS (
            SELECT t.oid, t.tgname, t.tgfoid, tables.relname
            FROM pg_trigger t
            JOIN tables ON tables.oid = t.tgrelid
            WHERE NOT t.tgisinternal
        )
        SELECT 'table ' || relname FROM tables
        UNION ALL
        SELECT format(
            'column %s.%s %s not_null=%s default=%s identity=%s',
            tables.relname, a.attname, format_type(a.atttypid, a.atttypmod),
            a.attnotnull, pg_get_expr(d.adbin, d.adrelid), a.attidentity
        )
        FROM tables
        JOIN pg_attribute a ON a.attrelid = tables.oid AND a.attnum > 0 AND NOT a.attisdropped
        LEFT JOIN pg_attrdef d ON d.adrelid = a.attrelid AND d.adnum = a.attnum
        UNION ALL
        SELECT format('constraint %s.%s %s', tables.relname, con.conname, pg_get_constraintdef(con.oid))
        FROM tables
        JOIN pg_constraint con ON con.conrelid = tables.oid
        UNION ALL
        SELECT format('index %s', pg_get_indexdef(i.indexrelid))
        FROM tables
        JOIN pg_index i ON i.indrelid = tables.oid
        UNION ALL
        SELECT format('trigger %s', pg_get_triggerdef(triggers.oid)) FROM triggers
        UNION ALL
        SELECT DISTINCT format(
            'trigger_function %s %s', p.proname, btrim(regexp_replace(p.prosrc, '\s+', ' ', 'g'))
        )
        FROM triggers
        JOIN pg_proc p ON p.oid = triggers.tgfoid
        UNION ALL
        SELECT format(
            'sequence %s %s start=%s increment=%s cycle=%s',
            sc.relname, format_type(s.seqtypid, NULL), s.seqstart, s.seqincrement, s.seqcycle
        )
        FROM pg_sequence s
        JOIN pg_class sc ON sc.oid = s.seqrelid
        WHERE sc.relnamespace = current_schema()::regnamespace
          AND sc.relname LIKE 'durable\_%'
          AND NOT EXISTS (
              SELECT 1 FROM unnest($1::text[]) AS ex(name) WHERE sc.relname LIKE ex.name || '\_%'
          )
        "#,
    )
    .bind(&excluded)
    .fetch_all(pool)
    .await
    .expect("read durable catalog");

    let mut set: BTreeSet<String> = lines
        .into_iter()
        // pg_get_*def qualify names outside the search path; both sides read
        // their own schema, so drop the qualifier to compare like with like.
        .map(|line| line.replace(&format!("{schema}."), ""))
        .collect();

    let counters: Vec<String> = sqlx::query_scalar(
        "SELECT name || ' shard ' || shard FROM durable_stat_counters ORDER BY name, shard",
    )
    .fetch_all(pool)
    .await
    .expect("read durable_stat_counters");
    set.extend(
        counters
            .into_iter()
            .map(|name| format!("counter_row {name}")),
    );
    set
}

fn assert_same(reference: &BTreeSet<String>, actual: &BTreeSet<String>, what: &str) {
    let missing: Vec<_> = reference.difference(actual).collect();
    let extra: Vec<_> = actual.difference(reference).collect();
    assert!(
        missing.is_empty() && extra.is_empty(),
        "{what}\n  only in server migrations:\n    {}\n  only in crate schema:\n    {}",
        missing
            .iter()
            .map(|s| s.as_str())
            .collect::<Vec<_>>()
            .join("\n    "),
        extra
            .iter()
            .map(|s| s.as_str())
            .collect::<Vec<_>>()
            .join("\n    "),
    );
}

#[tokio::test]
async fn crate_schema_matches_server_migrations() {
    let reference = reference_pool().await;
    let server = snapshot(&reference).await;
    assert!(
        server.contains("table durable_workflow_instances"),
        "DATABASE_URL has no durable tables; run `sqlx migrate run --source crates/server/migrations` first"
    );

    let scratch = ScratchSchema::create(&reference).await;
    PostgresWorkflowEventStore::migrate(&scratch.pool)
        .await
        .expect("apply crate schema to an empty schema");
    let first = snapshot(&scratch.pool).await;
    // Idempotent: a second run (every process start-up) changes nothing.
    PostgresWorkflowEventStore::migrate(&scratch.pool)
        .await
        .expect("re-apply crate schema");
    let second = snapshot(&scratch.pool).await;
    scratch.drop().await;

    assert_same(
        &server,
        &first,
        "crate schema/postgres.sql drifted from the server migrations for the durable tables; \
         update crates/durable/schema/postgres.sql to match",
    );
    assert_same(&first, &second, "re-applying the crate schema changed it");
}

#[tokio::test]
async fn migrate_is_a_no_op_on_a_server_migrated_database() {
    let reference = reference_pool().await;
    let before = snapshot(&reference).await;
    PostgresWorkflowEventStore::migrate(&reference)
        .await
        .expect("apply crate schema over server migrations");
    let after = snapshot(&reference).await;
    assert_same(
        &before,
        &after,
        "migrate() changed a database the server migrations already created",
    );
}

#[tokio::test]
async fn store_runs_on_a_database_created_by_migrate() {
    let reference = reference_pool().await;
    let scratch = ScratchSchema::create(&reference).await;
    PostgresWorkflowEventStore::migrate(&scratch.pool)
        .await
        .expect("apply crate schema");
    let store = PostgresWorkflowEventStore::new(scratch.pool.clone());

    let workflow_id = Uuid::now_v7();
    store
        .start_workflow_with_task(
            workflow_id,
            "schema_smoke",
            json!({ "n": 1 }),
            TaskDefinition {
                workflow_id: Some(workflow_id),
                activity_id: "step-1".into(),
                activity_type: "smoke".into(),
                input: json!({ "n": 1 }),
                options: ActivityOptions::default(),
            },
        )
        .await
        .expect("start workflow with task");

    let types = vec!["smoke".to_string()];
    store
        .register_worker(WorkerInfo::new("schema-worker", types.clone()))
        .await
        .expect("register worker");
    let claimed = store
        .claim_task("schema-worker", &types, 10)
        .await
        .expect("claim task");
    assert_eq!(claimed.len(), 1, "the enqueued task is claimable");
    store
        .complete_task(claimed[0].id, "schema-worker", json!({ "ok": true }))
        .await
        .expect("complete task");
    store
        .update_workflow_status(
            workflow_id,
            WorkflowStatus::Completed,
            Some(json!({ "ok": true })),
            None,
        )
        .await
        .expect("complete workflow");

    let info = store.get_workflow_info(workflow_id).await.expect("info");
    assert_eq!(info.status, WorkflowStatus::Completed);
    let health = store.get_system_health().await.expect("health");
    assert_eq!(health.completed_tasks, 1, "task counter trigger fired");
    assert_eq!(health.started_tasks, 1, "claim counter trigger fired");
    assert_eq!(
        health.completed_workflows, 1,
        "workflow counter trigger fired"
    );

    scratch.drop().await;
}

/// A database that took the crate schema before the counters were sharded
/// keeps its totals and gains the shard rows when `migrate` runs again.
#[tokio::test]
async fn migrate_shards_unsharded_counters_and_keeps_their_totals() {
    let reference = reference_pool().await;
    let scratch = ScratchSchema::create(&reference).await;
    sqlx::raw_sql(
        "CREATE TABLE durable_stat_counters (
             name TEXT PRIMARY KEY,
             value BIGINT NOT NULL DEFAULT 0 CHECK (value >= 0)
         );
         INSERT INTO durable_stat_counters (name, value) VALUES
             ('tasks_completed', 7), ('tasks_failed', 0), ('tasks_started', 9),
             ('workflows_completed', 3), ('workflows_failed', 1), ('workflows_started', 4);",
    )
    .execute(&scratch.pool)
    .await
    .expect("create the unsharded counters");

    PostgresWorkflowEventStore::migrate(&scratch.pool)
        .await
        .expect("apply crate schema over unsharded counters");

    let shards: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM durable_stat_counters")
        .fetch_one(&scratch.pool)
        .await
        .unwrap();
    assert_eq!(shards, 6 * 16, "every counter has 16 shard rows");
    let health = PostgresWorkflowEventStore::new(scratch.pool.clone())
        .get_system_health()
        .await
        .expect("health");
    assert_eq!(health.completed_tasks, 7);
    assert_eq!(health.started_tasks, 9);
    assert_eq!(health.completed_workflows, 3);
    assert_eq!(health.failed_workflows, 1);
    assert_eq!(health.started_workflows, 4);

    scratch.drop().await;
}
