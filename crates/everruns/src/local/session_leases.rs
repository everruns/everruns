//! Session leases in the local SQLite database.
//!
//! Decision (actor-based design, step 4): processes that share a data
//! directory (an app and `serve dev` on one profile, two copies of a daemon)
//! share this table, so only one of them runs a session's turn at a time.
//! One store instance is one holder, named by a fresh id, so engines in one
//! process that share these backends never block each other. Expiry is wall
//! time in milliseconds, as other processes read it; an `IMMEDIATE`
//! transaction makes the check and the take one step.

use std::time::Duration;

use async_trait::async_trait;
use everruns_contracts::error::{AgentLoopError, Result};
use everruns_contracts::typed_id::SessionId;
use everruns_core::host::{SessionLease, SessionLeases};
use rusqlite::{OptionalExtension, TransactionBehavior, params};
use uuid::Uuid;

use super::db::SqliteDb;
use crate::sqlite as rusqlite;

/// [`SessionLeases`] shared by every process that opens the same local
/// database.
#[derive(Clone)]
pub struct LocalSessionLeases {
    db: SqliteDb,
    holder: String,
}

impl std::fmt::Debug for LocalSessionLeases {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("LocalSessionLeases")
            .field("holder", &self.holder)
            .finish_non_exhaustive()
    }
}

impl LocalSessionLeases {
    /// A new holder over `db`, creating the lease table if needed.
    pub fn new(db: SqliteDb) -> Result<Self> {
        db.with_conn(|conn| {
            conn.execute_batch(
                "CREATE TABLE IF NOT EXISTS local_session_leases (
                    session_id    TEXT PRIMARY KEY,
                    holder        TEXT,
                    fence         INTEGER NOT NULL,
                    expires_at_ms INTEGER NOT NULL
                 );",
            )
        })
        .map_err(AgentLoopError::from)?;
        Ok(Self {
            db,
            holder: Uuid::new_v4().to_string(),
        })
    }

    /// Another holder over the same database, as a second process would be.
    pub fn another_holder(&self) -> Self {
        Self {
            db: self.db.clone(),
            holder: Uuid::new_v4().to_string(),
        }
    }
}

fn now_ms() -> i64 {
    chrono::Utc::now().timestamp_millis()
}

fn ttl_ms(ttl: Duration) -> i64 {
    i64::try_from(ttl.as_millis()).unwrap_or(i64::MAX)
}

fn as_fence(value: i64) -> u64 {
    u64::try_from(value).unwrap_or_default()
}

#[async_trait]
impl SessionLeases for LocalSessionLeases {
    async fn acquire(&self, session_id: SessionId, ttl: Duration) -> Result<Option<SessionLease>> {
        let key = session_id.to_string();
        let now = now_ms();
        let expires = now.saturating_add(ttl_ms(ttl));
        self.db
            .with_conn_mut(|conn| {
                let tx = conn.transaction_with_behavior(TransactionBehavior::Immediate)?;
                let held: Option<(Option<String>, i64, i64)> = tx
                    .query_row(
                        "SELECT holder, fence, expires_at_ms FROM local_session_leases
                         WHERE session_id = ?1",
                        params![key],
                        |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?)),
                    )
                    .optional()?;
                let fence = match held {
                    Some((Some(holder), fence, _)) if holder == self.holder => fence,
                    Some((Some(_), _, expires_at)) if expires_at > now => return Ok(None),
                    Some((_, fence, _)) => fence + 1,
                    None => 1,
                };
                tx.execute(
                    "INSERT INTO local_session_leases (session_id, holder, fence, expires_at_ms)
                     VALUES (?1, ?2, ?3, ?4)
                     ON CONFLICT(session_id) DO UPDATE SET
                        holder = excluded.holder,
                        fence = excluded.fence,
                        expires_at_ms = excluded.expires_at_ms",
                    params![key, self.holder, fence, expires],
                )?;
                tx.commit()?;
                Ok(Some(SessionLease::new(session_id, as_fence(fence))))
            })
            .map_err(AgentLoopError::from)
    }

    async fn renew(&self, lease: &SessionLease, ttl: Duration) -> Result<bool> {
        let expires = now_ms().saturating_add(ttl_ms(ttl));
        let fence = i64::try_from(lease.fence).unwrap_or(i64::MAX);
        let updated = self
            .db
            .with_conn(|conn| {
                conn.execute(
                    "UPDATE local_session_leases SET expires_at_ms = ?1
                     WHERE session_id = ?2 AND holder = ?3 AND fence = ?4",
                    params![expires, lease.session_id.to_string(), self.holder, fence],
                )
            })
            .map_err(AgentLoopError::from)?;
        Ok(updated == 1)
    }

    async fn release(&self, lease: &SessionLease) -> Result<()> {
        let fence = i64::try_from(lease.fence).unwrap_or(i64::MAX);
        // Keep the row and its fence, so the next holder's fence is higher.
        self.db
            .with_conn(|conn| {
                conn.execute(
                    "UPDATE local_session_leases SET holder = NULL, expires_at_ms = 0
                     WHERE session_id = ?1 AND holder = ?2 AND fence = ?3",
                    params![lease.session_id.to_string(), self.holder, fence],
                )
            })
            .map_err(AgentLoopError::from)?;
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn leases() -> LocalSessionLeases {
        LocalSessionLeases::new(SqliteDb::open_in_memory().unwrap()).unwrap()
    }

    #[tokio::test]
    async fn a_held_lease_blocks_another_process_until_released() {
        let ours = leases();
        let theirs = ours.another_holder();
        let session_id = SessionId::new();
        let ttl = Duration::from_secs(30);

        let lease = ours.acquire(session_id, ttl).await.unwrap().unwrap();
        assert_eq!(lease.fence, 1);
        assert_eq!(
            ours.acquire(session_id, ttl).await.unwrap(),
            Some(lease.clone())
        );
        assert_eq!(theirs.acquire(session_id, ttl).await.unwrap(), None);

        ours.release(&lease).await.unwrap();
        let taken = theirs.acquire(session_id, ttl).await.unwrap().unwrap();
        assert_eq!(taken.fence, 2);
        assert!(!ours.renew(&lease, ttl).await.unwrap());
        assert!(theirs.renew(&taken, ttl).await.unwrap());
    }

    #[tokio::test]
    async fn an_expired_lease_passes_to_another_process() {
        let ours = leases();
        let theirs = ours.another_holder();
        let session_id = SessionId::new();

        let lease = ours
            .acquire(session_id, Duration::ZERO)
            .await
            .unwrap()
            .unwrap();
        tokio::time::sleep(Duration::from_millis(5)).await;
        let taken = theirs
            .acquire(session_id, Duration::from_secs(30))
            .await
            .unwrap()
            .unwrap();
        assert!(taken.fence > lease.fence);
        assert!(!ours.renew(&lease, Duration::from_secs(30)).await.unwrap());
        // Releasing a lost lease leaves the new holder's alone.
        ours.release(&lease).await.unwrap();
        assert_eq!(
            ours.acquire(session_id, Duration::from_secs(30))
                .await
                .unwrap(),
            None
        );
    }
}
