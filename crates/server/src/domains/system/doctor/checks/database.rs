use super::{Findings, plural};
use crate::domains::system::doctor::{CheckArea, DoctorCheck, SystemDoctor};
use crate::setup::migrations::{self, LEGACY_LAST_VERSION};
use sqlx::PgPool;
use std::collections::HashSet;
use std::time::Instant;

const AREA: CheckArea = CheckArea::Database;

/// The oldest PostgreSQL the schema is tested on (`docs/getting-started/kubernetes.md`).
const MIN_POSTGRES: i64 = 150000;

/// A transaction open this long holds locks and blocks vacuum.
const LONG_TRANSACTION_MINUTES: i64 = 5;

pub(super) async fn run(doctor: &SystemDoctor) -> Findings {
    let mut found = Findings::default();
    let pool = doctor.db.pool();

    let reachable = DoctorCheck::new(AREA, "database.reachable", "Database reachable");
    let started = Instant::now();
    let version: Result<(String, String), sqlx::Error> = sqlx::query_as(
        "SELECT current_setting('server_version'), current_setting('server_version_num')",
    )
    .fetch_one(pool)
    .await;
    let (version, version_num) = match version {
        Ok(version) => version,
        Err(error) => {
            found.check(
                reachable
                    .problem(format!(
                        "The database does not answer: {}.",
                        sql_error(&error)
                    ))
                    .why("Nothing works without it: sign-in, sessions and workers all stop.")
                    .fix("Check that PostgreSQL is running and DATABASE_URL points at it."),
            );
            return found;
        }
    };
    let elapsed = started.elapsed().as_millis();
    // `server_version` may carry a distribution suffix: "16.4 (Ubuntu ...)".
    let version = version
        .split_whitespace()
        .next()
        .unwrap_or_default()
        .to_string();
    found.fact(format!("PostgreSQL {version}"));
    found.check(if elapsed > 500 {
        reachable
            .warning(format!("The database answered, but took {elapsed} ms."))
            .why("Every request waits on the database, so the whole app feels slow.")
            .fix("Check the database's load and the network between it and the server.")
    } else {
        reachable.ok(format!("The database answered in {elapsed} ms."))
    });

    let supported = DoctorCheck::new(AREA, "database.version", "PostgreSQL version");
    let version_num: i64 = version_num.parse().unwrap_or_default();
    found.check(if version_num < MIN_POSTGRES {
        supported
            .problem(format!("PostgreSQL {version} is older than 15."))
            .why("The schema relies on features of PostgreSQL 15 and later.")
            .fix("Upgrade PostgreSQL to 17.")
    } else {
        supported.ok(format!("PostgreSQL {version} is supported."))
    });

    found.check(migrations_check(pool).await);
    found.check(pool_check(doctor));
    found.check(long_transactions_check(pool).await);
    found
}

async fn migrations_check(pool: &PgPool) -> DoctorCheck {
    let check = DoctorCheck::new(AREA, "database.migrations", "Migrations applied");
    let applied: Vec<(i64, bool)> =
        match sqlx::query_as("SELECT version, success FROM _sqlx_migrations")
            .fetch_all(pool)
            .await
        {
            Ok(rows) => rows,
            Err(error) => {
                return check
                    .problem(format!(
                        "The migration history cannot be read: {}.",
                        sql_error(&error)
                    ))
                    .why("The schema may be missing tables the server needs.")
                    .fix("Restart the server; it applies migrations on startup.");
            }
        };
    let judged = judge_migrations(&applied, &expected_versions());
    match judged {
        MigrationState::Current(count) => check.ok(format!("All {count} migrations are applied.")),
        MigrationState::Failed(versions) => check
            .problem(format!("Migration {} failed partway.", list(&versions)))
            .why("The schema is in an unknown state; the server will not start again.")
            .fix("Fix the failure from the server log, then restart the server."),
        MigrationState::Missing(versions) => check
            .problem(format!(
                "{} not applied: {}.",
                plural(versions.len(), "migration is", "migrations are"),
                list(&versions)
            ))
            .why("The server expects tables and columns that are not there yet.")
            .fix("Restart the server, or run `everruns-migrate`."),
        MigrationState::Newer(versions) => check
            .warning(format!(
                "The database has {} this build does not know: {}.",
                plural(versions.len(), "migration", "migrations"),
                list(&versions)
            ))
            .why(
                "A newer version of Everruns migrated this database. This older build \
                 may misread it, and going back needs a backup taken before the upgrade.",
            )
            .fix(
                "Run the version that migrated the database. If an app embedding \
                 Everruns keeps its own migrations in `_sqlx_migrations`, move them to \
                 their own table.",
            ),
        MigrationState::Legacy => check
            .warning("The database still carries the legacy migration history.")
            .why("It is adopted onto the baseline when the server starts.")
            .fix("Restart the server to finish the adoption."),
    }
}

/// Whether this server can serve traffic: the database answers and its
/// schema is the one this build expects. The orchestrator readiness probe.
pub(crate) async fn readiness(pool: &PgPool) -> Result<(), &'static str> {
    let applied: Vec<(i64, bool)> = sqlx::query_as("SELECT version, success FROM _sqlx_migrations")
        .fetch_all(pool)
        .await
        .map_err(|_| "database unreachable")?;
    match judge_migrations(&applied, &expected_versions()) {
        // A newer schema is served by the newer replicas of a rolling deploy;
        // this one keeps serving rather than dropping out mid-rollout.
        MigrationState::Current(_) | MigrationState::Newer(_) => Ok(()),
        _ => Err("migrations not applied"),
    }
}

/// Versions this build applies: the baseline and every timestamped migration.
fn expected_versions() -> Vec<i64> {
    migrations::migrator()
        .iter()
        .filter(|migration| migration.migration_type.is_up_migration())
        .map(|migration| migration.version)
        .collect()
}

#[derive(Debug, PartialEq, Eq)]
pub(crate) enum MigrationState {
    Current(usize),
    Failed(Vec<i64>),
    Missing(Vec<i64>),
    Newer(Vec<i64>),
    Legacy,
}

/// Compare the database's history with this build's migrations.
pub(crate) fn judge_migrations(applied: &[(i64, bool)], expected: &[i64]) -> MigrationState {
    let failed: Vec<i64> = applied
        .iter()
        .filter(|(_, success)| !success)
        .map(|(version, _)| *version)
        .collect();
    if !failed.is_empty() {
        return MigrationState::Failed(failed);
    }
    if applied
        .iter()
        .any(|(version, _)| *version <= LEGACY_LAST_VERSION)
    {
        return MigrationState::Legacy;
    }
    let applied: HashSet<i64> = applied.iter().map(|(version, _)| *version).collect();
    let mut missing: Vec<i64> = expected
        .iter()
        .copied()
        .filter(|version| !applied.contains(version))
        .collect();
    if !missing.is_empty() {
        missing.sort_unstable();
        return MigrationState::Missing(missing);
    }
    let expected: HashSet<i64> = expected.iter().copied().collect();
    let mut newer: Vec<i64> = applied.difference(&expected).copied().collect();
    if !newer.is_empty() {
        newer.sort_unstable();
        return MigrationState::Newer(newer);
    }
    MigrationState::Current(expected.len())
}

fn pool_check(doctor: &SystemDoctor) -> DoctorCheck {
    let check = DoctorCheck::new(AREA, "database.connections", "Connection pools");
    let pools = [
        ("request", doctor.db.pool()),
        ("background", doctor.db.background_pool()),
    ];
    let mut full = Vec::new();
    let mut usage = Vec::new();
    for (name, pool) in pools {
        let max = pool.options().get_max_connections();
        let in_use = pool.size().saturating_sub(pool.num_idle() as u32);
        usage.push(format!("{name} {in_use} of {max}"));
        if max > 0 && in_use >= max {
            full.push(name);
        }
    }
    let usage = usage.join(", ");
    if full.is_empty() {
        check.ok(format!("Connections in use: {usage}."))
    } else {
        check
            .warning(format!("Every connection is busy ({usage})."))
            .why("New requests wait for a free connection, and time out if none frees up.")
            .fix("Raise DATABASE_POOL_MAX (or DATABASE_BACKGROUND_POOL_MAX), or find slow queries.")
    }
}

async fn long_transactions_check(pool: &PgPool) -> DoctorCheck {
    let check = DoctorCheck::new(AREA, "database.long_transactions", "Long transactions");
    let count: Result<(i64,), sqlx::Error> = sqlx::query_as(
        "SELECT count(*) FROM pg_stat_activity \
         WHERE datname = current_database() AND state <> 'idle' \
           AND xact_start < now() - make_interval(mins => $1::int)",
    )
    .bind(LONG_TRANSACTION_MINUTES as i32)
    .fetch_one(pool)
    .await;
    match count {
        Ok((0,)) => check.ok(format!(
            "No transaction has been open longer than {LONG_TRANSACTION_MINUTES} minutes."
        )),
        Ok((count,)) => check
            .warning(format!(
                "{} open longer than {LONG_TRANSACTION_MINUTES} minutes.",
                plural(
                    count as usize,
                    "transaction has been",
                    "transactions have been"
                )
            ))
            .why("Long transactions hold locks other work waits on, and stop vacuum.")
            .fix("Find them in pg_stat_activity and end the ones that are stuck."),
        Err(error) => check.info(format!(
            "Open transactions cannot be inspected: {}.",
            sql_error(&error)
        )),
    }
}

fn list(versions: &[i64]) -> String {
    const SHOWN: usize = 5;
    let mut text: Vec<String> = versions.iter().take(SHOWN).map(i64::to_string).collect();
    if versions.len() > SHOWN {
        text.push(format!("and {} more", versions.len() - SHOWN));
    }
    text.join(", ")
}

/// A database error without the statement or connection string.
fn sql_error(error: &sqlx::Error) -> String {
    match error {
        sqlx::Error::Database(error) => error.message().to_string(),
        sqlx::Error::PoolTimedOut => "no connection became free in time".to_string(),
        sqlx::Error::Io(error) => error.to_string(),
        other => other.to_string(),
    }
}
