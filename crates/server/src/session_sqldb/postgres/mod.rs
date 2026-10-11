// PostgreSQL-persisted session SQL databases ("page diff").
//
// Decisions:
// - PostgreSQL is the source of truth; each process keeps an in-memory SQLite
//   connection per database as a working cache. A cache miss rebuilds it from
//   the stored pages (`pages::open_from_pages`).
// - A write runs on the cache, then the changed pages are diffed against the
//   last persisted image and committed in one PostgreSQL transaction before
//   the call returns (`Database::commit_session_database_pages`). A call that
//   changes nothing (a SELECT through `sql_execute`, a failed statement that
//   left the database as it was) writes nothing.
// - Each `sql_execute` call is all-or-nothing: if it fails after changing the
//   cache (a later statement in the batch errors, a size limit is hit, the
//   commit fails) the cache is dropped, and the next call reloads what was
//   last committed. A call may not leave a transaction open, since an open
//   transaction's pages are not committed state.
// - Cross-process safety is a compare-and-set on `session_databases.generation`.
//   Every access re-reads the generation (one indexed lookup) and reloads a
//   stale cache before using it; a writer that loses the race between its
//   check and its commit discards its result, reloads, and re-runs the
//   statement, up to `MAX_WRITE_ATTEMPTS` times.
// - Within a process, one async mutex per database serializes access, so a
//   cache is never used by two calls at once.
// - The cache is bounded (`MAX_CACHED_DATABASES`); evicting is always safe
//   because the pages are persisted, it only costs a reload.

use super::error::SqlDbError;
use super::executor::{execute_query, execute_statement, get_schema};
use super::limits::{MAX_DATABASES_PER_SESSION, MAX_DB_SIZE_BYTES, MAX_SESSION_TOTAL_SIZE_BYTES};
use super::pages::{PageDiff, PageDigest, diff_connection, open_from_pages};
use super::validation::validate_database_name;
use crate::storage::{CreateSessionDatabase, Database, SessionDatabaseRow};
use async_trait::async_trait;
use everruns_contracts::session_sqldb::{
    DatabaseInfo, SessionSqlDbError, SessionSqlDbStore, SqlExecuteResult, SqlQueryResult,
    TableSchema,
};
use everruns_contracts::typed_id::SessionId;
use rusqlite::Connection;
use std::collections::HashMap;
use std::sync::Arc;
use std::time::Instant;
use tokio::sync::Mutex;
use tracing::{debug, warn};
use uuid::Uuid;

/// Databases kept open in memory per process before idle ones are evicted.
const MAX_CACHED_DATABASES: usize = 256;

/// Attempts at a write that keeps losing the generation race.
const MAX_WRITE_ATTEMPTS: usize = 3;

type DbKey = (SessionId, String);

/// A database open in memory, and the persisted state it was built from.
struct CachedDb {
    db_id: Uuid,
    /// Generation of the stored pages this connection matches.
    generation: i64,
    conn: Connection,
    /// Digests of the stored pages, the baseline the next write is diffed against.
    digests: Vec<PageDigest>,
}

type Slot = Arc<Mutex<Option<CachedDb>>>;

struct CacheEntry {
    slot: Slot,
    last_used: Instant,
}

/// Session SQL database store persisted to `session_databases` /
/// `session_database_pages`.
pub struct PostgresSqlDbStore {
    db: Database,
    cache: parking_lot::Mutex<HashMap<DbKey, CacheEntry>>,
}

impl PostgresSqlDbStore {
    pub fn new(db: Database) -> Self {
        Self {
            db,
            cache: parking_lot::Mutex::new(HashMap::new()),
        }
    }

    /// The per-database slot, created on first use.
    fn slot(&self, key: &DbKey) -> Slot {
        let mut cache = self.cache.lock();
        let now = Instant::now();
        if let Some(entry) = cache.get_mut(key) {
            entry.last_used = now;
            return entry.slot.clone();
        }
        if cache.len() >= MAX_CACHED_DATABASES {
            // Only an entry nobody holds can go: cloning a slot needs this
            // lock, so a count of one cannot change under us.
            let idle = cache
                .iter()
                .filter(|(_, entry)| Arc::strong_count(&entry.slot) == 1)
                .min_by_key(|(_, entry)| entry.last_used)
                .map(|(key, _)| key.clone());
            if let Some(idle) = idle {
                cache.remove(&idle);
            }
        }
        let slot = Slot::default();
        cache.insert(
            key.clone(),
            CacheEntry {
                slot: slot.clone(),
                last_used: now,
            },
        );
        slot
    }

    /// The cache for `meta`, rebuilt from the stored pages when missing or
    /// stale. Takes the cache out of `held`; the caller puts it back.
    async fn fresh(
        &self,
        held: &mut Option<CachedDb>,
        meta: &SessionDatabaseRow,
    ) -> Result<CachedDb, SessionSqlDbError> {
        if let Some(cached) = held.take()
            && cached.db_id == meta.id
            && cached.generation == meta.generation
        {
            return Ok(cached);
        }
        let (generation, pages) = self
            .db
            .load_session_database_pages(meta.id)
            .await
            .map_err(internal)?
            .ok_or_else(|| SessionSqlDbError::DatabaseNotFound(meta.name.clone()))?;
        debug!(database_id = %meta.id, generation, pages = pages.len(), "Loading session database");
        let (conn, digests) = blocking(move || open_from_pages(&pages)).await??;
        Ok(CachedDb {
            db_id: meta.id,
            generation,
            conn,
            digests,
        })
    }

    async fn create_row(
        &self,
        session_id: SessionId,
        name: &str,
    ) -> Result<CreateSessionDatabase, SessionSqlDbError> {
        self.db
            .create_session_database(session_id, name, MAX_DATABASES_PER_SESSION as i64)
            .await
            .map_err(internal)
    }

    /// Run a read against the current state of a database.
    async fn read<T, F>(
        &self,
        session_id: SessionId,
        db_name: &str,
        read: F,
    ) -> Result<T, SessionSqlDbError>
    where
        T: Send + 'static,
        F: FnOnce(&Connection) -> Result<T, SqlDbError> + Send + 'static,
    {
        validate_database_name(db_name)?;
        let slot = self.slot(&(session_id, db_name.to_string()));
        let mut held = slot.lock().await;
        let Some(meta) = self.metadata(session_id, db_name).await? else {
            *held = None;
            return Err(SessionSqlDbError::DatabaseNotFound(db_name.to_string()));
        };
        let cached = self.fresh(&mut held, &meta).await?;
        // Reads keep the cache: the executor refuses statements that write.
        let (cached, result) = blocking(move || {
            let result = read(&cached.conn);
            (cached, result)
        })
        .await?;
        *held = Some(cached);
        Ok(result?)
    }

    async fn metadata(
        &self,
        session_id: SessionId,
        name: &str,
    ) -> Result<Option<SessionDatabaseRow>, SessionSqlDbError> {
        self.db
            .get_session_database(session_id, name)
            .await
            .map_err(internal)
    }

    /// Check the size limits a write's result must stay within.
    async fn check_size(
        &self,
        session_id: SessionId,
        db_id: Uuid,
        diff: &PageDiff,
    ) -> Result<(), SessionSqlDbError> {
        let new_size = diff.size_bytes();
        if new_size > MAX_DB_SIZE_BYTES {
            return Err(SessionSqlDbError::LimitExceeded(format!(
                "database size {new_size} exceeds limit {MAX_DB_SIZE_BYTES}"
            )));
        }
        let others = self
            .db
            .session_databases_size_excluding(session_id, db_id)
            .await
            .map_err(internal)?;
        if others + new_size > MAX_SESSION_TOTAL_SIZE_BYTES {
            return Err(SessionSqlDbError::LimitExceeded(format!(
                "session total size would exceed limit {MAX_SESSION_TOTAL_SIZE_BYTES}"
            )));
        }
        Ok(())
    }
}

/// Run `sql` on the cache and diff the result. The flag on an error says
/// whether the cache is still identical to what is stored, and can be kept.
fn run_write(
    cached: &CachedDb,
    sql: &str,
) -> Result<(SqlExecuteResult, PageDiff), (SqlDbError, bool)> {
    let unchanged = |cached: &CachedDb| {
        cached.conn.is_autocommit()
            && diff_connection(&cached.conn, &cached.digests)
                .is_ok_and(|diff| diff.is_unchanged(cached.digests.len()))
    };
    let result = match execute_statement(&cached.conn, sql) {
        Ok(result) => result,
        Err(err) => return Err((err, unchanged(cached))),
    };
    if !cached.conn.is_autocommit() {
        return Err((
            SqlDbError::QueryError(
                "sql_execute must finish every transaction it opens (add COMMIT)".to_string(),
            ),
            false,
        ));
    }
    let diff = diff_connection(&cached.conn, &cached.digests).map_err(|err| (err, false))?;
    Ok((result, diff))
}

fn info(row: SessionDatabaseRow) -> DatabaseInfo {
    DatabaseInfo {
        name: row.name,
        size_bytes: row.size_bytes,
        page_count: row.page_count,
        created_at: row.created_at,
        updated_at: row.updated_at,
    }
}

fn internal(err: anyhow::Error) -> SessionSqlDbError {
    warn!(error = %err, "session SQL database storage failed");
    SessionSqlDbError::Internal(format!("session database storage failed: {err}"))
}

async fn blocking<T, F>(f: F) -> Result<T, SessionSqlDbError>
where
    T: Send + 'static,
    F: FnOnce() -> T + Send + 'static,
{
    tokio::task::spawn_blocking(f)
        .await
        .map_err(|e| SessionSqlDbError::Internal(format!("spawn_blocking failed: {e}")))
}

#[async_trait]
impl SessionSqlDbStore for PostgresSqlDbStore {
    async fn create_database(
        &self,
        session_id: SessionId,
        name: &str,
    ) -> Result<DatabaseInfo, SessionSqlDbError> {
        validate_database_name(name)?;
        match self.create_row(session_id, name).await? {
            CreateSessionDatabase::Created(row) => Ok(info(row)),
            CreateSessionDatabase::AlreadyExists => {
                Err(SessionSqlDbError::DatabaseAlreadyExists(name.to_string()))
            }
            CreateSessionDatabase::LimitReached => Err(SessionSqlDbError::LimitExceeded(format!(
                "maximum {MAX_DATABASES_PER_SESSION} databases per session"
            ))),
        }
    }

    async fn list_databases(
        &self,
        session_id: SessionId,
    ) -> Result<Vec<DatabaseInfo>, SessionSqlDbError> {
        let rows = self
            .db
            .list_session_databases(session_id)
            .await
            .map_err(internal)?;
        Ok(rows.into_iter().map(info).collect())
    }

    async fn get_database(
        &self,
        session_id: SessionId,
        name: &str,
    ) -> Result<Option<DatabaseInfo>, SessionSqlDbError> {
        Ok(self.metadata(session_id, name).await?.map(info))
    }

    async fn delete_database(
        &self,
        session_id: SessionId,
        name: &str,
    ) -> Result<bool, SessionSqlDbError> {
        let slot = self.slot(&(session_id, name.to_string()));
        let mut held = slot.lock().await;
        *held = None;
        self.db
            .delete_session_database(session_id, name)
            .await
            .map_err(internal)
    }

    async fn sql_execute(
        &self,
        session_id: SessionId,
        db_name: &str,
        sql: &str,
    ) -> Result<SqlExecuteResult, SessionSqlDbError> {
        validate_database_name(db_name)?;
        let slot = self.slot(&(session_id, db_name.to_string()));
        let mut held = slot.lock().await;
        for attempt in 1..=MAX_WRITE_ATTEMPTS {
            let meta = match self.metadata(session_id, db_name).await? {
                Some(meta) => meta,
                // Auto-create on first write; a concurrent create is fine.
                None => match self.create_row(session_id, db_name).await? {
                    CreateSessionDatabase::Created(row) => row,
                    CreateSessionDatabase::AlreadyExists => self
                        .metadata(session_id, db_name)
                        .await?
                        .ok_or_else(|| SessionSqlDbError::DatabaseNotFound(db_name.to_string()))?,
                    CreateSessionDatabase::LimitReached => {
                        return Err(SessionSqlDbError::LimitExceeded(format!(
                            "maximum {MAX_DATABASES_PER_SESSION} databases per session"
                        )));
                    }
                },
            };
            let cached = self.fresh(&mut held, &meta).await?;

            let statement = sql.to_string();
            let (cached, outcome) = blocking(move || {
                let outcome = run_write(&cached, &statement);
                (cached, outcome)
            })
            .await?;
            let (result, diff) = match outcome {
                Ok(done) => done,
                Err((err, keep)) => {
                    if keep {
                        *held = Some(cached);
                    }
                    return Err(err.into());
                }
            };
            if diff.is_unchanged(cached.digests.len()) {
                *held = Some(cached);
                return Ok(result);
            }
            // From here on, a failure leaves the cache dropped (`held` is
            // empty), so the next call reloads the committed state.
            self.check_size(session_id, cached.db_id, &diff).await?;
            let committed = self
                .db
                .commit_session_database_pages(
                    cached.db_id,
                    cached.generation,
                    &diff.changed,
                    diff.page_count,
                    diff.size_bytes(),
                )
                .await
                .map_err(internal)?;
            match committed {
                Some(generation) => {
                    debug!(
                        database_id = %cached.db_id,
                        generation,
                        changed_pages = diff.changed.len(),
                        page_count = diff.page_count,
                        "Persisted session database write"
                    );
                    *held = Some(CachedDb {
                        generation,
                        digests: diff.digests,
                        ..cached
                    });
                    return Ok(result);
                }
                None => {
                    debug!(
                        database_id = %cached.db_id,
                        attempt,
                        "Session database changed under this write; reloading"
                    );
                }
            }
        }
        Err(SessionSqlDbError::Internal(format!(
            "database {db_name} kept changing under concurrent writes; retry"
        )))
    }

    async fn sql_query(
        &self,
        session_id: SessionId,
        db_name: &str,
        sql: &str,
    ) -> Result<SqlQueryResult, SessionSqlDbError> {
        let sql = sql.to_string();
        self.read(session_id, db_name, move |conn| execute_query(conn, &sql))
            .await
    }

    async fn sql_schema(
        &self,
        session_id: SessionId,
        db_name: &str,
        table: Option<&str>,
    ) -> Result<Vec<TableSchema>, SessionSqlDbError> {
        let table = table.map(str::to_string);
        self.read(session_id, db_name, move |conn| {
            let schemas = get_schema(conn)?;
            Ok(match table {
                Some(table) => schemas.into_iter().filter(|t| t.name == table).collect(),
                None => schemas,
            })
        })
        .await
    }
}

#[cfg(test)]
mod tests;
