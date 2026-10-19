//! The 2026-10-11 baseline replaces the legacy history 001..222
//! (crates/server/src/setup/migrations.rs).
//!
//! A fresh database runs the baseline and ends up with the same schema and
//! seed rows as one that replayed the legacy history. A database that ran the
//! legacy history, fully or partway, is adopted: it ends with one baseline row
//! in `_sqlx_migrations` and never runs the baseline SQL. Adoption is a no-op
//! the second time.
//!
//! Run with: cargo test -p everruns-server --test domain migration_baseline_test

use crate::test_harness::database_migrated_before;

use everruns_server::setup::migrations::{
    BASELINE_VERSION, LEGACY_LAST_VERSION, legacy_migrator, run,
};
use sqlx::PgPool;

/// Everything the schema defines, one sorted line per object. Expressions are
/// normalized because PostgreSQL deparses `x IN ('a', 'b')` on a varchar
/// column differently once pg_dump output is parsed again; the casts and
/// parentheses it shuffles do not change meaning.
async fn schema_snapshot(pool: &PgPool) -> Vec<String> {
    sqlx::query_scalar(
        r#"
        WITH objects AS (
            SELECT 'column ' || c.relname || '.' || a.attname || ' ' ||
                   format_type(a.atttypid, a.atttypmod) ||
                   CASE WHEN a.attnotnull THEN ' not null' ELSE '' END ||
                   coalesce(' default ' || pg_get_expr(d.adbin, d.adrelid), '') ||
                   CASE WHEN a.attgenerated <> '' THEN ' generated' ELSE '' END AS line
            FROM pg_attribute a
            JOIN pg_class c ON c.oid = a.attrelid
            JOIN pg_namespace n ON n.oid = c.relnamespace
            LEFT JOIN pg_attrdef d ON d.adrelid = a.attrelid AND d.adnum = a.attnum
            WHERE n.nspname = 'public' AND c.relkind IN ('r', 'p', 'v', 'm')
              AND a.attnum > 0 AND NOT a.attisdropped AND c.relname <> '_sqlx_migrations'
            UNION ALL
            SELECT 'constraint ' || cl.relname || '.' || co.conname || ' ' ||
                   pg_get_constraintdef(co.oid)
            FROM pg_constraint co
            JOIN pg_class cl ON cl.oid = co.conrelid
            JOIN pg_namespace n ON n.oid = cl.relnamespace
            WHERE n.nspname = 'public' AND cl.relname <> '_sqlx_migrations'
            UNION ALL
            SELECT 'index ' || indexdef FROM pg_indexes
            WHERE schemaname = 'public' AND tablename <> '_sqlx_migrations'
            UNION ALL
            SELECT 'trigger ' || pg_get_triggerdef(t.oid)
            FROM pg_trigger t
            JOIN pg_class c ON c.oid = t.tgrelid
            JOIN pg_namespace n ON n.oid = c.relnamespace
            WHERE n.nspname = 'public' AND NOT t.tgisinternal
            UNION ALL
            SELECT 'function ' || p.proname || ' ' || md5(pg_get_functiondef(p.oid))
            FROM pg_proc p
            JOIN pg_namespace n ON n.oid = p.pronamespace
            LEFT JOIN pg_depend dep ON dep.objid = p.oid AND dep.deptype = 'e'
            WHERE n.nspname = 'public' AND dep.objid IS NULL
            UNION ALL
            SELECT 'view ' || viewname || ' ' || definition FROM pg_views
            WHERE schemaname = 'public'
            UNION ALL
            SELECT 'sequence ' || sequencename || ' ' || data_type::text || ' ' ||
                   coalesce(last_value::text, 'unused')
            FROM pg_sequences WHERE schemaname = 'public'
            UNION ALL
            SELECT 'extension ' || extname FROM pg_extension
            UNION ALL
            SELECT 'type ' || t.typname || ' ' ||
                   coalesce(string_agg(e.enumlabel, ',' ORDER BY e.enumsortorder), '')
            FROM pg_type t
            JOIN pg_namespace n ON n.oid = t.typnamespace
            LEFT JOIN pg_enum e ON e.enumtypid = t.oid
            WHERE n.nspname = 'public' AND t.typtype IN ('e', 'd', 'c')
              AND NOT EXISTS (SELECT 1 FROM pg_class c WHERE c.reltype = t.oid)
            GROUP BY t.typname
        )
        SELECT regexp_replace(line, '::(character varying|text)(\[\])?|[()]', '', 'g')
        FROM objects ORDER BY 1
        "#,
    )
    .fetch_all(pool)
    .await
    .expect("read schema snapshot")
}

/// Row count of every table, for the rows the migrations seed.
async fn row_counts(pool: &PgPool) -> Vec<(String, i64)> {
    sqlx::query_as(
        r#"
        SELECT tablename::text,
               (xpath('/row/c/text()',
                      query_to_xml(format('SELECT count(*) AS c FROM public.%I', tablename),
                                   false, true, '')))[1]::text::bigint
        FROM pg_tables
        WHERE schemaname = 'public' AND tablename <> '_sqlx_migrations'
        ORDER BY 1
        "#,
    )
    .fetch_all(pool)
    .await
    .expect("count rows")
}

async fn applied_versions(pool: &PgPool) -> Vec<i64> {
    sqlx::query_scalar("SELECT version FROM _sqlx_migrations ORDER BY version")
        .fetch_all(pool)
        .await
        .expect("read applied migrations")
}

#[tokio::test]
async fn baseline_matches_the_legacy_history_and_adoption_keeps_it() {
    // Fresh database: runs the baseline and the migrations after it.
    let fresh = database_migrated_before(1).await;
    run(&fresh.pool).await.expect("migrate a fresh database");
    let fresh_versions = applied_versions(&fresh.pool).await;
    assert_eq!(fresh_versions[0], BASELINE_VERSION);
    assert!(fresh_versions.iter().all(|v| *v >= BASELINE_VERSION));

    // Legacy database: replayed 001..222, then adopted.
    let legacy = database_migrated_before(LEGACY_LAST_VERSION + 1).await;
    assert_eq!(
        applied_versions(&legacy.pool).await.len() as i64,
        LEGACY_LAST_VERSION
    );
    run(&legacy.pool).await.expect("adopt a legacy database");
    // Compared after adoption, so both sides also ran every migration that
    // came after the baseline.
    let legacy_schema = schema_snapshot(&legacy.pool).await;
    let legacy_rows = row_counts(&legacy.pool).await;
    assert_eq!(applied_versions(&legacy.pool).await, fresh_versions);
    run(&legacy.pool)
        .await
        .expect("migrating an adopted database again is a no-op");
    assert_eq!(applied_versions(&legacy.pool).await, fresh_versions);

    let fresh_schema = schema_snapshot(&fresh.pool).await;
    let only_legacy: Vec<_> = legacy_schema
        .iter()
        .filter(|line| !fresh_schema.contains(line))
        .collect();
    let only_baseline: Vec<_> = fresh_schema
        .iter()
        .filter(|line| !legacy_schema.contains(line))
        .collect();
    assert!(
        only_legacy.is_empty() && only_baseline.is_empty(),
        "baseline schema differs from the legacy history\nonly legacy: {only_legacy:#?}\nonly baseline: {only_baseline:#?}"
    );
    assert_eq!(row_counts(&fresh.pool).await, legacy_rows);
}

#[tokio::test]
async fn a_partial_legacy_history_is_completed_then_adopted() {
    let partial = database_migrated_before(200).await;
    assert_eq!(applied_versions(&partial.pool).await.len(), 199);

    run(&partial.pool)
        .await
        .expect("adopt a partial legacy database");

    let versions = applied_versions(&partial.pool).await;
    assert_eq!(versions[0], BASELINE_VERSION);
    assert!(versions.iter().all(|v| *v >= BASELINE_VERSION));
    // The legacy migrations it was missing ran before adoption.
    let poppy_sessions: bool =
        sqlx::query_scalar("SELECT to_regclass('poppy_sessions') IS NOT NULL")
            .fetch_one(&partial.pool)
            .await
            .unwrap();
    assert!(poppy_sessions);
}

#[tokio::test]
async fn the_legacy_migrator_still_holds_the_full_history() {
    let migrator = legacy_migrator();
    let versions: Vec<i64> = migrator.iter().map(|m| m.version).collect();
    assert_eq!(versions, (1..=LEGACY_LAST_VERSION).collect::<Vec<_>>());
}
