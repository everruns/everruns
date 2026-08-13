//! SQLite-backed session identity catalog for local execution hosts.

use std::collections::HashMap;
use std::sync::{Arc, Mutex};

use async_trait::async_trait;
use everruns_core::AgentCapabilityConfig;
use everruns_core::error::{AgentLoopError, Result};
use everruns_core::session::ExecutionSession;
use everruns_core::traits::SessionStore;
use everruns_core::typed_id::{HarnessId, SessionId};
use everruns_host::{RuntimeSessionStore, SessionBuilder};
use everruns_platform::SessionMutator;
use rusqlite::{OptionalExtension, params};

use crate::SqliteDb;

/// Durable session identity catalog for local Framework hosts.
///
/// SQLite stores only session ids. Full runtime session configuration remains
/// in memory and is rebuilt from the resuming Agent, so MCP credentials and
/// other host configuration are not serialized into the catalog. Canonical
/// events remain the sole conversation write path.
// THREAT[TM-FS-017]: never add serialized Session/configuration columns here;
// durable resume needs identity only and Agent configuration may contain secrets.
#[derive(Clone)]
pub struct LocalSessionStore {
    db: SqliteDb,
    sessions: Arc<Mutex<HashMap<SessionId, ExecutionSession>>>,
}

impl LocalSessionStore {
    /// Create or open the catalog schema in `db`.
    pub fn new(db: SqliteDb) -> Result<Self> {
        db.with_conn(|conn| {
            conn.execute_batch(
                "CREATE TABLE IF NOT EXISTS framework_sessions (
                    session_id TEXT PRIMARY KEY NOT NULL
                );",
            )
        })
        .map_err(store_error)?;
        Ok(Self {
            db,
            sessions: Arc::new(Mutex::new(HashMap::new())),
        })
    }

    fn load(&self, session_id: SessionId) -> Result<Option<ExecutionSession>> {
        if let Some(session) = self
            .sessions
            .lock()
            .map_err(|_| AgentLoopError::store("local session catalog lock poisoned"))?
            .get(&session_id)
            .cloned()
        {
            return Ok(Some(session));
        }
        let exists = self
            .db
            .with_conn(|conn| {
                conn.query_row(
                    "SELECT 1 FROM framework_sessions WHERE session_id = ?1",
                    params![session_id.to_string()],
                    |_| Ok(()),
                )
                .optional()
            })
            .map_err(store_error)?
            .is_some();
        Ok(exists.then(|| SessionBuilder::new(HarnessId::new()).id(session_id).build()))
    }

    fn mutate(
        &self,
        session_id: SessionId,
        update: impl FnOnce(&mut ExecutionSession),
    ) -> Result<ExecutionSession> {
        let mut session = self
            .load(session_id)?
            .ok_or_else(|| AgentLoopError::store(format!("session not found: {session_id}")))?;
        update(&mut session);
        self.sessions
            .lock()
            .map_err(|_| AgentLoopError::store("local session catalog lock poisoned"))?
            .insert(session_id, session.clone());
        Ok(session)
    }
}

#[async_trait]
impl SessionStore for LocalSessionStore {
    async fn get_session(&self, session_id: SessionId) -> Result<Option<ExecutionSession>> {
        self.load(session_id)
    }
}

#[async_trait]
impl SessionMutator for LocalSessionStore {
    async fn update_session_title(
        &self,
        session_id: SessionId,
        title: String,
    ) -> Result<ExecutionSession> {
        self.mutate(session_id, |session| session.title = Some(title))
    }

    async fn upsert_session_capability(
        &self,
        session_id: SessionId,
        capability: AgentCapabilityConfig,
    ) -> Result<ExecutionSession> {
        self.mutate(session_id, |session| {
            if let Some(existing) = session
                .capabilities
                .iter_mut()
                .find(|existing| existing.capability_id() == capability.capability_id())
            {
                *existing = capability;
            } else {
                session.capabilities.push(capability);
            }
        })
    }

    async fn remove_session_capability(
        &self,
        session_id: SessionId,
        capability_id: &str,
    ) -> Result<ExecutionSession> {
        self.mutate(session_id, |session| {
            session
                .capabilities
                .retain(|capability| capability.capability_id() != capability_id);
        })
    }
}

#[async_trait]
impl RuntimeSessionStore for LocalSessionStore {
    async fn add_session(&self, session: ExecutionSession) -> Result<()> {
        let session_id = session.id;
        self.db
            .with_conn(|conn| {
                conn.execute(
                    "INSERT INTO framework_sessions (session_id) VALUES (?1)
                     ON CONFLICT(session_id) DO NOTHING",
                    params![session_id.to_string()],
                )?;
                Ok(())
            })
            .map_err(store_error)?;
        self.sessions
            .lock()
            .map_err(|_| AgentLoopError::store("local session catalog lock poisoned"))?
            .insert(session_id, session);
        Ok(())
    }
}

fn store_error(error: impl std::fmt::Display) -> AgentLoopError {
    AgentLoopError::store(error.to_string())
}

#[cfg(test)]
mod tests {
    use everruns_core::traits::SessionStore;
    use everruns_host::RuntimeSessionStore;

    use super::*;

    #[tokio::test]
    async fn catalog_survives_reopen_without_persisting_runtime_configuration() {
        let root = tempfile::tempdir().unwrap();
        let path = root.path().join("local.db");
        let session_id = SessionId::new();

        let store = LocalSessionStore::new(SqliteDb::open(&path).unwrap()).unwrap();
        let session = SessionBuilder::new(HarnessId::new()).id(session_id).build();
        store.add_session(session).await.unwrap();
        store
            .update_session_title(session_id, "resumable".into())
            .await
            .unwrap();
        drop(store);

        let reopened = LocalSessionStore::new(SqliteDb::open(&path).unwrap()).unwrap();
        let restored = reopened.get_session(session_id).await.unwrap().unwrap();
        assert_eq!(restored.id, session_id);
        assert_eq!(restored.title, None);

        let columns = reopened
            .db
            .with_conn(|conn| {
                let mut statement = conn.prepare("PRAGMA table_info(framework_sessions)")?;
                statement
                    .query_map([], |row| row.get::<_, String>(1))?
                    .collect::<rusqlite::Result<Vec<_>>>()
            })
            .unwrap();
        assert_eq!(columns, vec!["session_id"]);
    }

    #[tokio::test]
    async fn missing_catalog_identity_is_not_a_placeholder_session() {
        let db = SqliteDb::open_in_memory().unwrap();
        let store = LocalSessionStore::new(db).unwrap();
        let session_id = SessionId::new();

        assert!(store.get_session(session_id).await.unwrap().is_none());
    }
}
