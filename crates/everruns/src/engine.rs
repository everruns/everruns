//! Application-owned execution engine.

use std::collections::HashMap;
use std::fmt;
use std::sync::{Arc, Mutex, Weak};

#[cfg(feature = "local")]
use std::sync::OnceLock;

use async_trait::async_trait;
use everruns_host::{
    Environment, EnvironmentBindingStore, HostBackends, InMemoryEnvironmentBindingStore,
};
use tokio::sync::OnceCell;

use crate::agent::BackendInitError;
use crate::{Agent, ResumeError, Session, SessionEnvironmentError, SessionId};

/// The runtime resources owned by an [`Engine`].
pub(crate) struct EngineBackends {
    pub(crate) host: HostBackends,
    binding_store: Arc<dyn EnvironmentBindingStore>,
}

/// Private binding between a [`Session`] and its owning engine.
#[async_trait]
pub(crate) trait SessionExecution: Send + Sync + fmt::Debug {
    fn session_id(&self) -> SessionId;
    fn agent_snapshot(&self) -> Agent;
    async fn backends(&self) -> Result<Arc<EngineBackends>, BackendInitError>;
    async fn ensure_cataloged(&self) -> Result<(), crate::HistoryError>;
    async fn bind_environment(
        &self,
        environment: &Environment,
    ) -> Result<(), SessionEnvironmentError>;
    async fn bind_default_environment(&self) -> Result<Environment, SessionEnvironmentError>;
    async fn reopen_environment(&self) -> Result<Option<Environment>, ResumeError>;
}

/// Application-owned session execution engine.
///
/// Clones share a session catalog and backend bundle. Local profiles also share
/// one backend bundle across engines in the same process, preventing divergent
/// JSONL indexes and SQLite handles when an application constructs more than
/// one engine for the same profile.
#[derive(Clone, Default)]
pub struct Engine {
    inner: Arc<EngineInner>,
}

#[derive(Default)]
struct EngineInner {
    sessions: Mutex<HashMap<SessionId, EngineSessionEntry>>,
    memory_backends: Arc<OnceCell<Arc<EngineBackends>>>,
    #[cfg(feature = "local")]
    local_backends: Mutex<HashMap<std::path::PathBuf, Arc<OnceCell<Arc<EngineBackends>>>>>,
}

struct EngineSessionEntry {
    agent: Agent,
    state: Option<Weak<crate::session::SessionInner>>,
}

#[cfg(feature = "local")]
struct LocalBackendCell {
    workspace_root: std::path::PathBuf,
    cell: Weak<OnceCell<Arc<EngineBackends>>>,
}

#[cfg(feature = "local")]
fn local_backend_cells() -> &'static Mutex<HashMap<std::path::PathBuf, LocalBackendCell>> {
    static CELLS: OnceLock<Mutex<HashMap<std::path::PathBuf, LocalBackendCell>>> = OnceLock::new();
    CELLS.get_or_init(|| Mutex::new(HashMap::new()))
}

impl fmt::Debug for Engine {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        let sessions = self
            .inner
            .sessions
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
            .len();
        formatter
            .debug_struct("Engine")
            .field("sessions", &sessions)
            .finish()
    }
}

impl Engine {
    /// Construct an empty process-local engine.
    pub fn new() -> Self {
        Self::default()
    }

    /// Create a new engine-owned session from an immutable Agent snapshot.
    pub fn create(&self, agent: Agent) -> Session {
        let session_id = SessionId::new();
        self.attach_unchecked(session_id, agent);
        let session = Session::new(self.binding(session_id), None);
        self.remember_state(session_id, &session);
        session
    }

    /// Reopen a session already owned by this engine.
    pub async fn resume(&self, session_id: SessionId) -> Result<Session, ResumeError> {
        if let Some(state) = self.state(session_id) {
            return Ok(Session::from_inner(state));
        }
        if self.agent(session_id).is_none() {
            return Err(ResumeError::SessionNotFound { session_id });
        }
        let binding = self.binding(session_id);
        let environment = binding.reopen_environment().await?;
        let session = Session::new(binding, environment);
        self.remember_state(session_id, &session);
        Ok(session)
    }

    /// Attach a persisted local session to this engine.
    ///
    /// Rebuild the Agent from trusted application configuration after a process
    /// restart, attach it to the persisted id, then call [`resume`](Self::resume).
    pub async fn attach(&self, session_id: SessionId, agent: Agent) -> Result<(), ResumeError> {
        if self.agent(session_id).is_some() {
            return Ok(());
        }
        let backends = self
            .backends_for(&agent)
            .await
            .map_err(|error| error.resume_error())?;
        let exists = backends
            .host
            .session_store
            .get_session(session_id)
            .await
            .map_err(|_| ResumeError::Unavailable)?
            .is_some();
        if !exists {
            return Err(ResumeError::SessionNotFound { session_id });
        }
        self.inner
            .sessions
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
            .entry(session_id)
            .or_insert(EngineSessionEntry { agent, state: None });
        Ok(())
    }

    pub(crate) fn agent(&self, session_id: SessionId) -> Option<Agent> {
        self.inner
            .sessions
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
            .get(&session_id)
            .map(|entry| entry.agent.clone())
    }

    async fn backends_for(&self, agent: &Agent) -> Result<Arc<EngineBackends>, BackendInitError> {
        let cell = self.backend_cell(agent)?;
        cell.get_or_try_init(|| initialize_backends(agent))
            .await
            .cloned()
    }

    fn backend_cell(
        &self,
        agent: &Agent,
    ) -> Result<Arc<OnceCell<Arc<EngineBackends>>>, BackendInitError> {
        #[cfg(feature = "local")]
        if let Some(config) = agent.local_config() {
            let key = absolute_path(config.data_dir());
            let workspace_root = absolute_path(config.workspace_root());
            let mut cells = local_backend_cells()
                .lock()
                .unwrap_or_else(|poisoned| poisoned.into_inner());
            if let Some(existing) = cells.get(&key)
                && let Some(cell) = existing.cell.upgrade()
            {
                if existing.workspace_root != workspace_root {
                    return Err(BackendInitError::Host(
                        everruns_provider::error::AgentLoopError::config(format!(
                            "local profile {} is already open with workspace {}; requested {}",
                            key.display(),
                            existing.workspace_root.display(),
                            workspace_root.display()
                        )),
                    ));
                }
                self.inner
                    .local_backends
                    .lock()
                    .unwrap_or_else(|poisoned| poisoned.into_inner())
                    .insert(key, cell.clone());
                return Ok(cell);
            }
            let cell = Arc::new(OnceCell::new());
            cells.insert(
                key.clone(),
                LocalBackendCell {
                    workspace_root,
                    cell: Arc::downgrade(&cell),
                },
            );
            self.inner
                .local_backends
                .lock()
                .unwrap_or_else(|poisoned| poisoned.into_inner())
                .insert(key, cell.clone());
            return Ok(cell);
        }
        #[cfg(not(feature = "local"))]
        let _ = agent;
        Ok(self.inner.memory_backends.clone())
    }

    fn attach_unchecked(&self, session_id: SessionId, agent: Agent) {
        self.inner
            .sessions
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
            .insert(session_id, EngineSessionEntry { agent, state: None });
    }

    fn state(&self, session_id: SessionId) -> Option<Arc<crate::session::SessionInner>> {
        self.inner
            .sessions
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
            .get(&session_id)
            .and_then(|entry| entry.state.as_ref()?.upgrade())
    }

    fn remember_state(&self, session_id: SessionId, session: &Session) {
        if let Some(entry) = self
            .inner
            .sessions
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
            .get_mut(&session_id)
        {
            entry.state = Some(Arc::downgrade(&session.inner()));
        }
    }

    fn binding(&self, session_id: SessionId) -> Arc<dyn SessionExecution> {
        Arc::new(EngineSessionExecution {
            engine: self.clone(),
            session_id,
        })
    }
}

/// Compatibility name for the application-owned process-local engine.
pub type InMemoryEngine = Engine;

#[cfg(feature = "local")]
fn absolute_path(path: &std::path::Path) -> std::path::PathBuf {
    let absolute = if path.is_absolute() {
        path.to_path_buf()
    } else {
        std::env::current_dir()
            .unwrap_or_else(|_| std::path::PathBuf::from("."))
            .join(path)
    };
    let mut normalized = std::path::PathBuf::new();
    for component in absolute.components() {
        match component {
            std::path::Component::CurDir => {}
            std::path::Component::ParentDir => {
                normalized.pop();
            }
            other => normalized.push(other.as_os_str()),
        }
    }
    // Canonicalize the longest existing prefix so a profile path keeps the
    // same identity before and after its workspace directory is created. This
    // also normalizes platform aliases such as macOS `/var` and `/private/var`.
    let mut existing = normalized.as_path();
    let mut missing = Vec::new();
    loop {
        if let Ok(mut canonical) = std::fs::canonicalize(existing) {
            for component in missing.iter().rev() {
                canonical.push(component);
            }
            return canonical;
        }
        let Some(name) = existing.file_name() else {
            return normalized;
        };
        missing.push(name.to_os_string());
        let Some(parent) = existing.parent() else {
            return normalized;
        };
        existing = parent;
    }
}

async fn initialize_backends(agent: &Agent) -> Result<Arc<EngineBackends>, BackendInitError> {
    let backends = HostBackends::in_memory();
    #[cfg(feature = "local")]
    if let Some(config) = agent.local_config() {
        let profile = config.profile();
        profile.ensure_dirs().map_err(|error| {
            BackendInitError::Host(everruns_provider::error::AgentLoopError::config(
                error.to_string(),
            ))
        })?;
        let event_log = Arc::new(
            everruns_host::JsonlEventLog::open(config.data_dir().join("events.jsonl"))
                .await
                .map_err(BackendInitError::Event)?,
        );
        let local = crate::local::LocalBackends::new(profile, backends.with_event_log(event_log))
            .map_err(BackendInitError::Host)?;
        let session_store = Arc::new(
            crate::local::LocalSessionStore::new(local.db.clone())
                .map_err(BackendInitError::Host)?,
        );
        return Ok(Arc::new(EngineBackends {
            host: local
                .runtime_backends
                .with_session_store(session_store.clone()),
            binding_store: session_store,
        }));
    }
    #[cfg(not(feature = "local"))]
    let _ = agent;
    Ok(Arc::new(EngineBackends {
        host: backends,
        binding_store: Arc::new(InMemoryEnvironmentBindingStore::default()),
    }))
}

struct EngineSessionExecution {
    engine: Engine,
    session_id: SessionId,
}

impl fmt::Debug for EngineSessionExecution {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("SessionExecution")
            .field("session_id", &self.session_id)
            .finish_non_exhaustive()
    }
}

#[async_trait]
impl SessionExecution for EngineSessionExecution {
    fn session_id(&self) -> SessionId {
        self.session_id
    }

    fn agent_snapshot(&self) -> Agent {
        self.engine
            .agent(self.session_id)
            .expect("engine binding references a cataloged session")
    }

    async fn backends(&self) -> Result<Arc<EngineBackends>, BackendInitError> {
        self.engine.backends_for(&self.agent_snapshot()).await
    }

    async fn ensure_cataloged(&self) -> Result<(), crate::HistoryError> {
        let agent =
            self.engine
                .agent(self.session_id)
                .ok_or(crate::HistoryError::SessionNotFound {
                    session_id: self.session_id,
                })?;
        let backends = self
            .engine
            .backends_for(&agent)
            .await
            .map_err(|error| error.history_error())?;
        agent.catalog_session(&backends.host, self.session_id).await
    }

    async fn bind_environment(
        &self,
        environment: &Environment,
    ) -> Result<(), SessionEnvironmentError> {
        let agent = self.agent_snapshot();
        let backends = self
            .backends()
            .await
            .map_err(|_| SessionEnvironmentError::Unavailable)?;
        agent
            .bind_session_environment(
                backends.binding_store.as_ref(),
                self.session_id,
                environment,
            )
            .await
    }

    async fn bind_default_environment(&self) -> Result<Environment, SessionEnvironmentError> {
        let agent = self.agent_snapshot();
        let backends = self
            .backends()
            .await
            .map_err(|_| SessionEnvironmentError::Unavailable)?;
        agent
            .bind_default_session_environment(backends.binding_store.as_ref(), self.session_id)
            .await
    }

    async fn reopen_environment(&self) -> Result<Option<Environment>, ResumeError> {
        let agent = self.agent_snapshot();
        let backends = self
            .backends()
            .await
            .map_err(|error| error.resume_error())?;
        agent
            .reopen_session_environment(backends.binding_store.as_ref(), self.session_id)
            .await
    }
}
