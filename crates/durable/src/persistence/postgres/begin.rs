//! Cancellation-safe `BEGIN` and `COMMIT`.

use sqlx::postgres::{PgConnection, PgTransactionManager};
use sqlx::{PgPool, Postgres, Transaction};
use sqlx_core::transaction::TransactionManager;

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

/// Commit `tx` so that cancelling the caller cannot leave the outcome to the
/// transaction's drop guard.
///
/// Decision (EVERRUNS-35): sqlx 0.9 marks a transaction finished only once the
/// reply to `COMMIT` arrives. A caller dropped while `COMMIT` is in flight
/// drops a `Transaction` its guard still counts open, which queues a
/// `ROLLBACK` behind the `COMMIT` the server already ran: Postgres answers
/// "there is no transaction in progress", and the caller never learns that
/// its writes committed. Running the commit on its own task lets it finish
/// either way, mirroring [`begin_transaction`].
pub async fn commit_transaction(mut tx: Transaction<'static, Postgres>) -> Result<(), sqlx::Error> {
    // After `commit_in_place` sqlx counts no open transaction either way, so
    // dropping `tx` queues nothing.
    let commit = async move { commit_in_place(&mut tx).await };
    match tokio::spawn(commit).await {
        Ok(result) => result,
        Err(join) if join.is_panic() => std::panic::resume_unwind(join.into_panic()),
        // Only a runtime shutting down cancels the task.
        Err(_) => Err(sqlx::Error::PoolClosed),
    }
}

/// Commit the transaction open on `conn` and leave sqlx's count of open
/// transactions matching the server whichever way `COMMIT` goes.
///
/// Decision (EVERRUNS-35): a `COMMIT` the server rejects (a deferred
/// constraint check, a serialization failure) still ends the transaction, but
/// sqlx 0.9 leaves it counted open, so the drop guard's `ROLLBACK` draws
/// "there is no transaction in progress". sqlx cannot be told the transaction
/// is gone, so after a database error this opens an empty one for its count
/// and rolls that back. Any other error (the connection failed, so the outcome
/// is unknown) leaves the transaction counted open for the caller's guard.
pub async fn commit_in_place(conn: &mut PgConnection) -> Result<(), sqlx::Error> {
    match PgTransactionManager::commit(conn).await {
        Err(error @ sqlx::Error::Database(_)) => {
            let resync = async {
                sqlx::raw_sql("BEGIN").execute(&mut *conn).await?;
                PgTransactionManager::rollback(conn).await
            };
            if let Err(resync) = resync.await {
                tracing::warn!(error = %resync, "could not settle a rejected COMMIT");
            }
            Err(error)
        }
        result => result,
    }
}

#[cfg(all(test, feature = "postgres-tests"))]
mod tests {
    use std::sync::{Arc, Mutex};
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

    /// Postgres notices sqlx logs while the guard is held, e.g. the "there is
    /// no transaction in progress" a stray `ROLLBACK` draws.
    #[derive(Clone, Default)]
    struct Notices(Arc<Mutex<Vec<String>>>);

    impl<S: tracing::Subscriber> tracing_subscriber::Layer<S> for Notices {
        fn on_event(
            &self,
            event: &tracing::Event<'_>,
            _: tracing_subscriber::layer::Context<'_, S>,
        ) {
            struct Message<'a>(&'a mut String);
            impl tracing::field::Visit for Message<'_> {
                fn record_str(&mut self, field: &tracing::field::Field, value: &str) {
                    if field.name() == "message" {
                        self.0.push_str(value);
                    }
                }
                fn record_debug(
                    &mut self,
                    field: &tracing::field::Field,
                    value: &dyn std::fmt::Debug,
                ) {
                    if field.name() == "message" {
                        self.0.push_str(&format!("{value:?}"));
                    }
                }
            }
            if event.metadata().target() == "sqlx::postgres::notice" {
                let mut message = String::new();
                event.record(&mut Message(&mut message));
                self.0.lock().expect("notices").push(message);
            }
        }
    }

    /// EVERRUNS-35: a caller cancelled while `COMMIT` is in flight must not
    /// queue a `ROLLBACK` behind it. With sqlx's own `Transaction::commit`,
    /// the drop guard still counts the transaction open and rolls back a
    /// transaction the server already ended; the commit itself must finish.
    #[tokio::test]
    async fn cancelled_commit_finishes_without_a_stray_rollback() {
        use tracing_subscriber::layer::SubscriberExt;

        let notices = Notices::default();
        let _guard =
            tracing::subscriber::set_default(tracing_subscriber::registry().with(notices.clone()));
        let pool = sqlx::postgres::PgPoolOptions::new()
            .max_connections(1)
            .connect(&database_url())
            .await
            .expect("connect");
        sqlx::raw_sql(
            "DROP TABLE IF EXISTS cancelled_commit_probe; \
             CREATE TABLE cancelled_commit_probe (id INT PRIMARY KEY)",
        )
        .execute(&pool)
        .await
        .expect("create probe table");
        notices.0.lock().expect("notices").clear();

        for id in 0..200i32 {
            let mut tx = begin_transaction(&pool).await.expect("begin");
            sqlx::query("INSERT INTO cancelled_commit_probe (id) VALUES ($1)")
                .bind(id)
                .execute(&mut *tx)
                .await
                .expect("insert");
            // Cancel at varying points between sending COMMIT and its reply.
            let deadline = Duration::from_micros(u64::from(id.unsigned_abs() % 40) * 10);
            let _ = tokio::time::timeout(deadline, commit_transaction(tx)).await;
        }
        // Wait for every spawned commit, then drain the connection.
        let committed = loop {
            let count: i64 = sqlx::query_scalar("SELECT count(*) FROM cancelled_commit_probe")
                .fetch_one(&pool)
                .await
                .expect("count");
            if count == 200 {
                break count;
            }
            tokio::time::sleep(Duration::from_millis(10)).await;
            if !notices.0.lock().expect("notices").is_empty() {
                break count;
            }
        };
        assert!(!next_connection_in_transaction(&pool).await);
        sqlx::raw_sql("DROP TABLE cancelled_commit_probe")
            .execute(&pool)
            .await
            .expect("drop probe table");

        let notices = notices.0.lock().expect("notices").clone();
        assert!(
            notices.is_empty(),
            "cancelled COMMITs drew notices: {notices:?}"
        );
        assert_eq!(committed, 200, "a cancelled COMMIT was lost");
    }

    /// EVERRUNS-35: a `COMMIT` the server rejects (here a deferred foreign key
    /// check) ends the transaction server-side. The failure must reach the
    /// caller without a stray `ROLLBACK` behind it, and the connection must go
    /// back to the pool idle.
    #[tokio::test]
    async fn rejected_commit_draws_no_stray_rollback() {
        use tracing_subscriber::layer::SubscriberExt;

        let notices = Notices::default();
        let _guard =
            tracing::subscriber::set_default(tracing_subscriber::registry().with(notices.clone()));
        let pool = sqlx::postgres::PgPoolOptions::new()
            .max_connections(1)
            .connect(&database_url())
            .await
            .expect("connect");
        sqlx::raw_sql(
            "DROP TABLE IF EXISTS rejected_commit_child, rejected_commit_parent; \
             CREATE TABLE rejected_commit_parent (id INT PRIMARY KEY); \
             CREATE TABLE rejected_commit_child (id INT PRIMARY KEY REFERENCES \
             rejected_commit_parent(id) DEFERRABLE INITIALLY DEFERRED)",
        )
        .execute(&pool)
        .await
        .expect("create probe tables");
        notices.0.lock().expect("notices").clear();

        let mut tx = begin_transaction(&pool).await.expect("begin");
        sqlx::query("INSERT INTO rejected_commit_child (id) VALUES (1)")
            .execute(&mut *tx)
            .await
            .expect("insert defers its foreign key check");
        let error = commit_transaction(tx)
            .await
            .expect_err("the deferred foreign key check rejects COMMIT");
        assert!(
            matches!(&error, sqlx::Error::Database(db) if db.is_foreign_key_violation()),
            "unexpected commit error: {error:?}"
        );

        // Drain whatever the guard queued on the one pooled connection.
        assert!(!next_connection_in_transaction(&pool).await);
        let rows: i64 = sqlx::query_scalar("SELECT count(*) FROM rejected_commit_child")
            .fetch_one(&pool)
            .await
            .expect("count");
        sqlx::raw_sql("DROP TABLE rejected_commit_child, rejected_commit_parent")
            .execute(&pool)
            .await
            .expect("drop probe tables");

        assert_eq!(rows, 0, "a rejected COMMIT kept its writes");
        let notices = notices.0.lock().expect("notices").clone();
        assert!(
            notices.is_empty(),
            "a rejected COMMIT drew notices: {notices:?}"
        );
    }
}
