//! Companion-owned schema tracking permits additive rollout without changing the
//! running application's _sqlx_migrations history. Platform adoption later runs
//! the same idempotent DDL through its normal migration path.
use crate::{CATALOG_MIGRATION, PARALLEL_MIGRATION, segment::sha256};
use anyhow::{Result, ensure};
use sqlx::PgPool;

pub async fn migrate_catalog(pool: &PgPool) -> Result<usize> {
    let mut tx = everruns_durable::begin_transaction(pool).await?;
    sqlx::query("SET LOCAL lock_timeout='5s'")
        .execute(&mut *tx)
        .await?;
    sqlx::query("SET LOCAL statement_timeout='30s'")
        .execute(&mut *tx)
        .await?;
    sqlx::query(
        "SELECT pg_advisory_xact_lock(hashtextextended('everruns-trace-store-schema',37024))",
    )
    .execute(&mut *tx)
    .await?;
    sqlx::query("CREATE TABLE IF NOT EXISTS trace_store_schema(version BIGINT PRIMARY KEY,checksum TEXT NOT NULL,applied_at TIMESTAMPTZ NOT NULL DEFAULT now())").execute(&mut *tx).await?;
    let known: [(i64, &'static str); 2] = [
        (20261102213413, CATALOG_MIGRATION),
        (20261103015837, PARALLEL_MIGRATION),
    ];
    let recorded: Vec<(i64, String)> =
        sqlx::query_as("SELECT version,checksum FROM trace_store_schema ORDER BY version")
            .fetch_all(&mut *tx)
            .await?;
    for (version, checksum) in &recorded {
        let source = known.iter().find(|(v, _)| v == version);
        ensure!(
            source.is_some_and(|(_, sql)| sha256(sql.as_bytes()) == *checksum),
            "unknown or changed companion catalog migration"
        );
    }
    let mut applied = 0;
    for (version, sql) in known {
        if recorded.iter().any(|(v, _)| *v == version) {
            continue;
        }
        sqlx::raw_sql(sql).execute(&mut *tx).await?;
        sqlx::query("INSERT INTO trace_store_schema(version,checksum) VALUES($1,$2)")
            .bind(version)
            .bind(sha256(sql.as_bytes()))
            .execute(&mut *tx)
            .await?;
        applied += 1;
    }
    everruns_durable::commit_transaction(tx).await?;
    Ok(applied)
}
