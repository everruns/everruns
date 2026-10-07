// PostgreSQL repository: advisory locks held across pool-using critical sections
//
// `pg_advisory_xact_lock` waits inside a checked-out pool connection. That is
// fine when the holder does all of its work on the locking transaction, but
// some holders also draw more connections from the same pool (re-reads,
// persistence) and spend time on external calls under the lock. Once
// contenders outnumber the pool, every connection is parked in the lock wait,
// the holder cannot get one back, and it stalls until the acquire timeout,
// starving unrelated requests with it. Those holders use the polling lock
// below, which returns the connection to the pool between attempts.

use super::Database;
use anyhow::Result;
use sqlx::{Postgres, Transaction};
use std::time::Duration;

/// Default bound on how long a contender waits for a polling advisory lock.
/// Holders may make external API calls under the lock, so this has to cover
/// one slow round-trip with headroom; past it the contender fails instead of
/// queuing indefinitely.
pub const ADVISORY_LOCK_WAIT: Duration = Duration::from_secs(30);

const ADVISORY_LOCK_RETRY_INTERVAL: Duration = Duration::from_millis(50);

impl Database {
    /// Take a transaction-scoped advisory lock on `namespace:key` without
    /// holding a pool connection while waiting for it.
    ///
    /// The lock key is `hashtextextended('<namespace>:<key>', 0)`. The lock is
    /// released when the returned transaction ends, including on drop, so it
    /// cannot outlive its guard on any return or panic path. Fails once `wait`
    /// elapses without acquiring.
    pub async fn advisory_xact_lock_polling(
        &self,
        namespace: &str,
        key: &str,
        wait: Duration,
    ) -> Result<Transaction<'static, Postgres>> {
        let deadline = tokio::time::Instant::now() + wait;
        loop {
            // A lock guard, not data: it must end when its holder drops it,
            // not when an enclosing command transaction does.
            let mut tx = self.pool.begin_detached().await?;
            let acquired: bool = sqlx::query_scalar(
                "SELECT pg_try_advisory_xact_lock(hashtextextended($1 || ':' || $2, 0))",
            )
            .bind(namespace)
            .bind(key)
            .fetch_one(&mut *tx)
            .await?;
            if acquired {
                return Ok(tx);
            }
            // Hand the connection back before sleeping; that is the point of
            // polling rather than blocking in `pg_advisory_xact_lock`.
            tx.rollback().await?;

            let now = tokio::time::Instant::now();
            if now >= deadline {
                anyhow::bail!("timed out waiting for advisory lock {namespace}:{key}");
            }
            tokio::time::sleep_until(deadline.min(now + ADVISORY_LOCK_RETRY_INTERVAL)).await;
        }
    }
}
