use super::*;
use async_trait::async_trait;
use std::{path::Path, sync::Mutex};
/// Versioned state returned by an atomic store.
#[derive(Debug, Clone, Default)]
pub struct VersionedState {
    /// Compare-and-swap revision; an absent key starts at zero.
    pub revision: i64,
    /// Retained state.
    pub state: State,
}
/// Atomic per-trigger persistence. Receipt tombstones must outlive outbox/history pruning.
#[async_trait]
pub trait StateStore: Send + Sync {
    /// Whether this delivery was previously accepted or filtered.
    async fn seen(&self, key: &str, delivery: &str) -> Result<bool, Error>;
    /// Read the latest committed state.
    async fn load(&self, key: &str) -> Result<VersionedState, Error>;
    /// Atomically replace only when revision still matches and receipt is unseen.
    /// A supplied receipt must be inserted in the same transaction. Return false on contention.
    async fn compare_exchange(
        &self,
        key: &str,
        revision: i64,
        state: &State,
        receipt: Option<&str>,
    ) -> Result<bool, Error>;
}
/// SQLite-backed atomic outbox and permanent receipt tombstones.
/// Multiple processes coordinate using SQLite transactions.
#[derive(Clone)]
pub struct SqliteStore(Arc<Mutex<rusqlite::Connection>>);
impl SqliteStore {
    /// Open private application storage outside the agent workspace.
    /// Receipt keys persist until this database is explicitly retired.
    pub fn open(path: impl AsRef<Path>) -> Result<Self, Error> {
        let path = path.as_ref();
        if std::fs::symlink_metadata(path).is_ok_and(|metadata| metadata.file_type().is_symlink()) {
            return Err(Error("Trigger storage must not be a symlink".into()));
        }
        let mut options = std::fs::OpenOptions::new();
        options.read(true).write(true).create(true).truncate(false);
        #[cfg(unix)]
        {
            use std::os::unix::fs::OpenOptionsExt;
            options.mode(0o600);
        }
        options
            .open(path)
            .map_err(|_| Error("Could not create private trigger storage".into()))?;
        let db = rusqlite::Connection::open(path)
            .map_err(|_| Error("Could not open trigger database".into()))?;
        db.busy_timeout(std::time::Duration::from_secs(5))
            .map_err(|_| Error("Could not configure trigger database".into()))?;
        db.execute_batch(
            "PRAGMA journal_mode=WAL;
             CREATE TABLE IF NOT EXISTS framework_triggers(
                 key TEXT PRIMARY KEY, revision INTEGER NOT NULL, state TEXT NOT NULL
             );
             CREATE TABLE IF NOT EXISTS framework_trigger_receipts(
                 trigger_key TEXT NOT NULL, delivery TEXT NOT NULL,
                 PRIMARY KEY(trigger_key, delivery)
             );",
        )
        .map_err(|_| Error("Could not initialize trigger database".into()))?;
        Ok(Self(Arc::new(Mutex::new(db))))
    }
}
#[async_trait]
impl StateStore for SqliteStore {
    async fn seen(&self, key: &str, delivery: &str) -> Result<bool, Error> {
        let db = self.0.clone();
        let key = key.to_string();
        let delivery = delivery.to_string();
        tokio::task::spawn_blocking(move || {
            let db = db
                .lock()
                .map_err(|_| Error("Trigger database unavailable".into()))?;
            db.query_row(
                "SELECT EXISTS(SELECT 1 FROM framework_trigger_receipts
                 WHERE trigger_key=?1 AND delivery=?2)",
                rusqlite::params![key, delivery],
                |r| r.get(0),
            )
            .map_err(|_| Error("Could not read trigger receipt".into()))
        })
        .await
        .map_err(|_| Error("Trigger storage task failed".into()))?
    }
    async fn load(&self, key: &str) -> Result<VersionedState, Error> {
        let db = self.0.clone();
        let key = key.to_string();
        tokio::task::spawn_blocking(move || {
            use rusqlite::OptionalExtension;
            let db = db
                .lock()
                .map_err(|_| Error("Trigger database unavailable".into()))?;
            let row: Option<(i64, String)> = db
                .query_row(
                    "SELECT revision,state FROM framework_triggers WHERE key=?1",
                    [key],
                    |r| Ok((r.get(0)?, r.get(1)?)),
                )
                .optional()
                .map_err(|_| Error("Could not read trigger state".into()))?;
            match row {
                None => Ok(VersionedState::default()),
                Some((revision, state)) => Ok(VersionedState {
                    revision,
                    state: serde_json::from_str(&state)
                        .map_err(|_| Error("Corrupt trigger state".into()))?,
                }),
            }
        })
        .await
        .map_err(|_| Error("Trigger storage task failed".into()))?
    }
    async fn compare_exchange(
        &self,
        key: &str,
        revision: i64,
        state: &State,
        receipt: Option<&str>,
    ) -> Result<bool, Error> {
        let db = self.0.clone();
        let key = key.to_string();
        let receipt = receipt.map(str::to_string);
        let data = serde_json::to_string(state)
            .map_err(|_| Error("Could not encode trigger state".into()))?;
        tokio::task::spawn_blocking(move || {
            let mut db = db.lock().map_err(|_| Error("Trigger database unavailable".into()))?;
            let tx = db.transaction_with_behavior(rusqlite::TransactionBehavior::Immediate)
                .map_err(|_| Error("Could not lock trigger state".into()))?;
            if let Some(receipt) = &receipt {
                let seen: bool = tx.query_row(
                    "SELECT EXISTS(SELECT 1 FROM framework_trigger_receipts
                     WHERE trigger_key=?1 AND delivery=?2)",
                    rusqlite::params![key, receipt], |r| r.get(0),
                ).map_err(|_| Error("Could not read trigger receipt".into()))?;
                if seen { return Ok(false); }
            }
            let changed = if revision == 0 {
                tx.execute(
                    "INSERT INTO framework_triggers(key,revision,state) VALUES(?1,1,?2)
                     ON CONFLICT(key) DO NOTHING",
                    rusqlite::params![key, data],
                )
            } else {
                tx.execute(
                    "UPDATE framework_triggers SET revision=revision+1,state=?1
                     WHERE key=?2 AND revision=?3",
                    rusqlite::params![data, key, revision],
                )
            }.map_err(|_| Error("Could not commit trigger state".into()))?;
            if changed == 1 {
                if let Some(receipt) = receipt {
                    tx.execute(
                        "INSERT INTO framework_trigger_receipts(trigger_key,delivery) VALUES(?1,?2)",
                        rusqlite::params![key, receipt],
                    ).map_err(|_| Error("Could not retain trigger receipt".into()))?;
                }
                tx.commit().map_err(|_| Error("Could not commit trigger state".into()))?;
            }
            Ok(changed == 1)
        }).await.map_err(|_| Error("Trigger storage task failed".into()))?
    }
}
