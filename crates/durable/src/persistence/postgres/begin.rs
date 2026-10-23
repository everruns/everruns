//! Cancellation-safe `BEGIN`.

use sqlx::{PgPool, Postgres, Transaction};

/// Begin a transaction on `pool` so that cancelling the caller cannot leave
/// the transaction open on a pooled connection.
///
/// Decision (EVERRUNS-4H, EVERRUNS-35): sqlx 0.9 counts a transaction only
/// once the reply to `BEGIN` arrives, and its drop guard rolls back only
/// counted transactions. A caller dropped while `BEGIN` is in flight (a gRPC
/// handler past its deadline or whose client went away, an HTTP request whose
/// client disconnected) returns the connection to the pool inside a
/// server-side transaction sqlx does not know about. Later autocommit
/// statements on that connection then stay uncommitted, holding their row
/// locks, until some unrelated `COMMIT`, or are lost with a `ROLLBACK`; the
/// next `BEGIN` on it logs "there is already a transaction in progress".
///
/// Running `pool.begin()` on its own task lets `BEGIN` finish whatever happens
/// to the caller. If the caller is gone, the finished `Transaction` is dropped
/// on that task, which queues its rollback before the pool reuses the
/// connection. The cost is one task spawn per transaction.
pub async fn begin_transaction(
    pool: &PgPool,
) -> Result<Transaction<'static, Postgres>, sqlx::Error> {
    let pool = pool.clone();
    #[allow(clippy::disallowed_methods)] // the one sanctioned call
    let begin = async move { pool.begin().await };
    match tokio::spawn(begin).await {
        Ok(result) => result,
        Err(join) if join.is_panic() => std::panic::resume_unwind(join.into_panic()),
        // Only a runtime shutting down cancels the task.
        Err(_) => Err(sqlx::Error::PoolClosed),
    }
}
