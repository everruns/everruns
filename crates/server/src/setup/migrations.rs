//! Server schema migrations.
//!
//! On 2026-10-11 the sequential migrations 001..222 were squashed into one
//! baseline, `migrations/20261011000000_baseline.sql`, generated with pg_dump
//! from a database that ran all 222. New migrations are named by UTC timestamp
//! (`just new-migration`), so parallel branches never pick the same version.
//!
//! A fresh database runs the baseline. A database that ran the legacy history
//! is adopted instead: any legacy migrations it is still missing are applied
//! from `migrations/legacy/`, then its legacy rows in `_sqlx_migrations` are
//! replaced by one row for the baseline, so the baseline never runs there.
//! Drop the legacy directory and the adoption once no deployment can still be
//! on the legacy history.
//!
//! Every path that migrates a real database goes through [`run`]: server
//! startup, and the `everruns-migrate` binary the admin container uses.

use anyhow::{Context, Result, bail};
use sqlx::migrate::Migrator;
use sqlx::{Acquire, PgConnection, PgPool};

/// Version of the baseline migration.
pub const BASELINE_VERSION: i64 = 20261011000000;

/// Last version of the legacy sequential history the baseline replaces.
pub const LEGACY_LAST_VERSION: i64 = 222;

/// Advisory lock that serializes adoption across servers starting together.
/// sqlx's own migration lock uses a different key, so holding both on one
/// connection is fine.
const ADOPTION_LOCK_KEY: i64 = 0x6576_6572_5f62_6173; // "ever_bas"

/// The current migrations: the baseline and every timestamped migration.
pub fn migrator() -> Migrator {
    sqlx::migrate!("./migrations")
}

/// The legacy sequential history, 001..222. Only adoption and tests of the
/// historical data conversions use it.
pub fn legacy_migrator() -> Migrator {
    let mut migrator = sqlx::migrate!("./migrations/legacy");
    // An adopted database also carries timestamped versions it does not know.
    migrator.set_ignore_missing(true);
    migrator
}

/// Migrate `pool`'s database to the current schema.
pub async fn run(pool: &PgPool) -> Result<()> {
    adopt_legacy_history(pool).await?;
    migrator()
        .run(pool)
        .await
        .context("Database migration failed - check migration files and database state")
}

/// Replace a legacy history with the baseline, completing it first if the
/// database stopped partway through. A no-op on fresh or adopted databases.
pub async fn adopt_legacy_history(pool: &PgPool) -> Result<()> {
    let mut conn = pool.acquire().await?;
    let conn: &mut PgConnection = &mut conn;

    if legacy_row_count(conn).await? == 0 {
        return Ok(());
    }

    sqlx::query("SELECT pg_advisory_lock($1)")
        .bind(ADOPTION_LOCK_KEY)
        .execute(&mut *conn)
        .await?;
    let adopted = adopt_locked(conn).await;
    sqlx::query("SELECT pg_advisory_unlock($1)")
        .bind(ADOPTION_LOCK_KEY)
        .execute(&mut *conn)
        .await?;
    adopted
}

async fn adopt_locked(conn: &mut PgConnection) -> Result<()> {
    // Another server may have adopted while this one waited for the lock.
    if legacy_row_count(conn).await? == 0 {
        return Ok(());
    }

    // `run_direct` on the concrete connection rather than the generic `run`,
    // whose `Acquire` bound makes the future fail higher-ranked `Send` checks.
    legacy_migrator()
        .run_direct(None, &mut *conn, false)
        .await
        .context("Completing the legacy migration history before adopting the baseline")?;

    let mut tx = conn.begin().await?;
    let (applied, failed): (i64, i64) = sqlx::query_as(
        "SELECT count(*), count(*) FILTER (WHERE NOT success)
         FROM _sqlx_migrations WHERE version <= $1",
    )
    .bind(LEGACY_LAST_VERSION)
    .fetch_one(&mut *tx)
    .await?;
    if applied != LEGACY_LAST_VERSION || failed != 0 {
        bail!(
            "legacy migration history is incomplete ({applied} applied, {failed} failed of \
             {LEGACY_LAST_VERSION}); refusing to adopt the baseline"
        );
    }

    let baseline = migrator();
    let baseline = baseline
        .iter()
        .find(|migration| migration.version == BASELINE_VERSION)
        .context("baseline migration is missing")?;

    sqlx::query("DELETE FROM _sqlx_migrations WHERE version <= $1")
        .bind(LEGACY_LAST_VERSION)
        .execute(&mut *tx)
        .await?;
    sqlx::query(
        "INSERT INTO _sqlx_migrations (version, description, success, checksum, execution_time)
         VALUES ($1, $2, TRUE, $3, 0)",
    )
    .bind(baseline.version)
    .bind(&*baseline.description)
    .bind(&*baseline.checksum)
    .execute(&mut *tx)
    .await?;
    tx.commit().await?;

    tracing::info!(
        "Adopted the migration baseline: legacy migrations 001..{LEGACY_LAST_VERSION} \
         replaced by version {BASELINE_VERSION}"
    );
    Ok(())
}

async fn legacy_row_count(conn: &mut PgConnection) -> Result<i64> {
    let exists: bool = sqlx::query_scalar("SELECT to_regclass('_sqlx_migrations') IS NOT NULL")
        .fetch_one(&mut *conn)
        .await?;
    if !exists {
        return Ok(0);
    }
    Ok(
        sqlx::query_scalar("SELECT count(*) FROM _sqlx_migrations WHERE version <= $1")
            .bind(LEGACY_LAST_VERSION)
            .fetch_one(&mut *conn)
            .await?,
    )
}
