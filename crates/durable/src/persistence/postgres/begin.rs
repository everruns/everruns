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

#[cfg(all(test, feature = "postgres-tests"))]
mod tests {
    use std::time::Duration;

    use super::*;

    fn database_url() -> String {
        std::env::var("DATABASE_URL").unwrap_or_else(|_| {
            let port = std::env::var("DB_PORT").unwrap_or_else(|_| "9332".to_string());
            format!("postgres://postgres:postgres@localhost:{port}/everruns_test")
        })
    }

    /// Whether the connection the pool hands out next is inside an open
    /// transaction: `SAVEPOINT` succeeds only there.
    async fn next_connection_in_transaction(pool: &PgPool) -> bool {
        let mut connection = pool.acquire().await.expect("acquire");
        match sqlx::raw_sql("SAVEPOINT probe")
            .execute(&mut *connection)
            .await
        {
            Ok(_) => {
                sqlx::raw_sql("ROLLBACK")
                    .execute(&mut *connection)
                    .await
                    .expect("roll back the leaked transaction");
                true
            }
            Err(_) => false,
        }
    }

    /// EVERRUNS-4H: a caller cancelled while `BEGIN` is in flight must not
    /// return the connection to the pool inside an open transaction. With
    /// sqlx's own `pool.begin()`, 10 to 28 of the 200 leaked one locally.
    #[tokio::test]
    async fn cancelled_begin_leaves_no_open_transaction() {
        let pool = sqlx::postgres::PgPoolOptions::new()
            .max_connections(1)
            .connect(&database_url())
            .await
            .expect("connect");
        assert!(!next_connection_in_transaction(&pool).await);

        let mut leaked = 0;
        for i in 0..200u64 {
            // Cancel at varying points between sending BEGIN and its reply.
            let deadline = Duration::from_micros((i % 40) * 10);
            let _ = tokio::time::timeout(deadline, begin_transaction(&pool)).await;
            if next_connection_in_transaction(&pool).await {
                leaked += 1;
            }
        }
        assert_eq!(
            leaked, 0,
            "{leaked} of 200 cancelled BEGINs left a transaction open"
        );
    }
}
