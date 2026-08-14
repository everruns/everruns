//! SQLite-backed session identity catalog for local execution hosts.

use std::collections::HashMap;
use std::sync::{Arc, Mutex};

use async_trait::async_trait;
use everruns_capability::CapabilityRef as AgentCapabilityConfig;
use everruns_core::execution_loading::SessionStore;
use everruns_core::session::ExecutionSession;
use everruns_host::{
    EnvironmentBindingError, EnvironmentBindingStore, RuntimeSessionStore, SessionBuilder,
    WorkspaceBinding,
};
use everruns_platform::SessionMutator;
use everruns_provider::error::{AgentLoopError, Result};
use everruns_provider::typed_id::{HarnessId, SessionId};
use rusqlite::{OptionalExtension, params};

use crate::SqliteDb;

/// Durable session identity catalog for local Framework hosts.
///
/// SQLite stores session ids plus the provider-owned, credential-free opaque
/// workspace binding required to reopen the exact head. Full runtime session
/// configuration remains in memory and is rebuilt from the resuming Agent, so
/// MCP credentials and other host configuration are not serialized. Canonical
/// events remain the sole conversation write path.
// THREAT[TM-FS-017]: never serialize Agent/Session configuration or credentials
// here. Environment bindings are size-bounded and providers must keep secrets
// out of their opaque payload.
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
                );
                 CREATE TABLE IF NOT EXISTS framework_session_environments (
                    session_id TEXT PRIMARY KEY NOT NULL,
                    binding_json TEXT NOT NULL,
                    FOREIGN KEY(session_id) REFERENCES framework_sessions(session_id)
                );
                 CREATE TABLE IF NOT EXISTS framework_workspace_head_claims (
                    provider_id TEXT NOT NULL,
                    workspace_id TEXT NOT NULL,
                    head_id TEXT NOT NULL,
                    access TEXT NOT NULL CHECK (access IN ('isolated', 'shared')),
                    owner_session_id TEXT,
                    PRIMARY KEY(provider_id, workspace_id, head_id),
                    FOREIGN KEY(owner_session_id) REFERENCES framework_sessions(session_id)
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

// serde_json represents Vec<u8> as decimal values, so the encoded ceiling is
// larger than the provider payload ceiling while remaining strictly bounded.
const MAX_BINDING_BYTES: usize = WorkspaceBinding::MAX_PAYLOAD_BYTES * 4 + 4096;

#[async_trait]
impl EnvironmentBindingStore for LocalSessionStore {
    async fn load(
        &self,
        session_id: SessionId,
    ) -> std::result::Result<Option<WorkspaceBinding>, EnvironmentBindingError> {
        let encoded = self
            .db
            .with_conn(|conn| {
                conn.query_row(
                    "SELECT binding_json FROM framework_session_environments WHERE session_id = ?1",
                    params![session_id.to_string()],
                    |row| row.get::<_, String>(0),
                )
                .optional()
            })
            .map_err(|_| EnvironmentBindingError::Unavailable)?;
        let Some(encoded) = encoded else {
            return Ok(None);
        };
        if encoded.len() > MAX_BINDING_BYTES {
            return Err(EnvironmentBindingError::Corrupt);
        }
        let binding: WorkspaceBinding =
            serde_json::from_str(&encoded).map_err(|_| EnvironmentBindingError::Corrupt)?;
        if binding.payload.len() > WorkspaceBinding::MAX_PAYLOAD_BYTES {
            return Err(EnvironmentBindingError::Corrupt);
        }
        Ok(Some(binding))
    }

    async fn bind(
        &self,
        session_id: SessionId,
        binding: &WorkspaceBinding,
    ) -> std::result::Result<(), EnvironmentBindingError> {
        if binding.payload.len() > WorkspaceBinding::MAX_PAYLOAD_BYTES {
            return Err(EnvironmentBindingError::Corrupt);
        }
        let encoded =
            serde_json::to_string(binding).map_err(|_| EnvironmentBindingError::Corrupt)?;
        if encoded.len() > MAX_BINDING_BYTES {
            return Err(EnvironmentBindingError::Corrupt);
        }
        self.db
            .with_conn_mut(|conn| {
                let transaction = conn.transaction()?;
                transaction.execute(
                    "INSERT INTO framework_sessions (session_id) VALUES (?1)
                     ON CONFLICT(session_id) DO NOTHING",
                    params![session_id.to_string()],
                )?;
                let recorded = transaction
                    .query_row(
                        "SELECT binding_json FROM framework_session_environments WHERE session_id = ?1",
                        params![session_id.to_string()],
                        |row| row.get::<_, String>(0),
                    )
                    .optional()?;
                if let Some(recorded) = recorded {
                    if recorded != encoded {
                        return Err(rusqlite::Error::InvalidParameterName(
                            "environment binding conflict".into(),
                        ));
                    }
                } else {
                    let provider_id = binding.provider_id.to_string();
                    let workspace_id = binding.workspace_id.to_string();
                    let head_id = binding.head_id.to_string();
                    let access = match binding.access {
                        everruns_host::WorkspaceHeadAccess::Isolated => "isolated",
                        everruns_host::WorkspaceHeadAccess::Shared => "shared",
                    };
                    let owner = (binding.access == everruns_host::WorkspaceHeadAccess::Isolated)
                        .then(|| session_id.to_string());
                    let claim = transaction
                        .query_row(
                            "SELECT access, owner_session_id
                             FROM framework_workspace_head_claims
                             WHERE provider_id = ?1 AND workspace_id = ?2 AND head_id = ?3",
                            params![provider_id, workspace_id, head_id],
                            |row| Ok((row.get::<_, String>(0)?, row.get::<_, Option<String>>(1)?)),
                        )
                        .optional()?;
                    if let Some((recorded_access, recorded_owner)) = claim {
                        if recorded_access != access
                            || (access == "isolated"
                                && recorded_owner.as_deref() != owner.as_deref())
                        {
                            return Err(rusqlite::Error::InvalidParameterName(
                                "workspace head claim conflict".into(),
                            ));
                        }
                    } else {
                        transaction.execute(
                            "INSERT INTO framework_workspace_head_claims
                             (provider_id, workspace_id, head_id, access, owner_session_id)
                             VALUES (?1, ?2, ?3, ?4, ?5)",
                            params![provider_id, workspace_id, head_id, access, owner],
                        )?;
                    }
                    transaction.execute(
                        "INSERT INTO framework_session_environments (session_id, binding_json)
                         VALUES (?1, ?2)",
                        params![session_id.to_string(), encoded],
                    )?;
                }
                transaction.commit()
            })
            .map_err(|error| {
                if error.to_string().contains("environment binding conflict")
                    || error.to_string().contains("workspace head claim conflict")
                    || error.to_string().contains("UNIQUE constraint failed")
                {
                    EnvironmentBindingError::Conflict
                } else {
                    EnvironmentBindingError::Unavailable
                }
            })
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
    use everruns_core::execution_loading::SessionStore;
    use everruns_host::{
        EnvironmentBindingStore, RuntimeSessionStore, WorkspaceHeadAccess, WorkspaceHeadId,
        WorkspaceProviderId,
    };
    use everruns_provider::typed_id::WorkspaceId;

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

    #[tokio::test]
    async fn opaque_binding_survives_reopen_and_enforces_isolated_ownership() {
        let root = tempfile::tempdir().unwrap();
        let path = root.path().join("local.db");
        let first = SessionId::new();
        let second = SessionId::new();
        let binding = WorkspaceBinding {
            provider_id: WorkspaceProviderId::new("test.workspace").unwrap(),
            workspace_id: WorkspaceId::from_seed(41),
            head_id: WorkspaceHeadId::new(),
            access: WorkspaceHeadAccess::Isolated,
            payload: b"opaque-v1".to_vec(),
        };

        let store = LocalSessionStore::new(SqliteDb::open(&path).unwrap()).unwrap();
        store.bind(first, &binding).await.unwrap();
        assert_eq!(
            store.bind(second, &binding).await,
            Err(EnvironmentBindingError::Conflict)
        );
        drop(store);

        let reopened = LocalSessionStore::new(SqliteDb::open(&path).unwrap()).unwrap();
        assert_eq!(
            EnvironmentBindingStore::load(&reopened, first)
                .await
                .unwrap(),
            Some(binding)
        );
    }

    #[tokio::test]
    async fn explicitly_shared_binding_accepts_multiple_sessions() {
        let store = LocalSessionStore::new(SqliteDb::open_in_memory().unwrap()).unwrap();
        let binding = WorkspaceBinding {
            provider_id: WorkspaceProviderId::new("test.workspace").unwrap(),
            workspace_id: WorkspaceId::from_seed(42),
            head_id: WorkspaceHeadId::new(),
            access: WorkspaceHeadAccess::Shared,
            payload: b"opaque-v1".to_vec(),
        };
        store.bind(SessionId::new(), &binding).await.unwrap();
        store.bind(SessionId::new(), &binding).await.unwrap();
    }

    #[tokio::test]
    async fn a_head_cannot_switch_between_isolated_and_shared_claims() {
        for initial_access in [WorkspaceHeadAccess::Isolated, WorkspaceHeadAccess::Shared] {
            let store = LocalSessionStore::new(SqliteDb::open_in_memory().unwrap()).unwrap();
            let mut binding = WorkspaceBinding {
                provider_id: WorkspaceProviderId::new("test.workspace").unwrap(),
                workspace_id: WorkspaceId::from_seed(43),
                head_id: WorkspaceHeadId::new(),
                access: initial_access,
                payload: b"opaque-v1".to_vec(),
            };
            store.bind(SessionId::new(), &binding).await.unwrap();
            binding.access = match initial_access {
                WorkspaceHeadAccess::Isolated => WorkspaceHeadAccess::Shared,
                WorkspaceHeadAccess::Shared => WorkspaceHeadAccess::Isolated,
            };
            assert_eq!(
                store.bind(SessionId::new(), &binding).await,
                Err(EnvironmentBindingError::Conflict)
            );
        }
    }
}
