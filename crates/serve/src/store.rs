//! The host's own SQLite: the session catalog and channel thread → session
//! routing.
//!
//! Decision: serve keeps no event log. Conversation state and the durable
//! canonical event log are the everruns engine's (`LocalConfig` in dev and
//! start, in memory in evals); the wire API reads them through
//! `Session::events_after` / `events_from`. This store keeps only what the
//! engine does not know: which agent and build a session runs, its title,
//! tags and metadata, and where its replies are delivered.
//!
//! No compatibility with earlier serve databases: an older schema is dropped.

use std::path::Path;
use std::sync::Mutex;

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
                 updated_at TEXT NOT NULL,
                 deliver_to TEXT
             );
             CREATE TABLE IF NOT EXISTS channel_threads (
                 channel TEXT NOT NULL,
                 thread TEXT NOT NULL,
                 session_id TEXT NOT NULL,
                 PRIMARY KEY (channel, thread)
             );
             PRAGMA user_version = {SCHEMA_VERSION};"
        ))?;
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
               (id, agent, build_id, title, tags, hints, metadata, created_at, updated_at, deliver_to)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10)",
            params![
                row.id,
                row.agent,
                row.build_id,
                row.title,
                serde_json::to_string(&row.tags)?,
                row.hints.as_ref().map(Value::to_string),
                row.metadata.as_ref().map(Value::to_string),
                row.created_at,
                row.updated_at,
                row.deliver_to
            ],
        )?;
        Ok(())
    }

    pub(crate) fn session(&self, id: &str) -> crate::Result<Option<SessionRow>> {
        let json = |text: Option<String>| text.and_then(|text| serde_json::from_str(&text).ok());
        Ok(self
            .conn()
            .query_row(
                "SELECT id, agent, build_id, title, tags, hints, metadata, created_at, updated_at, deliver_to
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
                        deliver_to: row.get(9)?,
                    })
                },
            )
            .optional()?)
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
            deliver_to: Some("slack:C1".into()),
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
        Connection::open(&path)
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
