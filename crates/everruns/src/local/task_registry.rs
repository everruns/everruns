// SQLite-backed SessionTaskRegistry.
//
// Persists tasks and their message channel so a freshly-spawned process can
// reopen the database file and read / continue / inspect tasks
// (restart-survivability). Lifecycle invariants are NOT reimplemented here:
// every update routes through `everruns_core::session_task::apply_task_update`,
// matching the postgres / in-memory backends.
//
// Storage shape: one `local_tasks` row per task. The full `SessionTask` is
// stored as a JSON snapshot for faithful round-tripping; `session_id`, `kind`,
// and `state` are also stored as plain columns so `SessionTaskFilter` queries
// hit an index instead of deserializing every row. Messages live in
// `local_task_messages`, ordered by an autoincrement `seq` to give a stable
// oldest-first order and a cheap `after_id` cursor.

use crate::sqlite as rusqlite;
use async_trait::async_trait;
use chrono::Utc;
use everruns_contracts::error::{AgentLoopError, Result};
use everruns_contracts::typed_id::SessionId;
use everruns_core::session_task::{
    CreateSessionTask, NewTaskMessage, SessionTask, SessionTaskFilter, SessionTaskRegistry,
    SessionTaskState, SessionTaskUpdate, TaskMessage, TaskMessageDirection, apply_task_update,
    generate_task_message_id, new_session_task,
};
use rusqlite::OptionalExtension;

use super::db::SqliteDb;
use super::error::LocalError;

/// SQLite-backed task registry for local embedded hosts.
#[derive(Clone)]
pub struct LocalSessionTaskRegistry {
    db: SqliteDb,
}

impl LocalSessionTaskRegistry {
    /// Open (and migrate) a registry over the given database handle.
    pub fn new(db: SqliteDb) -> Result<Self> {
        db.with_conn(|conn| {
            conn.execute_batch(
                "CREATE TABLE IF NOT EXISTS local_tasks (
                    id          TEXT PRIMARY KEY,
                    session_id  TEXT NOT NULL,
                    kind        TEXT NOT NULL,
                    state       TEXT NOT NULL,
                    snapshot    TEXT NOT NULL
                 );
                 CREATE INDEX IF NOT EXISTS idx_local_tasks_session
                    ON local_tasks(session_id);
                 CREATE TABLE IF NOT EXISTS local_task_messages (
                    seq         INTEGER PRIMARY KEY AUTOINCREMENT,
                    id          TEXT NOT NULL UNIQUE,
                    task_id     TEXT NOT NULL,
                    snapshot    TEXT NOT NULL,
                    FOREIGN KEY(task_id) REFERENCES local_tasks(id)
                 );
                 CREATE INDEX IF NOT EXISTS idx_local_task_messages_task
                    ON local_task_messages(task_id, seq);
                 CREATE TABLE IF NOT EXISTS local_task_recovery_wakes (
                    session_id TEXT NOT NULL,
                    task_id    TEXT NOT NULL,
                    attempt    INTEGER NOT NULL,
                    snapshot   TEXT NOT NULL,
                    PRIMARY KEY(session_id, task_id, attempt),
                    FOREIGN KEY(task_id) REFERENCES local_tasks(id)
                 );",
            )
        })
        .map_err(AgentLoopError::from)?;
        Ok(Self { db })
    }

    // Persist the terminal wake with its transition. Delivery is at least once; consumers
    // must make terminal wakes idempotent by task id and attempt.
    fn store_recovery_wake(conn: &::rusqlite::Connection, task: &SessionTask) -> Result<()> {
        // Preserve only the internal origin needed for delayed wakes. Keep the
        // public serializer's secret redaction for the rest of the outbox spec.
        let mut snapshot = serde_json::to_value(task).map_err(LocalError::from)?;
        if let Some(context) = everruns_core::session_task::task_origin_event_context(task) {
            everruns_core::session_task::set_task_origin_event_context(
                &mut snapshot["spec"],
                &context,
            );
        }
        everruns_core::causal::task_causal_context(task).write_task_context(&mut snapshot["spec"]);
        let snapshot = serde_json::to_string(&snapshot).map_err(LocalError::from)?;
        conn.execute(
            "INSERT OR IGNORE INTO local_task_recovery_wakes
                (session_id, task_id, attempt, snapshot) VALUES (?1, ?2, ?3, ?4)",
            rusqlite::params![task.session_id.to_string(), task.id, task.attempt, snapshot],
        )
        .map_err(LocalError::from)?;
        Ok(())
    }

    pub(crate) fn pending_recovery_wakes(
        db: &SqliteDb,
        session_id: Option<SessionId>,
    ) -> Result<Vec<SessionTask>> {
        let snapshots = db
            .with_conn(|conn| {
                let session_id = session_id.map(|id| id.to_string());
                let mut statement = conn.prepare(
                    "SELECT snapshot FROM local_task_recovery_wakes
                     WHERE (?1 IS NULL OR session_id = ?1) ORDER BY task_id, attempt",
                )?;
                statement
                    .query_map([session_id.as_deref()], |row| row.get::<_, String>(0))?
                    .collect::<std::result::Result<Vec<_>, _>>()
            })
            .map_err(AgentLoopError::from)?;
        snapshots
            .into_iter()
            .map(|snapshot| {
                serde_json::from_str(&snapshot)
                    .map_err(LocalError::from)
                    .map_err(AgentLoopError::from)
            })
            .collect()
    }

    pub(crate) fn acknowledge_recovery_wake(db: &SqliteDb, task: &SessionTask) -> Result<()> {
        db.with_conn(|conn| {
            conn.execute(
                "DELETE FROM local_task_recovery_wakes
                 WHERE session_id = ?1 AND task_id = ?2 AND attempt = ?3",
                rusqlite::params![task.session_id.to_string(), task.id, task.attempt],
            )?;
            Ok(())
        })
        .map_err(AgentLoopError::from)
    }

    fn load_task(&self, task_id: &str) -> Result<Option<SessionTask>> {
        self.db
            .with_conn(|conn| Ok(Self::load_task_from(conn, task_id)))
            .map_err(AgentLoopError::from)?
    }

    fn load_task_from(
        conn: &rusqlite::QueryConnection,
        task_id: &str,
    ) -> Result<Option<SessionTask>> {
        let snapshot: Option<String> = conn
            .query_row(
                "SELECT snapshot FROM local_tasks WHERE id = ?1",
                [task_id],
                |row| row.get(0),
            )
            .optional()
            .map_err(LocalError::from)?;
        snapshot
            .map(|json| {
                serde_json::from_str(&json).map_err(|e| AgentLoopError::from(LocalError::from(e)))
            })
            .transpose()
    }

    fn store_task(conn: &rusqlite::QueryConnection, task: &SessionTask) -> Result<()> {
        // SessionTask's public serializer redacts webhook secrets. Local snapshots
        // are persistence, not presentation, so restore the raw spec before storage.
        let mut snapshot =
            serde_json::to_value(task).map_err(|e| AgentLoopError::from(LocalError::from(e)))?;
        snapshot["spec"] = task.spec.clone();
        let snapshot = serde_json::to_string(&snapshot)
            .map_err(|e| AgentLoopError::from(LocalError::from(e)))?;
        conn.execute(
            "INSERT INTO local_tasks (id, session_id, kind, state, snapshot)
             VALUES (?1, ?2, ?3, ?4, ?5)
             ON CONFLICT(id) DO UPDATE SET
                session_id = excluded.session_id,
                kind = excluded.kind,
                state = excluded.state,
                snapshot = excluded.snapshot",
            rusqlite::params![
                task.id,
                task.session_id.to_string(),
                task.kind,
                task.state.to_string(),
                snapshot,
            ],
        )
        .map_err(LocalError::from)?;
        Ok(())
    }

    fn with_task_transaction<T>(
        &self,
        operation: impl FnOnce(&rusqlite::QueryConnection) -> Result<T>,
    ) -> Result<T> {
        self.db
            .with_conn_mut(|conn| {
                // Lock the writer before reading: the connection mutex alone
                // cannot serialize registries opened on the same file.
                let tx =
                    conn.transaction_with_behavior(rusqlite::TransactionBehavior::Immediate)?;
                let result = operation(&tx);
                if result.is_ok() {
                    tx.commit()?;
                }
                Ok(result)
            })
            .map_err(AgentLoopError::from)?
    }
}

#[async_trait]
impl SessionTaskRegistry for LocalSessionTaskRegistry {
    async fn create(&self, input: CreateSessionTask) -> Result<SessionTask> {
        self.with_task_transaction(|conn| {
            // Idempotent on a caller-supplied id, but only within the same session.
            // Reusing an id across sessions is rejected, matching the canonical
            // DB-backed registry, so a caller cannot alias another session's task.
            if let Some(id) = &input.id
                && let Some(existing) = Self::load_task_from(conn, id)?
            {
                if existing.session_id == input.session_id {
                    return Ok(existing);
                }
                return Err(AgentLoopError::store(format!(
                    "task id {id} already exists under a different session"
                )));
            }
            let task = new_session_task(input, Utc::now());
            Self::store_task(conn, &task)?;
            Ok(task)
        })
    }

    async fn update(
        &self,
        session_id: SessionId,
        task_id: &str,
        update: SessionTaskUpdate,
    ) -> Result<Option<SessionTask>> {
        self.with_task_transaction(|conn| {
            let Some(mut task) = Self::load_task_from(conn, task_id)? else {
                return Ok(None);
            };
            // Session-scoped: ignore updates targeting a task in another session.
            if task.session_id != session_id {
                return Ok(None);
            }
            if update.expected_recovery.as_ref().is_some_and(|guard| {
                !guard.matches(&task)
                    || update
                        .expected_attempt
                        .is_some_and(|attempt| attempt != task.attempt)
            }) {
                return Ok(None);
            }
            let recovery_wake = update.expected_recovery.is_some()
                && update.increment_attempt
                && update.state.is_some_and(|state| state.is_terminal());
            let prior_attempt = task.attempt;
            apply_task_update(&mut task, update, Utc::now());
            Self::store_task(conn, &task)?;
            if recovery_wake && task.state.is_terminal() && task.attempt != prior_attempt {
                Self::store_recovery_wake(conn, &task)?;
            }
            Ok(Some(task))
        })
    }

    async fn get(&self, session_id: SessionId, task_id: &str) -> Result<Option<SessionTask>> {
        // Session-scoped: a task id from another session is not visible here.
        Ok(self
            .load_task(task_id)?
            .filter(|task| task.session_id == session_id))
    }

    async fn list(
        &self,
        session_id: SessionId,
        filter: Option<&SessionTaskFilter>,
    ) -> Result<Vec<SessionTask>> {
        let session = session_id.to_string();
        let kind = filter.and_then(|f| f.kind.clone());
        let state = filter.and_then(|f| f.state.map(|s| s.to_string()));
        let snapshots: Vec<String> = self
            .db
            .with_conn(|conn| {
                // Build the query with optional kind/state predicates. Bind
                // params positionally to keep the prepared statement simple.
                let mut sql =
                    String::from("SELECT snapshot FROM local_tasks WHERE session_id = ?1");
                if kind.is_some() {
                    sql.push_str(" AND kind = ?2");
                }
                if state.is_some() {
                    // ?3 if kind present, else ?2 — rusqlite positional binding
                    // tolerates gaps, so always use ?3 and bind kind as NULL
                    // when absent is not possible; instead branch explicitly.
                    sql.push_str(if kind.is_some() {
                        " AND state = ?3"
                    } else {
                        " AND state = ?2"
                    });
                }
                sql.push_str(" ORDER BY rowid ASC");

                let mut stmt = conn.prepare(&sql)?;
                let rows = match (&kind, &state) {
                    (Some(k), Some(s)) => stmt
                        .query_map(rusqlite::params![session, k, s], |row| row.get(0))?
                        .collect::<rusqlite::Result<Vec<String>>>()?,
                    (Some(k), None) => stmt
                        .query_map(rusqlite::params![session, k], |row| row.get(0))?
                        .collect::<rusqlite::Result<Vec<String>>>()?,
                    (None, Some(s)) => stmt
                        .query_map(rusqlite::params![session, s], |row| row.get(0))?
                        .collect::<rusqlite::Result<Vec<String>>>()?,
                    (None, None) => stmt
                        .query_map(rusqlite::params![session], |row| row.get(0))?
                        .collect::<rusqlite::Result<Vec<String>>>()?,
                };
                Ok(rows)
            })
            .map_err(AgentLoopError::from)?;
        snapshots
            .into_iter()
            .map(|json| {
                serde_json::from_str(&json).map_err(|e| AgentLoopError::from(LocalError::from(e)))
            })
            .collect()
    }

    async fn request_cancel(
        &self,
        session_id: SessionId,
        task_id: &str,
    ) -> Result<Option<SessionTask>> {
        self.with_task_transaction(|conn| {
            let Some(mut task) = Self::load_task_from(conn, task_id)? else {
                return Ok(None);
            };
            if task.session_id != session_id {
                return Ok(None);
            }
            if !task.state.is_terminal() && task.cancel_requested_at.is_none() {
                let now = Utc::now();
                task.cancel_requested_at = Some(now);
                task.updated_at = now;
                Self::store_task(conn, &task)?;
            }
            Ok(Some(task))
        })
    }

    async fn record_message(
        &self,
        session_id: SessionId,
        task_id: &str,
        message: NewTaskMessage,
    ) -> Result<TaskMessage> {
        self.with_task_transaction(|conn| {
            // Session-scoped: a message may only be appended to a task that belongs
            // to the calling session (mirrors the DB-backed registry).
            let mut task = Self::load_task_from(conn, task_id)?
                .filter(|task| task.session_id == session_id)
                .ok_or_else(|| AgentLoopError::tool(format!("no task {task_id}")))?;
            // Stale-attempt fence: reject writes from a superseded executor so the
            // thread cannot grow under a zombie. Mirrors the postgres backend.
            if let Some(expected) = message.expected_attempt
                && expected != task.attempt
            {
                return Err(AgentLoopError::tool(format!(
                    "stale attempt for task {task_id}: expected {expected}, current {}",
                    task.attempt
                )));
            }

            let record = TaskMessage {
                id: generate_task_message_id(),
                task_id: task_id.to_string(),
                direction: message.direction,
                content: message.content,
                in_reply_to: message.in_reply_to.clone(),
                created_at: Utc::now(),
            };
            let snapshot = serde_json::to_string(&record)
                .map_err(|e| AgentLoopError::from(LocalError::from(e)))?;
            let id = record.id.clone();
            let tid = task_id.to_string();
            conn.execute(
                "INSERT INTO local_task_messages (id, task_id, snapshot)
             VALUES (?1, ?2, ?3)",
                rusqlite::params![id, tid, snapshot],
            )
            .map_err(LocalError::from)?;

            // An inbound answer (in_reply_to set) clears a matching pending input
            // request and returns the task to running. Only inbound messages resume
            // the task, matching the DB-backed registry; outbound messages never do.
            if message.direction == TaskMessageDirection::Inbound
                && let Some(reply_id) = &message.in_reply_to
                && task
                    .input_request
                    .as_ref()
                    .is_some_and(|req| &req.id == reply_id)
            {
                apply_task_update(
                    &mut task,
                    SessionTaskUpdate {
                        state: Some(SessionTaskState::Running),
                        ..Default::default()
                    },
                    Utc::now(),
                );
                Self::store_task(conn, &task)?;
            }

            Ok(record)
        })
    }

    async fn list_messages(
        &self,
        session_id: SessionId,
        task_id: &str,
        limit: Option<u32>,
        after_id: Option<&str>,
    ) -> Result<Vec<TaskMessage>> {
        // Session-scoped: do not leak another session's message history even
        // when the task id is known. Missing/foreign task -> empty list.
        if self.get(session_id, task_id).await?.is_none() {
            return Ok(Vec::new());
        }
        let tid = task_id.to_string();
        let after = after_id.map(|s| s.to_string());
        let limit = limit.map(|l| l as i64);
        let snapshots: Vec<String> = self
            .db
            .with_conn(|conn| {
                // Resolve the exclusive cursor seq, if any.
                let after_seq: Option<i64> = match &after {
                    Some(id) => conn
                        .query_row(
                            "SELECT seq FROM local_task_messages WHERE id = ?1",
                            [id],
                            |row| row.get(0),
                        )
                        .optional()?,
                    None => None,
                };
                let mut sql =
                    String::from("SELECT snapshot FROM local_task_messages WHERE task_id = ?1");
                if after_seq.is_some() {
                    sql.push_str(" AND seq > ?2");
                }
                sql.push_str(" ORDER BY seq ASC");
                if limit.is_some() {
                    sql.push_str(if after_seq.is_some() {
                        " LIMIT ?3"
                    } else {
                        " LIMIT ?2"
                    });
                }
                let mut stmt = conn.prepare(&sql)?;
                let rows = match (after_seq, limit) {
                    (Some(seq), Some(lim)) => stmt
                        .query_map(rusqlite::params![tid, seq, lim], |row| row.get(0))?
                        .collect::<rusqlite::Result<Vec<String>>>()?,
                    (Some(seq), None) => stmt
                        .query_map(rusqlite::params![tid, seq], |row| row.get(0))?
                        .collect::<rusqlite::Result<Vec<String>>>()?,
                    (None, Some(lim)) => stmt
                        .query_map(rusqlite::params![tid, lim], |row| row.get(0))?
                        .collect::<rusqlite::Result<Vec<String>>>()?,
                    (None, None) => stmt
                        .query_map(rusqlite::params![tid], |row| row.get(0))?
                        .collect::<rusqlite::Result<Vec<String>>>()?,
                };
                Ok(rows)
            })
            .map_err(AgentLoopError::from)?;
        snapshots
            .into_iter()
            .map(|json| {
                serde_json::from_str(&json).map_err(|e| AgentLoopError::from(LocalError::from(e)))
            })
            .collect()
    }
}
