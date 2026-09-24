//! The host's own SQLite: session catalog, the wire event log, and channel
//! thread → session routing.
//!
//! Decision: conversation state stays in everruns (`LocalConfig`, its durable
//! canonical event log); this store keeps only what the wire API needs. The
//! `events` table is an ordered per-session log of the events the API emits,
//! and its `seq` is the SSE `id`, so `Last-Event-ID` resumes exactly after
//! the last event a client saw. A production host would read the canonical
//! log directly instead of mirroring it.

use std::path::Path;
use std::sync::Mutex;

use rusqlite::{Connection, OptionalExtension, params};
use serde::{Deserialize, Serialize};
use serde_json::Value;

/// One event in a session's wire log.
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
pub struct WireEvent {
    /// Position in the session's log; the SSE event id and resume cursor.
    pub seq: i64,
    pub session_id: String,
    #[serde(rename = "type")]
    pub kind: String,
    pub at: String,
    pub data: Value,
}

/// A session row.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct SessionRow {
    pub id: String,
    pub agent: String,
    /// The build the session started on. Hosts route it back there.
    pub build_id: String,
    pub created_at: String,
    pub metadata: Value,
    /// `channel:target` for sessions whose replies are delivered somewhere.
    pub deliver_to: Option<String>,
}

pub(crate) struct Store {
    conn: Mutex<Connection>,
}

impl Store {
    pub(crate) fn open(path: &Path) -> crate::Result<Self> {
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent)?;
        }
        Self::init(Connection::open(path)?)
    }

    pub(crate) fn in_memory() -> crate::Result<Self> {
        Self::init(Connection::open_in_memory()?)
    }

    fn init(conn: Connection) -> crate::Result<Self> {
        conn.execute_batch(
            "PRAGMA journal_mode = WAL;
             CREATE TABLE IF NOT EXISTS sessions (
                 id TEXT PRIMARY KEY,
                 agent TEXT NOT NULL,
                 build_id TEXT NOT NULL,
                 created_at TEXT NOT NULL,
                 metadata TEXT NOT NULL,
                 deliver_to TEXT
             );
             CREATE TABLE IF NOT EXISTS events (
                 session_id TEXT NOT NULL,
                 seq INTEGER NOT NULL,
                 kind TEXT NOT NULL,
                 at TEXT NOT NULL,
                 data TEXT NOT NULL,
                 PRIMARY KEY (session_id, seq)
             );
             CREATE TABLE IF NOT EXISTS channel_threads (
                 channel TEXT NOT NULL,
                 thread TEXT NOT NULL,
                 session_id TEXT NOT NULL,
                 PRIMARY KEY (channel, thread)
             );",
        )?;
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
            "INSERT INTO sessions (id, agent, build_id, created_at, metadata, deliver_to)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6)",
            params![
                row.id,
                row.agent,
                row.build_id,
                row.created_at,
                row.metadata.to_string(),
                row.deliver_to
            ],
        )?;
        Ok(())
    }

    pub(crate) fn session(&self, id: &str) -> crate::Result<Option<SessionRow>> {
        let row = self
            .conn()
            .query_row(
                "SELECT id, agent, build_id, created_at, metadata, deliver_to FROM sessions WHERE id = ?1",
                params![id],
                |row| {
                    Ok((
                        row.get::<_, String>(0)?,
                        row.get::<_, String>(1)?,
                        row.get::<_, String>(2)?,
                        row.get::<_, String>(3)?,
                        row.get::<_, String>(4)?,
                        row.get::<_, Option<String>>(5)?,
                    ))
                },
            )
            .optional()?;
        Ok(row.map(
            |(id, agent, build_id, created_at, metadata, deliver_to)| SessionRow {
                id,
                agent,
                build_id,
                created_at,
                metadata: serde_json::from_str(&metadata).unwrap_or(Value::Null),
                deliver_to,
            },
        ))
    }

    /// Append to a session's log. Sequence numbers are assigned under the
    /// connection lock, so they are dense and ordered per session.
    pub(crate) fn append(
        &self,
        session_id: &str,
        kind: &str,
        data: Value,
    ) -> crate::Result<WireEvent> {
        let conn = self.conn();
        let seq: i64 = conn.query_row(
            "SELECT COALESCE(MAX(seq), 0) + 1 FROM events WHERE session_id = ?1",
            params![session_id],
            |row| row.get(0),
        )?;
        let at = chrono::Utc::now().to_rfc3339_opts(chrono::SecondsFormat::Millis, true);
        conn.execute(
            "INSERT INTO events (session_id, seq, kind, at, data) VALUES (?1, ?2, ?3, ?4, ?5)",
            params![session_id, seq, kind, at, data.to_string()],
        )?;
        Ok(WireEvent {
            seq,
            session_id: session_id.to_string(),
            kind: kind.to_string(),
            at,
            data,
        })
    }

    /// Events strictly after `cursor`, oldest first.
    pub(crate) fn events_after(
        &self,
        session_id: &str,
        cursor: i64,
    ) -> crate::Result<Vec<WireEvent>> {
        let conn = self.conn();
        let mut statement = conn.prepare(
            "SELECT seq, kind, at, data FROM events WHERE session_id = ?1 AND seq > ?2 ORDER BY seq",
        )?;
        let rows = statement.query_map(params![session_id, cursor], |row| {
            Ok(WireEvent {
                seq: row.get(0)?,
                session_id: session_id.to_string(),
                kind: row.get(1)?,
                at: row.get(2)?,
                data: serde_json::from_str(&row.get::<_, String>(3)?).unwrap_or(Value::Null),
            })
        })?;
        Ok(rows.collect::<Result<_, _>>()?)
    }

    /// Whether the runtime's terminal event for `turn_id` is in the log.
    pub(crate) fn turn_logged(&self, session_id: &str, turn_id: &str) -> crate::Result<bool> {
        Ok(self
            .conn()
            .query_row(
                "SELECT 1 FROM events WHERE session_id = ?1
                   AND kind IN ('turn.completed', 'turn.failed', 'turn.cancelled')
                   AND json_extract(data, '$.turn_id') = ?2 LIMIT 1",
                params![session_id, turn_id],
                |_| Ok(()),
            )
            .optional()?
            .is_some())
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

    pub(crate) fn bind_thread(
        &self,
        channel: &str,
        thread: &str,
        session_id: &str,
    ) -> crate::Result {
        self.conn().execute(
            "INSERT OR REPLACE INTO channel_threads (channel, thread, session_id) VALUES (?1, ?2, ?3)",
            params![channel, thread, session_id],
        )?;
        Ok(())
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

    #[test]
    fn events_are_dense_per_session_and_resume_after_a_cursor() {
        let store = Store::in_memory().unwrap();
        for i in 0..3 {
            store.append("a", "x", json!({ "i": i })).unwrap();
        }
        store.append("b", "x", json!({})).unwrap();
        let all = store.events_after("a", 0).unwrap();
        assert_eq!(all.iter().map(|e| e.seq).collect::<Vec<_>>(), vec![1, 2, 3]);
        let rest = store.events_after("a", 2).unwrap();
        assert_eq!(rest.len(), 1);
        assert_eq!(rest[0].data, json!({ "i": 2 }));
        assert_eq!(store.events_after("b", 0).unwrap()[0].seq, 1);
    }

    #[test]
    fn sessions_and_threads_round_trip() {
        let store = Store::in_memory().unwrap();
        let row = SessionRow {
            id: "s1".into(),
            agent: "analyst".into(),
            build_id: "b1".into(),
            created_at: "now".into(),
            metadata: json!({ "k": "v" }),
            deliver_to: Some("slack:C1".into()),
        };
        store.insert_session(&row).unwrap();
        let loaded = store.session("s1").unwrap().unwrap();
        assert_eq!(loaded.metadata, json!({ "k": "v" }));
        assert_eq!(loaded.deliver_to.as_deref(), Some("slack:C1"));
        assert!(store.session("nope").unwrap().is_none());

        assert!(store.thread_session("slack", "C1:1").unwrap().is_none());
        store.bind_thread("slack", "C1:1", "s1").unwrap();
        assert_eq!(
            store.thread_session("slack", "C1:1").unwrap().as_deref(),
            Some("s1")
        );
    }

    #[test]
    fn the_log_survives_reopening_the_file() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("serve.db");
        Store::open(&path)
            .unwrap()
            .append("a", "x", json!(1))
            .unwrap();
        let reopened = Store::open(&path).unwrap();
        assert_eq!(reopened.append("a", "x", json!(2)).unwrap().seq, 2);
    }
}
