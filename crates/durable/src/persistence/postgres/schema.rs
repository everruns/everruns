//! Standalone PostgreSQL schema for [`PostgresWorkflowEventStore`].
//!
//! The crate ships an [idempotent PostgreSQL schema](https://github.com/everruns/everruns/blob/main/crates/durable/schema/postgres.sql)
//! so applications can create durable tables without the Everruns server's
//! migrations. [`PostgresWorkflowEventStore::SCHEMA_SQL`] exposes the same SQL
//! for custom migration tools. Running it against a server-migrated database
//! is a no-op.

use sqlx::PgPool;

use super::PostgresWorkflowEventStore;

/// Arbitrary, stable key for the transaction-scoped advisory lock that
/// serializes concurrent [`PostgresWorkflowEventStore::migrate`] calls.
const SCHEMA_LOCK_KEY: i64 = 0x6576_6572_6475_7261; // "everdura"

/// The table whose comment records which [`SCHEMA_SQL`] a database last took.
///
/// Decision: re-applying the schema is not free. `CREATE INDEX IF NOT EXISTS`
/// and `CREATE OR REPLACE TRIGGER` lock their tables before they find nothing
/// to do, so a process migrating while another runs tasks could deadlock
/// with it. A start-up whose schema is already applied therefore only reads
/// this comment (no lock beyond a catalog read) and skips the DDL. A comment
/// keeps the schema itself identical to the server's migrations.
///
/// [`SCHEMA_SQL`]: PostgresWorkflowEventStore::SCHEMA_SQL
const SCHEMA_MARKER_TABLE: &str = "durable_workflow_instances";

/// `durable-schema:<FNV-1a of SCHEMA_SQL>`: stable across builds and Rust
/// versions, unlike `std`'s hasher.
fn schema_marker() -> String {
    let mut hash: u64 = 0xcbf2_9ce4_8422_2325;
    for byte in PostgresWorkflowEventStore::SCHEMA_SQL.bytes() {
        hash ^= u64::from(byte);
        hash = hash.wrapping_mul(0x0000_0100_0000_01b3);
    }
    format!("durable-schema:{hash:016x}")
}

/// The schema marker the database holds, if any.
async fn applied_marker<'e, E>(executor: E) -> Result<Option<String>, sqlx::Error>
where
    E: sqlx::PgExecutor<'e>,
{
    sqlx::query_scalar("SELECT obj_description(to_regclass($1), 'pg_class')")
        .bind(SCHEMA_MARKER_TABLE)
        .fetch_one(executor)
        .await
}

impl PostgresWorkflowEventStore {
    /// The idempotent SQL that [`migrate`](Self::migrate) applies.
    ///
    /// Exposed for applications that manage schema with their own migration
    /// tool: copy it into a migration instead of calling `migrate` at start-up.
    pub const SCHEMA_SQL: &'static str = include_str!("../../../schema/postgres.sql");

    /// Create or update the durable tables, indexes, functions and triggers.
    ///
    /// Safe to call on every start-up: every statement is idempotent, the whole
    /// schema is applied in one transaction, concurrent callers serialize on
    /// an advisory lock, and a database that already took this exact schema
    /// is only read, not locked. Apply a new schema version before traffic
    /// that uses it, as with any migration. A database already migrated by the Everruns server
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
        let marker = schema_marker();
        if applied_marker(pool).await?.as_deref() == Some(marker.as_str()) {
            return Ok(());
        }
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
        // Another caller may have applied it while this one waited.
        if applied_marker(&mut *tx).await?.as_deref() == Some(marker.as_str()) {
            return tx.commit().await;
        }
        sqlx::raw_sql(Self::SCHEMA_SQL).execute(&mut *tx).await?;
        // COMMENT takes no bind parameters; the statement is built only from
        // a constant table name and a hex hash, so nothing untrusted reaches it.
        let comment = format!("COMMENT ON TABLE {SCHEMA_MARKER_TABLE} IS '{marker}'");
        sqlx::raw_sql(sqlx::AssertSqlSafe(comment))
            .execute(&mut *tx)
            .await?;
        tx.commit().await
    }

    /// Connect to the database at `url`, apply the schema with
    /// [`migrate`](Self::migrate), and return a store over the new pool.
    ///
    /// The one-call start for an application that keeps no pool of its own,
    /// so it never touches the database driver. An application that manages
    /// its pool or its schema itself builds the store with
    /// [`new`](Self::new) instead.
    ///
    /// # Example
    ///
    /// ```no_run
    /// use everruns_durable::PostgresWorkflowEventStore;
    ///
    /// # async fn run() -> Result<(), sqlx::Error> {
    /// let store = PostgresWorkflowEventStore::connect("postgres://localhost/my_app").await?;
    /// # let _ = store;
    /// # Ok(()) }
    /// ```
    pub async fn connect(url: &str) -> Result<Self, sqlx::Error> {
        let pool = PgPool::connect(url).await?;
        Self::migrate(&pool).await?;
        Ok(Self::new(pool))
    }
}
