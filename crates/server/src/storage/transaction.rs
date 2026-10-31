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
    savepoints: Arc<AtomicUsize>,
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
        crate::metrics_names::DB_QUERIES_OUTSIDE_TRANSACTION,
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

/// A connection with an open transaction, rolled back if dropped before it is
/// taken.
struct BegunConnection(Option<PoolConnection<Postgres>>);

impl Drop for BegunConnection {
    fn drop(&mut self) {
        if let Some(connection) = self.0.as_mut() {
            while PgTransactionManager::get_transaction_depth(connection) > 0 {
                PgTransactionManager::start_rollback(connection);
            }
        }
    }
}

/// Acquire a connection and open a transaction on it, on a task of its own so
/// a cancelled caller cannot leave the transaction open on a pooled
/// connection; see `everruns_durable::begin_transaction` (EVERRUNS-4H). A
/// caller gone by the time `BEGIN` returns drops the `BegunConnection` on that
/// task, which queues the rollback.
async fn begin_connection(pool: &PgPool) -> Result<PoolConnection<Postgres>, sqlx::Error> {
    let pool = pool.clone();
    let begun = tokio::spawn(async move {
        let mut connection = pool.acquire().await?;
        PgTransactionManager::begin(&mut connection, None).await?;
        Ok::<_, sqlx::Error>(BegunConnection(Some(connection)))
    })
    .await;
    match begun {
        Ok(Ok(mut begun)) => Ok(begun.0.take().expect("set on the task")),
        Ok(Err(error)) => Err(error),
        Err(join) if join.is_panic() => std::panic::resume_unwind(join.into_panic()),
        // Only a runtime shutting down cancels the task.
        Err(_) => Err(sqlx::Error::PoolClosed),
    }
}

/// Commit the transaction open on `connection` on a task of its own, so a
/// caller cancelled while `COMMIT` is in flight cannot queue a rollback behind
/// it, nor a rejected `COMMIT` a stray one; see
/// `everruns_durable::commit_transaction` (EVERRUNS-35).
async fn commit_connection(mut connection: PoolConnection<Postgres>) -> Result<(), sqlx::Error> {
    let commit = tokio::spawn(async move {
        let result = everruns_durable::commit_in_place(&mut connection).await;
        // Only a failed connection is still counted open: queue its rollback.
        PgTransactionManager::start_rollback(&mut connection);
        result
    })
    .await;
    match commit {
        Ok(result) => result,
        Err(join) if join.is_panic() => std::panic::resume_unwind(join.into_panic()),
        // Only a runtime shutting down cancels the task.
        Err(_) => Err(sqlx::Error::PoolClosed),
    }
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

/// [`after_commit`] for an effect that must not be lost once its rows are
/// committed: the effect runs on its own task, and the caller still waits for
/// it (so consecutive effects keep their order), but dropping the caller no
/// longer drops the effect.
///
/// Decision: a request future is dropped whenever its client goes away (a
/// worker SIGKILLed mid-RPC resets its gRPC stream; hyper then drops the
/// handler). Run inline, an event publish queued behind its committed insert
/// died with it: the row was stored but never reached live subscribers, and an
/// SSE client resuming from a later event's sequence skipped it for good
/// (chaos harness: a turn's `turn.started` / `turn.completed` never arrived).
pub async fn after_commit_detached<F>(effect: F)
where
    F: Future<Output = ()> + Send + 'static,
{
    after_commit(run_detached(effect)).await;
}

async fn run_detached<F>(effect: F)
where
    F: Future<Output = ()> + Send + 'static,
{
    use tracing::Instrument;
    if let Err(error) = tokio::spawn(effect.in_current_span()).await {
        tracing::error!(error = %error, "post-commit effect panicked");
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
    let connection = match begin_connection(&pool).await {
        Ok(connection) => connection,
        Err(error) => return Err(internal(error.into())),
    };
    let slot = Arc::new(TxSlot {
        pool: pool_identity(&pool),
        connection: Arc::new(Mutex::new(Some(connection))),
        savepoints: Arc::new(AtomicUsize::new(0)),
        after_commit: std::sync::Mutex::new(Vec::new()),
    });
    let result = everruns_durable::schedule_transaction::scope(
        &pool,
        slot.connection.clone(),
        slot.savepoints.clone(),
        CURRENT.scope(slot.clone(), body),
    )
    .await;
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
            // Effects may wait (a cancel waits for the turn's holder): the
            // connection goes back to the pool when the commit task ends.
            if let Err(error) = commit_connection(connection).await {
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

    /// Try a lock on the current request transaction, never an implicit one.
    /// Unlike ordinary queries this must fail closed when there is no matching
    /// scope or a savepoint holds its connection: a pool fallback would release
    /// the lock before the operation it is supposed to protect.
    pub(crate) async fn try_advisory_xact_lock(&self, key: &str) -> Result<bool, sqlx::Error> {
        let slot = self.slot().ok_or_else(|| {
            sqlx::Error::Protocol("advisory lock requires a matching request transaction".into())
        })?;
        let mut guard = slot.lock().await.ok_or_else(|| {
            sqlx::Error::Protocol("advisory lock cannot bypass an open savepoint".into())
        })?;
        let connection = guard.as_mut().ok_or_else(closed)?;
        sqlx::query_scalar("SELECT pg_try_advisory_xact_lock(hashtextextended($1::text, 0))")
            .bind(key)
            .fetch_one(&mut **connection)
            .await
    }

    /// Begin a transaction of its own on the pool, outside any command
    /// transaction: for lock guards whose lifetime is their holder's.
    pub async fn begin_detached(
        &self,
    ) -> Result<sqlx::Transaction<'static, Postgres>, sqlx::Error> {
        everruns_durable::begin_transaction(&self.pool).await
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
        Ok(DbTx(Inner::Own(Some(
            everruns_durable::begin_transaction(&self.pool).await?,
        ))))
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
            Inner::Own(tx) => {
                everruns_durable::commit_transaction(tx.take().ok_or_else(closed)?).await
            }
            // A savepoint stays on this task: its connection belongs to the
            // enclosing `scope`, whose cancellation rolls the whole
            // transaction back.
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

    /// A detached effect that has started finishes even when its caller is
    /// dropped mid-way, as a gRPC handler is when its worker dies.
    #[tokio::test]
    async fn detached_effects_survive_a_dropped_caller() {
        let (entered_tx, entered_rx) = tokio::sync::oneshot::channel::<()>();
        let (gate_tx, gate_rx) = tokio::sync::oneshot::channel::<()>();
        let (done_tx, done_rx) = tokio::sync::oneshot::channel::<()>();
        let caller = tokio::spawn(after_commit_detached(async move {
            let _ = entered_tx.send(());
            let _ = gate_rx.await;
            let _ = done_tx.send(());
        }));
        entered_rx.await.unwrap();
        caller.abort();
        assert!(caller.await.unwrap_err().is_cancelled());
        gate_tx.send(()).unwrap();
        tokio::time::timeout(std::time::Duration::from_secs(5), done_rx)
            .await
            .expect("the effect must finish after its caller was dropped")
            .expect("the effect was dropped with its caller");
    }
}
