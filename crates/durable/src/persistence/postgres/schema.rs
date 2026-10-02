//! Standalone PostgreSQL schema for [`PostgresWorkflowEventStore`].
//!
//! Decision: the crate ships its own idempotent schema
//! (`crates/durable/schema/postgres.sql`) so a crates.io user can create the
//! durable tables without the Everruns server's migrations. The server keeps
//! its own versioned migrations; `tests/schema_drift_test.rs` proves the two
//! produce the same durable tables, and running this schema against a
//! server-migrated database is a no-op.

use sqlx::PgPool;

use super::PostgresWorkflowEventStore;

/// Arbitrary, stable key for the transaction-scoped advisory lock that
/// serializes concurrent [`PostgresWorkflowEventStore::migrate`] calls.
const SCHEMA_LOCK_KEY: i64 = 0x6576_6572_6475_7261; // "everdura"

impl PostgresWorkflowEventStore {
    /// The idempotent SQL that [`migrate`](Self::migrate) applies.
    ///
    /// Exposed for applications that manage schema with their own migration
    /// tool: copy it into a migration instead of calling `migrate` at start-up.
    pub const SCHEMA_SQL: &'static str = include_str!("../../../schema/postgres.sql");

    /// Create or update the durable tables, indexes, functions and triggers.
    ///
    /// Safe to call on every start-up: every statement is idempotent, the whole
    /// schema is applied in one transaction, and concurrent callers serialize
    /// on an advisory lock. A database already migrated by the Everruns server
    /// is left unchanged. Objects land in the first schema of the connection's
    /// `search_path`, normally `public`.
    ///
    /// Requires PostgreSQL 14 or newer and no extensions.
    ///
    /// # Example
    ///
    /// ```no_run
    /// use everruns_durable::PostgresWorkflowEventStore;
    ///
    /// # async fn run() -> Result<(), sqlx::Error> {
    /// let pool = sqlx::PgPool::connect("postgres://localhost/my_app").await?;
    /// PostgresWorkflowEventStore::migrate(&pool).await?;
    /// let store = PostgresWorkflowEventStore::new(pool);
    /// # let _ = store;
    /// # Ok(()) }
    /// ```
    pub async fn migrate(pool: &PgPool) -> Result<(), sqlx::Error> {
        let mut tx = pool.begin().await?;
        // `CREATE ... IF NOT EXISTS` reports every existing object as a
        // NOTICE; keep re-runs quiet.
        sqlx::query("SET LOCAL client_min_messages = warning")
            .execute(&mut *tx)
            .await?;
        sqlx::query("SELECT pg_advisory_xact_lock($1)")
            .bind(SCHEMA_LOCK_KEY)
            .execute(&mut *tx)
            .await?;
        sqlx::raw_sql(Self::SCHEMA_SQL).execute(&mut *tx).await?;
        tx.commit().await
    }
}
