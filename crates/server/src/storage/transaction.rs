// Request-scoped transactions: one PostgreSQL transaction per mutating command.
//
// Decision: a mutation, the `entity_changes` row that records it and the
// idempotency record of the request that made it commit or roll back together
// (knowledge/execution/change-reasons-and-manager-context.md, "Write
// semantics"). Threading a `&mut Transaction` through every repository method
// and service would touch hundreds of signatures, so the transaction is carried
// in a task-local slot instead, the same pattern `change_history::intent` uses
// for the request's change intent.
//
// Decision: the repositories keep writing `.execute(&self.pool)`. `self.pool`
// is a `TxPool`, whose executor runs the query on the slot's connection when
// the current task is inside `scope` for the same pool, and on the pool
// otherwise. Every repository method is therefore converted at once; nothing
// has to opt in, and a method added later cannot forget to.
//
// Decision: the slot's connection sits behind a mutex held for one query at a
// time, never across unrelated awaits, so concurrent futures in one task
// (`join!`) serialize instead of failing. A repository method that opens its
// own transaction (`self.pool.begin()`) gets a SAVEPOINT on the slot's
// connection and holds the mutex until it commits or drops. While a savepoint
// is open, any other query from the same task falls back to the pool (today's
// behavior for that query) instead of deadlocking on the mutex, and is counted
// in `DB_QUERIES_OUTSIDE_TRANSACTION` so the remaining unconverted paths can be
// found.
//
// Decision: side effects other processes observe (event publish to SSE/NATS,
// listener notification, background work that reads the new rows) are queued
// with `after_commit` / `spawn_after_commit` while a transaction is open, run
// after it commits, and dropped when it rolls back. Outside a transaction they
// run immediately, so callers need not know which case they are in.
//
// Nested scopes reuse the outermost transaction: a command composed from other
// commands, or the snapshot read `PendingChange::record` dispatches, sees the
// outer command's uncommitted rows and commits with it.

use std::fmt;
use std::future::Future;
use std::sync::Arc;
use std::sync::atomic::{AtomicUsize, Ordering};

use futures::future::BoxFuture;
use futures::stream::{BoxStream, StreamExt};
use sqlx::pool::PoolConnection;
use sqlx::postgres::{PgConnection, PgQueryResult, PgRow, PgTransactionManager};
use sqlx::{Either, Execute, Executor, PgPool, Postgres};
use sqlx_core::transaction::TransactionManager;
use tokio::sync::{Mutex, OwnedMutexGuard};

use super::StorageBackend;

tokio::task_local! {
    static CURRENT: Arc<TxSlot>;
}

type SlotConnection = Option<PoolConnection<Postgres>>;

/// The open transaction of the current task.
struct TxSlot {
    /// Identity of the pool the transaction belongs to; see `pool_identity`.
    pool: usize,
    /// `None` once the transaction has finished.
    connection: Arc<Mutex<SlotConnection>>,
    /// Savepoints currently open on `connection` (each holds its mutex).
    savepoints: AtomicUsize,
    after_commit: std::sync::Mutex<Vec<BoxFuture<'static, ()>>>,
}

impl TxSlot {
    /// The slot's connection for one query, or `None` when a savepoint holds
    /// it and the query must fall back to the pool rather than wait on itself.
    async fn lock(&self) -> Option<OwnedMutexGuard<SlotConnection>> {
        if self.savepoints.load(Ordering::SeqCst) > 0 {
            count_outside("savepoint_open");
            return None;
        }
        Some(self.connection.clone().lock_owned().await)
    }
}

impl Drop for TxSlot {
    // A panic or a cancelled request leaves the connection here: roll it back
    // before the pool takes it, as `sqlx::Transaction` does on drop.
    fn drop(&mut self) {
        if let Ok(mut guard) = self.connection.try_lock()
            && let Some(connection) = guard.as_mut()
        {
            while PgTransactionManager::get_transaction_depth(connection) > 0 {
                PgTransactionManager::start_rollback(connection);
            }
        }
    }
}

fn count_outside(path: &'static str) {
    metrics::counter!(
        crate::api::prometheus::names::DB_QUERIES_OUTSIDE_TRANSACTION,
        "path" => path
    )
    .increment(1);
    tracing::debug!(
        path,
        "query ran outside the open command transaction; it commits on its own"
    );
}

fn closed() -> sqlx::Error {
    sqlx::Error::Protocol("the command transaction has already finished".into())
}

/// Identity of a pool and its clones. `connect_options` is one `Arc` shared by
/// every clone of a pool and distinct between pools.
fn pool_identity(pool: &PgPool) -> usize {
    Arc::as_ptr(&pool.connect_options()) as usize
}

/// Whether the current task runs inside a command transaction.
pub fn in_transaction() -> bool {
    CURRENT.try_with(|_| ()).is_ok()
}

/// Run `effect` after the current transaction commits; drop it if the
/// transaction rolls back. Outside a transaction it runs now.
pub async fn after_commit<F>(effect: F)
where
    F: Future<Output = ()> + Send + 'static,
{
    match CURRENT.try_with(Arc::clone) {
        Ok(slot) => slot
            .after_commit
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
            .push(Box::pin(effect)),
        Err(_) => effect.await,
    }
}

/// Spawn `task` once the current transaction commits (it may read the rows the
/// transaction wrote); never, if it rolls back. Outside a transaction it is
/// spawned now.
pub fn spawn_after_commit<F>(task: F)
where
    F: Future<Output = ()> + Send + 'static,
{
    match CURRENT.try_with(Arc::clone) {
        Ok(slot) => slot
            .after_commit
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
            .push(Box::pin(async move {
                tokio::spawn(task);
            })),
        Err(_) => {
            tokio::spawn(task);
        }
    }
}

/// Run `body` in one transaction on `db`: commit when it returns `Ok`, then run
/// its `after_commit` effects; roll back when it returns `Err` or the commit
/// fails. A scope already open on this task is reused, so the outermost scope
/// decides. `internal` turns a failure to begin or commit into the caller's
/// error type.
pub async fn scope<T, E, F>(
    db: &StorageBackend,
    body: F,
    internal: impl FnOnce(anyhow::Error) -> E,
) -> Result<T, E>
where
    F: Future<Output = Result<T, E>>,
{
    let database = db.database();
    if in_transaction() {
        return body.await;
    }
    let pool = database.tx_pool().pool.clone();
    let begin = async {
        let mut connection = pool.acquire().await?;
        PgTransactionManager::begin(&mut connection, None).await?;
        Ok::<_, sqlx::Error>(connection)
    };
    let connection = match begin.await {
        Ok(connection) => connection,
        Err(error) => return Err(internal(error.into())),
    };
    let slot = Arc::new(TxSlot {
        pool: pool_identity(&pool),
        connection: Arc::new(Mutex::new(Some(connection))),
        savepoints: AtomicUsize::new(0),
        after_commit: std::sync::Mutex::new(Vec::new()),
    });
    let result = CURRENT.scope(slot.clone(), body).await;
    let connection = slot.connection.lock().await.take();
    let effects = std::mem::take(
        &mut *slot
            .after_commit
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner()),
    );
    let Some(mut connection) = connection else {
        return Err(internal(closed().into()));
    };
    match result {
        Ok(value) => {
            if let Err(error) = PgTransactionManager::commit(&mut connection).await {
                PgTransactionManager::start_rollback(&mut connection);
                return Err(internal(error.into()));
            }
            for effect in effects {
                effect.await;
            }
            Ok(value)
        }
        Err(error) => {
            if let Err(rollback) = PgTransactionManager::rollback(&mut connection).await {
                tracing::warn!(error = %rollback, "command transaction rollback failed");
            }
            Err(error)
        }
    }
}

/// The request pool, aware of the current task's transaction. Repository
/// methods execute on `&TxPool` exactly as they would on `&PgPool`.
#[derive(Clone)]
pub struct TxPool {
    pool: PgPool,
}

impl fmt::Debug for TxPool {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("TxPool").finish_non_exhaustive()
    }
}

impl TxPool {
    pub fn new(pool: PgPool) -> Self {
        Self { pool }
    }

    /// The underlying pool, bypassing any open transaction. A query run on it
    /// while one is open commits on its own, so this is counted.
    pub fn raw(&self) -> &PgPool {
        if self.slot().is_some() {
            count_outside("raw_pool");
        }
        &self.pool
    }

    fn slot(&self) -> Option<Arc<TxSlot>> {
        CURRENT
            .try_with(Arc::clone)
            .ok()
            .filter(|slot| slot.pool == pool_identity(&self.pool))
    }

    /// Begin a transaction of its own on the pool, outside any command
    /// transaction: for lock guards whose lifetime is their holder's.
    pub async fn begin_detached(
        &self,
    ) -> Result<sqlx::Transaction<'static, Postgres>, sqlx::Error> {
        self.pool.begin().await
    }

    /// Begin a transaction: a savepoint inside the current task's transaction
    /// when there is one, else a transaction of its own on the pool.
    pub async fn begin(&self) -> Result<DbTx, sqlx::Error> {
        if let Some(slot) = self.slot()
            && let Some(mut guard) = slot.lock().await
        {
            let Some(connection) = guard.as_mut() else {
                return Err(closed());
            };
            PgTransactionManager::begin(connection, None).await?;
            slot.savepoints.fetch_add(1, Ordering::SeqCst);
            return Ok(DbTx(Inner::Savepoint {
                guard,
                slot,
                open: true,
            }));
        }
        Ok(DbTx(Inner::Own(Some(self.pool.begin().await?))))
    }
}

/// A transaction a repository method opened with `TxPool::begin`. Derefs to
/// the connection, so `&mut *tx` is an executor as with `sqlx::Transaction`.
pub struct DbTx(Inner);

enum Inner {
    Own(Option<sqlx::Transaction<'static, Postgres>>),
    Savepoint {
        guard: OwnedMutexGuard<SlotConnection>,
        slot: Arc<TxSlot>,
        open: bool,
    },
}

impl DbTx {
    /// Commit this transaction, or release its savepoint.
    pub async fn commit(mut self) -> Result<(), sqlx::Error> {
        match &mut self.0 {
            Inner::Own(tx) => tx.take().ok_or_else(closed)?.commit().await,
            Inner::Savepoint { guard, open, .. } => {
                let connection = guard.as_mut().ok_or_else(closed)?;
                PgTransactionManager::commit(connection).await?;
                *open = false;
                Ok(())
            }
        }
    }

    /// Roll this transaction back, or back to its savepoint.
    pub async fn rollback(mut self) -> Result<(), sqlx::Error> {
        match &mut self.0 {
            Inner::Own(tx) => tx.take().ok_or_else(closed)?.rollback().await,
            Inner::Savepoint { guard, open, .. } => {
                let connection = guard.as_mut().ok_or_else(closed)?;
                PgTransactionManager::rollback(connection).await?;
                *open = false;
                Ok(())
            }
        }
    }
}

impl Drop for DbTx {
    fn drop(&mut self) {
        if let Inner::Savepoint { guard, slot, open } = &mut self.0 {
            if *open && let Some(connection) = guard.as_mut() {
                PgTransactionManager::start_rollback(connection);
            }
            slot.savepoints.fetch_sub(1, Ordering::SeqCst);
        }
    }
}

impl std::ops::Deref for DbTx {
    type Target = PgConnection;

    fn deref(&self) -> &PgConnection {
        match &self.0 {
            Inner::Own(tx) => tx.as_ref().expect("transaction is open"),
            Inner::Savepoint { guard, .. } => guard.as_ref().expect("transaction is open"),
        }
    }
}

impl std::ops::DerefMut for DbTx {
    fn deref_mut(&mut self) -> &mut PgConnection {
        match &mut self.0 {
            Inner::Own(tx) => tx.as_mut().expect("transaction is open"),
            Inner::Savepoint { guard, .. } => guard.as_mut().expect("transaction is open"),
        }
    }
}

impl<'p> Executor<'p> for &'_ TxPool {
    type Database = Postgres;

    fn fetch_many<'e, 'q: 'e, E>(
        self,
        query: E,
    ) -> BoxStream<'e, Result<Either<PgQueryResult, PgRow>, sqlx::Error>>
    where
        E: 'q + Execute<'q, Postgres>,
    {
        let Some(slot) = self.slot() else {
            return self.pool.fetch_many(query);
        };
        let pool = self.pool.clone();
        // Rows are collected while the connection is held, so no stream
        // borrows the slot past this one query.
        futures::stream::once(async move {
            match slot.lock().await {
                Some(mut guard) => match guard.as_mut() {
                    Some(connection) => {
                        (&mut **connection)
                            .fetch_many(query)
                            .collect::<Vec<_>>()
                            .await
                    }
                    None => vec![Err(closed())],
                },
                None => pool.fetch_many(query).collect::<Vec<_>>().await,
            }
        })
        .flat_map(futures::stream::iter)
        .boxed()
    }

    fn fetch_optional<'e, 'q: 'e, E>(
        self,
        query: E,
    ) -> BoxFuture<'e, Result<Option<PgRow>, sqlx::Error>>
    where
        E: 'q + Execute<'q, Postgres>,
    {
        let Some(slot) = self.slot() else {
            return self.pool.fetch_optional(query);
        };
        let pool = self.pool.clone();
        Box::pin(async move {
            match slot.lock().await {
                Some(mut guard) => match guard.as_mut() {
                    Some(connection) => (&mut **connection).fetch_optional(query).await,
                    None => Err(closed()),
                },
                None => pool.fetch_optional(query).await,
            }
        })
    }

    fn prepare_with<'e>(
        self,
        sql: sqlx::SqlStr,
        parameters: &'e [sqlx::postgres::PgTypeInfo],
    ) -> BoxFuture<'e, Result<sqlx::postgres::PgStatement, sqlx::Error>>
    where
        'p: 'e,
    {
        self.pool.prepare_with(sql, parameters)
    }

    #[doc(hidden)]
    fn describe<'e>(
        self,
        sql: sqlx::SqlStr,
    ) -> BoxFuture<'e, Result<sqlx::Describe<Postgres>, sqlx::Error>>
    where
        'p: 'e,
    {
        self.pool.describe(sql)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::atomic::AtomicBool;

    #[tokio::test]
    async fn outside_a_transaction_effects_run_at_once() {
        let ran = Arc::new(AtomicBool::new(false));
        let flag = ran.clone();
        after_commit(async move { flag.store(true, Ordering::SeqCst) }).await;
        assert!(ran.load(Ordering::SeqCst));
        assert!(!in_transaction());
    }
}
