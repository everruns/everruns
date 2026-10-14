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
use sqlx::postgres::PgConnectOptions;
use sqlx::{Acquire, ConnectOptions, PgConnection, PgPool};

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
    run_without_statement_logging(pool, &migrator()).await
}

/// Run `migrator` on a dedicated connection without sqlx's statement logging.
///
/// A migration that takes over a second (an index build, a data backfill) is
/// expected, yet the pool's default logs it as a `slow statement` warning,
/// which production reports as a Sentry issue on every such release
/// (EVERRUNS-2N, EVERRUNS-2V). Each migration's duration is still kept in
/// `_sqlx_migrations.execution_time`.
async fn run_without_statement_logging(pool: &PgPool, migrator: &Migrator) -> Result<()> {
    let mut conn = migration_connect_options(pool)
        .connect()
        .await
        .context("Could not connect to the database to run migrations")?;
    let migrated = migrator
        .run(&mut conn)
        .await
        .context("Database migration failed - check migration files and database state");
    // Close cleanly either way; a failed close must not mask a failed migration.
    let _ = sqlx::Connection::close(conn).await;
    migrated
}

fn migration_connect_options(pool: &PgPool) -> PgConnectOptions {
    (*pool.connect_options())
        .clone()
        .disable_statement_logging()
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

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::{Arc, Mutex};
    use tracing_subscriber::layer::SubscriberExt;
    use tracing_subscriber::{Layer, Registry};

    /// Counts `sqlx::query` warnings, the events sqlx emits for a slow statement.
    #[derive(Clone, Default)]
    struct SlowStatementWarnings(Arc<Mutex<usize>>);

    impl<S: tracing::Subscriber> Layer<S> for SlowStatementWarnings {
        fn on_event(
            &self,
            event: &tracing::Event<'_>,
            _ctx: tracing_subscriber::layer::Context<'_, S>,
        ) {
            let metadata = event.metadata();
            if metadata.target() == "sqlx::query" && *metadata.level() == tracing::Level::WARN {
                *self.0.lock().unwrap() += 1;
            }
        }
    }

    impl SlowStatementWarnings {
        fn take(&self) -> usize {
            std::mem::take(&mut *self.0.lock().unwrap())
        }
    }

    #[tokio::test]
    async fn a_slow_migration_is_not_reported_as_a_slow_statement() {
        let (pool, _db) = crate::storage::test_database::create_test_database();
        let dir = tempfile::tempdir().unwrap();
        // Past sqlx's default one-second slow-statement threshold.
        std::fs::write(
            dir.path().join("29991231000000_slow.sql"),
            "SELECT pg_sleep(1.2);",
        )
        .unwrap();
        let mut slow = Migrator::new(dir.path()).await.unwrap();
        // The database already carries the real migrations this one omits.
        slow.set_ignore_missing(true);

        let warnings = SlowStatementWarnings::default();
        let _guard = tracing::subscriber::set_default(Registry::default().with(warnings.clone()));

        // Control: the pool itself does report a statement this slow.
        sqlx::query("SELECT pg_sleep(1.2)")
            .execute(&pool)
            .await
            .unwrap();
        assert_eq!(warnings.take(), 1, "the pool warns about a slow statement");

        run_without_statement_logging(&pool, &slow).await.unwrap();
        assert_eq!(warnings.take(), 0, "a slow migration is not a warning");
        let applied: bool = sqlx::query_scalar(
            "SELECT EXISTS (SELECT 1 FROM _sqlx_migrations WHERE version = 29991231000000)",
        )
        .fetch_one(&pool)
        .await
        .unwrap();
        assert!(applied, "the migration still ran");
    }
}
