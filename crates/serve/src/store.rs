//! The host's own SQLite: the session catalog, and what the channel host
//! persists (bindings, platform delivery keys already accepted, turns that
//! still owe a reply).
//!
//! Decision: serve keeps no event log. Conversation state and the durable
//! canonical event log are the everruns engine's (`LocalConfig` in dev and
//! start, in memory in evals); the wire API reads them through
//! `Session::events_after` / `events_from`. This store keeps only what the
//! engine does not know: which agent and build a session runs, its title,
//! tags and metadata, the channel state above, and the last occurrence of
//! each app schedule it handled (so a restart can catch up one it missed).
//!
//! No compatibility with earlier serve databases: an older schema is dropped.

use everruns::sqlite as rusqlite;
use std::path::Path;
use std::sync::Mutex;
use std::time::Duration;

use async_trait::async_trait;
use chrono::{DateTime, Utc};
use everruns::channels::{ChannelError, ChannelStore, DeliveryCheckpoint, PendingDelivery};
use rusqlite::{Connection, OptionalExtension, params};
use serde_json::Value;

/// Schema version in `PRAGMA user_version`.
const SCHEMA_VERSION: i64 = 2;

/// A session row.
#[derive(Clone, Debug)]
pub(crate) struct SessionRow {
    pub id: String,
    pub agent: String,
    /// The build the session started on. Hosts route it back there.
    pub build_id: String,
    pub title: Option<String>,
    pub tags: Vec<String>,
    pub hints: Option<Value>,
    pub metadata: Option<Value>,
    pub created_at: String,
    pub updated_at: String,
}

pub(crate) struct Store {
    conn: Mutex<Connection>,
}

impl Store {
    pub(crate) fn open(path: &Path) -> crate::Result<Self> {
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent)?;
        }
        Self::init(rusqlite::open(path)?)
    }

    pub(crate) fn in_memory() -> crate::Result<Self> {
        Self::init(rusqlite::open_in_memory()?)
    }

    fn init(mut conn: Connection) -> crate::Result<Self> {
        conn.execute_batch("PRAGMA journal_mode = WAL;")?;
        let version: i64 = conn.query_row("PRAGMA user_version", [], |row| row.get(0))?;
        if version < SCHEMA_VERSION {
            // Earlier serve builds mirrored events here; that log is gone.
            conn.execute_batch(
                "DROP TABLE IF EXISTS events;
                 DROP TABLE IF EXISTS sessions;",
            )?;
        }
        conn.execute_batch(&format!(
            "CREATE TABLE IF NOT EXISTS sessions (
                 id TEXT PRIMARY KEY,
                 agent TEXT NOT NULL,
                 build_id TEXT NOT NULL,
                 title TEXT,
                 tags TEXT NOT NULL,
                 hints TEXT,
                 metadata TEXT,
                 created_at TEXT NOT NULL,
                 updated_at TEXT NOT NULL
             );
             CREATE TABLE IF NOT EXISTS channel_threads (
                 channel TEXT NOT NULL,
                 thread TEXT NOT NULL,
                 session_id TEXT NOT NULL,
                 PRIMARY KEY (channel, thread)
             );
             CREATE TABLE IF NOT EXISTS channel_seen (
                 channel TEXT NOT NULL,
                 key TEXT NOT NULL,
                 seen_at INTEGER NOT NULL,
                 PRIMARY KEY (channel, key)
             );
             CREATE TABLE IF NOT EXISTS channel_pending (
                 session_id TEXT NOT NULL,
                 input_message_id TEXT NOT NULL,
                 delivery TEXT NOT NULL,
                 PRIMARY KEY (session_id, input_message_id)
             );
             CREATE TABLE IF NOT EXISTS schedule_runs (
                 name TEXT PRIMARY KEY,
                 due_at TEXT NOT NULL
             );
             PRAGMA user_version = {SCHEMA_VERSION};"
        ))?;
        // Additive upgrade: never discard existing channel bindings or pending replies.
        let transaction =
            conn.transaction_with_behavior(rusqlite::TransactionBehavior::Immediate)?;
        for (name, definition) in [
            ("holder", "TEXT NOT NULL DEFAULT ''"),
            ("lease_until", "INTEGER NOT NULL DEFAULT 0"),
            ("checkpoint", "TEXT"),
        ] {
            let present: bool = transaction.query_row(
                "SELECT COUNT(*) > 0 FROM pragma_table_info('channel_pending') WHERE name = ?1",
                params![name],
                |row| row.get(0),
            )?;
            if !present {
                transaction.execute_batch(&format!(
                    "ALTER TABLE channel_pending ADD COLUMN {name} {definition}"
                ))?;
            }
        }
        transaction.commit()?;
        Ok(Self {
            conn: Mutex::new(conn),
        })
    }

    fn conn(&self) -> std::sync::MutexGuard<'_, Connection> {
        // A panic while holding the lock leaves SQLite itself consistent.
        self.conn
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
    }

    pub(crate) fn insert_session(&self, row: &SessionRow) -> crate::Result {
        self.conn().execute(
            "INSERT INTO sessions
               (id, agent, build_id, title, tags, hints, metadata, created_at, updated_at)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9) ON CONFLICT(id) DO UPDATE SET
                 agent=excluded.agent, build_id=excluded.build_id",
            params![
                row.id,
                row.agent,
                row.build_id,
                row.title,
                serde_json::to_string(&row.tags)?,
                row.hints.as_ref().map(Value::to_string),
                row.metadata.as_ref().map(Value::to_string),
                row.created_at,
                row.updated_at
            ],
        )?;
        Ok(())
    }

    pub(crate) fn session(&self, id: &str) -> crate::Result<Option<SessionRow>> {
        let json = |text: Option<String>| text.and_then(|text| serde_json::from_str(&text).ok());
        Ok(self
            .conn()
            .query_row(
                "SELECT id, agent, build_id, title, tags, hints, metadata, created_at, updated_at
                 FROM sessions WHERE id = ?1",
                params![id],
                |row| {
                    Ok(SessionRow {
                        id: row.get(0)?,
                        agent: row.get(1)?,
                        build_id: row.get(2)?,
                        title: row.get(3)?,
                        tags: serde_json::from_str(&row.get::<_, String>(4)?).unwrap_or_default(),
                        hints: json(row.get(5)?),
                        metadata: json(row.get(6)?),
                        created_at: row.get(7)?,
                        updated_at: row.get(8)?,
                    })
                },
            )
            .optional()?)
    }

    /// Ids of one agent's sessions, most recently active first.
    pub(crate) fn session_ids_for_agent(
        &self,
        agent: &str,
        limit: usize,
    ) -> crate::Result<Vec<String>> {
        let conn = self.conn();
        let mut statement = conn.prepare(
            "SELECT id FROM sessions WHERE agent = ?1 ORDER BY updated_at DESC, id DESC LIMIT ?2",
        )?;
        let ids = statement
            .query_map(params![agent, limit as i64], |row| row.get(0))?
            .collect::<Result<Vec<String>, _>>()?;
        Ok(ids)
    }

    /// Ids of sessions active at or after `since` (an RFC 3339 timestamp in
    /// the catalog's own format), most recently active first.
    pub(crate) fn session_ids_active_since(&self, since: &str) -> crate::Result<Vec<String>> {
        let conn = self.conn();
        let mut statement = conn.prepare(
            "SELECT id FROM sessions WHERE updated_at >= ?1 ORDER BY updated_at DESC, id DESC",
        )?;
        let ids = statement
            .query_map(params![since], |row| row.get(0))?
            .collect::<Result<Vec<String>, _>>()?;
        Ok(ids)
    }

    /// Record activity on a session (a message was accepted).
    pub(crate) fn touch(&self, id: &str, at: &str) -> crate::Result {
        self.conn().execute(
            "UPDATE sessions SET updated_at = ?1 WHERE id = ?2",
            params![at, id],
        )?;
        Ok(())
    }

    pub(crate) fn thread_session(
        &self,
        channel: &str,
        thread: &str,
    ) -> crate::Result<Option<String>> {
        Ok(self
            .conn()
            .query_row(
                "SELECT session_id FROM channel_threads WHERE channel = ?1 AND thread = ?2",
                params![channel, thread],
                |row| row.get(0),
            )
            .optional()?)
    }

    /// Bind `thread` unless it is bound; returns the session it is bound to.
    pub(crate) fn bind_thread(
        &self,
        channel: &str,
        thread: &str,
        session_id: &str,
    ) -> crate::Result<String> {
        let conn = self.conn();
        conn.execute(
            "INSERT OR IGNORE INTO channel_threads (channel, thread, session_id) VALUES (?1, ?2, ?3)",
            params![channel, thread, session_id],
        )?;
        Ok(conn.query_row(
            "SELECT session_id FROM channel_threads WHERE channel = ?1 AND thread = ?2",
            params![channel, thread],
            |row| row.get(0),
        )?)
    }

    /// The last occurrence of the app schedule `name` this host handled.
    pub(crate) fn schedule_due_at(&self, name: &str) -> crate::Result<Option<DateTime<Utc>>> {
        let at: Option<String> = self
            .conn()
            .query_row(
                "SELECT due_at FROM schedule_runs WHERE name = ?1",
                params![name],
                |row| row.get(0),
            )
            .optional()?;
        Ok(at
            .and_then(|at| DateTime::parse_from_rfc3339(&at).ok())
            .map(|at| at.with_timezone(&Utc)))
    }

    /// Record `due_at` as the last occurrence of `name` this host handled.
    pub(crate) fn record_schedule_due(&self, name: &str, due_at: DateTime<Utc>) -> crate::Result {
        self.conn().execute(
            "INSERT OR REPLACE INTO schedule_runs (name, due_at) VALUES (?1, ?2)",
            params![name, due_at.to_rfc3339()],
        )?;
        Ok(())
    }
}

/// Platform retries arrive within minutes; a day of keys is plenty.
const SEEN_KEEP_SECS: i64 = 24 * 60 * 60;

fn store_error(error: impl std::fmt::Display) -> ChannelError {
    ChannelError::Other(anyhow::anyhow!("serve store: {error}"))
}

#[async_trait]
impl ChannelStore for Store {
    fn supports_delivery_checkpoints(&self) -> bool {
        true
    }

    async fn delivery_checkpoint(
        &self,
        session_id: &str,
        input_message_id: &str,
    ) -> Result<Option<DeliveryCheckpoint>, ChannelError> {
        let state: Option<String> = self.conn().query_row("SELECT checkpoint FROM channel_pending WHERE session_id = ?1 AND input_message_id = ?2", params![session_id, input_message_id], |row| row.get(0)).optional().map_err(store_error)?.flatten();
        state
            .map(|state| serde_json::from_str(&state).map_err(store_error))
            .transpose()
    }

    async fn save_delivery_checkpoint(
        &self,
        session_id: &str,
        input_message_id: &str,
        holder: &str,
        checkpoint: &DeliveryCheckpoint,
    ) -> Result<(), ChannelError> {
        let state = serde_json::to_string(checkpoint).map_err(store_error)?;
        let updated = self.conn().execute("UPDATE channel_pending SET checkpoint = ?4 WHERE session_id = ?1 AND input_message_id = ?2 AND holder = ?3 AND lease_until > ?5", params![session_id, input_message_id, holder, state, chrono::Utc::now().timestamp_millis()]).map_err(store_error)?;
        if updated != 1 {
            return Err(ChannelError::Session(
                "delivery lease is no longer held".into(),
            ));
        }
        Ok(())
    }

    async fn complete_delivery(
        &self,
        session_id: &str,
        input_message_id: &str,
        holder: &str,
    ) -> Result<(), ChannelError> {
        let updated = self.conn().execute("DELETE FROM channel_pending WHERE session_id = ?1 AND input_message_id = ?2 AND holder = ?3 AND lease_until > ?4", params![session_id, input_message_id, holder, chrono::Utc::now().timestamp_millis()]).map_err(store_error)?;
        if updated != 1 {
            return Err(ChannelError::Session(
                "delivery lease is no longer held".into(),
            ));
        }
        Ok(())
    }

    async fn renew_pending(
        &self,
        session_id: &str,
        input_message_id: &str,
        holder: &str,
        lease: Duration,
    ) -> Result<(), ChannelError> {
        let now = chrono::Utc::now().timestamp_millis();
        let until = now.saturating_add(i64::try_from(lease.as_millis()).unwrap_or(i64::MAX));
        let updated = self.conn().execute("UPDATE channel_pending SET lease_until = ?4 WHERE session_id = ?1 AND input_message_id = ?2 AND holder = ?3 AND lease_until > ?5", params![session_id, input_message_id, holder, until, now]).map_err(store_error)?;
        if updated != 1 {
            return Err(ChannelError::Session(
                "delivery lease is no longer held".into(),
            ));
        }
        Ok(())
    }

    async fn release_pending(
        &self,
        session_id: &str,
        input_message_id: &str,
        holder: &str,
    ) -> Result<(), ChannelError> {
        self.conn().execute("UPDATE channel_pending SET lease_until = 0 WHERE session_id = ?1 AND input_message_id = ?2 AND holder = ?3", params![session_id, input_message_id, holder]).map_err(store_error)?;
        Ok(())
    }

    async fn session_for(&self, channel: &str, key: &str) -> Result<Option<String>, ChannelError> {
        self.thread_session(channel, key).map_err(store_error)
    }

    async fn bind(
        &self,
        channel: &str,
        key: &str,
        session_id: &str,
    ) -> Result<String, ChannelError> {
        self.bind_thread(channel, key, session_id)
            .map_err(store_error)
    }

    async fn first_sighting(&self, channel: &str, dedup_key: &str) -> Result<bool, ChannelError> {
        let now = chrono::Utc::now().timestamp();
        let conn = self.conn();
        conn.execute(
            "DELETE FROM channel_seen WHERE seen_at < ?1",
            params![now - SEEN_KEEP_SECS],
        )
        .map_err(store_error)?;
        let inserted = conn
            .execute(
                "INSERT OR IGNORE INTO channel_seen (channel, key, seen_at) VALUES (?1, ?2, ?3)",
                params![channel, dedup_key, now],
            )
            .map_err(store_error)?;
        Ok(inserted == 1)
    }

    // Fenced leases also cover overlapping processes during a restart.
    async fn save_pending(
        &self,
        pending: &PendingDelivery,
        holder: &str,
        lease: Duration,
    ) -> Result<(), ChannelError> {
        let delivery = serde_json::to_string(pending).map_err(store_error)?;
        let until = chrono::Utc::now()
            .timestamp_millis()
            .saturating_add(i64::try_from(lease.as_millis()).unwrap_or(i64::MAX));
        self.conn()
            .execute(
                "INSERT INTO channel_pending (session_id, input_message_id, delivery, holder, lease_until)
                 VALUES (?1, ?2, ?3, ?4, ?5)
                 ON CONFLICT (session_id, input_message_id) DO NOTHING",
                params![pending.session_id, pending.input_message_id, delivery, holder, until],
            )
            .map_err(store_error)?;
        Ok(())
    }

    async fn clear_pending(
        &self,
        session_id: &str,
        input_message_id: &str,
    ) -> Result<(), ChannelError> {
        self.conn()
            .execute(
                "DELETE FROM channel_pending WHERE session_id = ?1 AND input_message_id = ?2",
                params![session_id, input_message_id],
            )
            .map_err(store_error)?;
        Ok(())
    }

    async fn claim_pending(
        &self,
        holder: &str,
        lease: Duration,
    ) -> Result<Vec<PendingDelivery>, ChannelError> {
        let conn = self.conn();
        let now = chrono::Utc::now().timestamp_millis();
        let until = now.saturating_add(i64::try_from(lease.as_millis()).unwrap_or(i64::MAX));
        let mut statement = conn
            .prepare("UPDATE channel_pending SET holder = ?1, lease_until = ?2 WHERE lease_until <= ?3 RETURNING delivery")
            .map_err(store_error)?;
        let rows = statement
            .query_map(params![holder, until, now], |row| row.get::<_, String>(0))
            .map_err(store_error)?
            .collect::<Result<Vec<_>, _>>()
            .map_err(store_error)?;
        // A row this build cannot read is skipped rather than failing every
        // other recovery.
        Ok(rows
            .iter()
            .filter_map(|row| serde_json::from_str(row).ok())
            .collect())
    }
}

#[cfg(test)]
impl Store {
    /// Pretend a session started on another build.
    pub(crate) fn repin_for_test(&self, id: &str, build_id: &str) {
        self.conn()
            .execute(
                "UPDATE sessions SET build_id = ?1 WHERE id = ?2",
                params![build_id, id],
            )
            .unwrap();
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    const LEASE: Duration = everruns::channels::DEFAULT_DELIVERY_LEASE;

    fn row() -> SessionRow {
        SessionRow {
            id: "s1".into(),
            agent: "analyst".into(),
            build_id: "b1".into(),
            title: Some("Weekly".into()),
            tags: vec!["finance".into()],
            hints: None,
            metadata: Some(json!({ "k": "v" })),
            created_at: "t0".into(),
            updated_at: "t0".into(),
        }
    }

    #[test]
    fn sessions_and_threads_round_trip() {
        let store = Store::in_memory().unwrap();
        store.insert_session(&row()).unwrap();
        store.touch("s1", "t1").unwrap();
        let loaded = store.session("s1").unwrap().unwrap();
        assert_eq!(loaded.metadata, Some(json!({ "k": "v" })));
        assert_eq!(loaded.tags, vec!["finance"]);
        assert_eq!(loaded.title.as_deref(), Some("Weekly"));
        assert_eq!(loaded.updated_at, "t1");
        assert!(store.session("nope").unwrap().is_none());

        assert!(store.thread_session("slack", "C1:1").unwrap().is_none());
        assert_eq!(store.bind_thread("slack", "C1:1", "s1").unwrap(), "s1");
        assert_eq!(
            store.bind_thread("slack", "C1:1", "s2").unwrap(),
            "s1",
            "the first binding wins"
        );
        assert_eq!(
            store.thread_session("slack", "C1:1").unwrap().as_deref(),
            Some("s1")
        );
    }

    #[tokio::test]
    async fn channel_state_survives_reopening_the_file() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("serve.db");
        let pending = PendingDelivery {
            channel: "slack".into(),
            session_id: "s1".into(),
            input_message_id: "m1".into(),
            target: everruns::channels::DeliveryTarget::new("C1", "1.2"),
            after_sequence: 4,
        };
        {
            let store = Store::open(&path).unwrap();
            assert!(store.first_sighting("slack", "Ev1").await.unwrap());
            assert!(!store.first_sighting("slack", "Ev1").await.unwrap());
            assert!(store.first_sighting("hook", "Ev1").await.unwrap());
            store
                .bind("slack", "slack:thread:C1:1.2", "s1")
                .await
                .unwrap();
            store.save_pending(&pending, "p", LEASE).await.unwrap();
            store.release_pending("s1", "m1", "p").await.unwrap();
        }
        let store = Store::open(&path).unwrap();
        assert!(!store.first_sighting("slack", "Ev1").await.unwrap());
        assert_eq!(
            store
                .session_for("slack", "slack:thread:C1:1.2")
                .await
                .unwrap()
                .as_deref(),
            Some("s1")
        );
        assert_eq!(
            store.claim_pending("p", LEASE).await.unwrap(),
            vec![pending]
        );
        store.clear_pending("s1", "m1").await.unwrap();
        assert!(store.claim_pending("p", LEASE).await.unwrap().is_empty());
    }

    #[tokio::test]
    async fn output_checkpoint_and_receipts_survive_reopen_with_fenced_takeover() {
        let directory = tempfile::tempdir().unwrap();
        let path = directory.path().join("serve.db");
        let pending = PendingDelivery {
            channel: "installation-a".into(),
            session_id: "s1".into(),
            input_message_id: "m1".into(),
            target: everruns::channels::DeliveryTarget::new("C1", "1.0"),
            after_sequence: 3,
        };
        let checkpoint = DeliveryCheckpoint {
            state: json!({"stream":"known-handle","sent":6}),
            ..Default::default()
        };
        {
            let store = Store::open(&path).unwrap();
            store
                .save_pending(&pending, "owner-a", LEASE)
                .await
                .unwrap();
            store
                .save_delivery_checkpoint("s1", "m1", "owner-a", &checkpoint)
                .await
                .unwrap();
            store
                .save_pending(&pending, "owner-b", LEASE)
                .await
                .unwrap();
            assert!(
                store
                    .complete_delivery("s1", "m1", "owner-b")
                    .await
                    .is_err()
            );
            assert!(
                store
                    .claim_pending("owner-b", LEASE)
                    .await
                    .unwrap()
                    .is_empty()
            );
            store.release_pending("s1", "m1", "owner-a").await.unwrap();
        }
        let store = Store::open(&path).unwrap();
        assert_eq!(
            store.claim_pending("owner-b", LEASE).await.unwrap(),
            [pending]
        );
        assert_eq!(
            store
                .delivery_checkpoint("s1", "m1")
                .await
                .unwrap()
                .unwrap()
                .state,
            checkpoint.state
        );
        assert!(
            store
                .save_delivery_checkpoint("s1", "m1", "owner-a", &checkpoint)
                .await
                .is_err()
        );
        assert!(
            store
                .complete_delivery("s1", "m1", "owner-a")
                .await
                .is_err()
        );
        store
            .save_delivery_checkpoint("s1", "m1", "owner-b", &checkpoint)
            .await
            .unwrap();
        store
            .complete_delivery("s1", "m1", "owner-b")
            .await
            .unwrap();
        assert!(
            store
                .delivery_checkpoint("s1", "m1")
                .await
                .unwrap()
                .is_none()
        );
    }

    #[tokio::test]
    async fn checkpoint_schema_upgrade_preserves_existing_bindings_and_pending_work() {
        let directory = tempfile::tempdir().unwrap();
        let path = directory.path().join("serve.db");
        let pending = PendingDelivery {
            channel: "slack".into(),
            session_id: "s1".into(),
            input_message_id: "m1".into(),
            target: everruns::channels::DeliveryTarget::new("C1", "1.0"),
            after_sequence: 3,
        };
        {
            let connection = rusqlite::open(&path).unwrap();
            connection.execute_batch("PRAGMA user_version=2; CREATE TABLE channel_threads (channel TEXT, thread TEXT, session_id TEXT, PRIMARY KEY(channel,thread)); INSERT INTO channel_threads VALUES('slack','C1:1.0','s1'); CREATE TABLE channel_pending (session_id TEXT, input_message_id TEXT, delivery TEXT, PRIMARY KEY(session_id,input_message_id));").unwrap();
            connection
                .execute(
                    "INSERT INTO channel_pending VALUES('s1','m1',?1)",
                    params![serde_json::to_string(&pending).unwrap()],
                )
                .unwrap();
        }
        let store = Store::open(&path).unwrap();
        assert_eq!(
            store
                .session_for("slack", "C1:1.0")
                .await
                .unwrap()
                .as_deref(),
            Some("s1")
        );
        assert_eq!(
            store.claim_pending("owner", LEASE).await.unwrap(),
            [pending]
        );
        store
            .save_delivery_checkpoint("s1", "m1", "owner", &DeliveryCheckpoint::default())
            .await
            .unwrap();
    }

    #[test]
    fn the_catalog_survives_reopening_the_file() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("serve.db");
        Store::open(&path).unwrap().insert_session(&row()).unwrap();
        let reopened = Store::open(&path).unwrap();
        assert!(reopened.session("s1").unwrap().is_some());
    }

    #[test]
    fn an_older_schema_is_replaced() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("serve.db");
        rusqlite::open(&path)
            .unwrap()
            .execute_batch(
                "CREATE TABLE sessions (id TEXT PRIMARY KEY, agent TEXT NOT NULL);
                 CREATE TABLE events (session_id TEXT, seq INTEGER);",
            )
            .unwrap();
        let store = Store::open(&path).unwrap();
        store.insert_session(&row()).unwrap();
        assert!(store.session("s1").unwrap().is_some());
    }
}
