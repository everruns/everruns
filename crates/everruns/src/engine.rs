//! Application-owned execution engines.

use std::collections::HashMap;
use std::fmt;
use std::sync::{Arc, Mutex, Weak};

use async_trait::async_trait;

use crate::{Agent, ResumeError, Session, SessionId};

/// Object-safe binding between a public [`Session`] and its owning engine.
///
/// The binding deliberately exposes only Framework identity. Backend, store,
/// host-runtime, and platform transport values remain private to each engine.
#[async_trait]
pub trait SessionExecution: Send + Sync + fmt::Debug {
    /// The stable Framework session identity owned by this binding.
    fn session_id(&self) -> SessionId;

    /// Return the immutable behavior snapshot selected by the engine.
    ///
    /// Engine implementations should resolve this from their own catalog; a
    /// Session does not store the Agent value itself.
    #[doc(hidden)]
    fn agent_snapshot(&self) -> Agent;

    /// Ensure this engine-owned identity is present in its session catalog.
    #[doc(hidden)]
    async fn ensure_cataloged(&self) -> Result<(), crate::HistoryError>;
}

/// Application-facing service-provider interface for session execution.
#[async_trait]
pub trait Engine: Send + Sync {
    /// Create a new engine-owned session from an immutable Agent snapshot.
    fn create(&self, agent: Agent) -> Session;

    /// Reopen a session already owned by this engine.
    async fn resume(&self, session_id: SessionId) -> Result<Session, ResumeError>;
}

/// Process-local engine for embedded applications and tests.
///
/// Clones share one catalog. Each catalog entry retains the immutable Agent
/// snapshot used at creation, so sessions remain runnable and resumable after
/// the caller drops its original Agent handle.
#[derive(Clone, Default)]
pub struct InMemoryEngine {
    inner: Arc<InMemoryEngineInner>,
}

#[derive(Default)]
struct InMemoryEngineInner {
    sessions: Mutex<HashMap<SessionId, EngineSessionEntry>>,
}

struct EngineSessionEntry {
    agent: Agent,
    state: Option<Weak<crate::session::SessionInner>>,
}

impl fmt::Debug for InMemoryEngine {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        let sessions = self
            .inner
            .sessions
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
            .len();
        formatter
            .debug_struct("InMemoryEngine")
            .field("sessions", &sessions)
            .finish()
    }
}

impl InMemoryEngine {
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

    /// Reopen a session already attached to this engine.
    pub async fn resume(&self, session_id: SessionId) -> Result<Session, ResumeError> {
        if let Some(state) = self.state(session_id) {
            return Ok(Session::from_inner(state));
        }
        let agent = self
            .agent(session_id)
            .ok_or(ResumeError::SessionNotFound { session_id })?;
        let environment = agent.reopen_session_environment(session_id).await?;
        let session = Session::new(self.binding(session_id), environment);
        self.remember_state(session_id, &session);
        Ok(session)
    }

    pub(crate) fn agent(&self, session_id: SessionId) -> Option<Agent> {
        self.inner
            .sessions
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
            .get(&session_id)
            .map(|entry| entry.agent.clone())
    }

    fn attach_unchecked(&self, session_id: SessionId, agent: Agent) {
        self.inner
            .sessions
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
            .insert(session_id, EngineSessionEntry { agent, state: None });
    }

    /// Attach a persisted local session to this process-local engine.
    ///
    /// Agent values may contain provider drivers, function handlers, and hook
    /// closures, so the local catalog deliberately never serializes them. After
    /// a process restart, rebuild the Agent from trusted application
    /// configuration, attach it to its persisted session id, then call
    /// [`Engine::resume`]. Same-process sessions created by this engine need no
    /// attachment. Attachment is idempotent: an existing engine binding keeps
    /// its original Agent snapshot.
    pub async fn attach(&self, session_id: SessionId, agent: Agent) -> Result<(), ResumeError> {
        if self.agent(session_id).is_some() {
            return Ok(());
        }
        let backends = agent
            .shared_backends()
            .await
            .map_err(|error| error.resume_error())?;
        let exists = backends
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
        Arc::new(InMemorySessionExecution {
            engine: self.clone(),
            session_id,
        })
    }
}

#[async_trait]
impl Engine for InMemoryEngine {
    fn create(&self, agent: Agent) -> Session {
        InMemoryEngine::create(self, agent)
    }

    async fn resume(&self, session_id: SessionId) -> Result<Session, ResumeError> {
        InMemoryEngine::resume(self, session_id).await
    }
}

struct InMemorySessionExecution {
    engine: InMemoryEngine,
    session_id: SessionId,
}

impl fmt::Debug for InMemorySessionExecution {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("SessionExecution")
            .field("session_id", &self.session_id)
            .finish_non_exhaustive()
    }
}

#[async_trait]
impl SessionExecution for InMemorySessionExecution {
    fn session_id(&self) -> SessionId {
        self.session_id
    }

    fn agent_snapshot(&self) -> Agent {
        self.engine
            .agent(self.session_id)
            .expect("engine binding references a cataloged session")
    }

    async fn ensure_cataloged(&self) -> Result<(), crate::HistoryError> {
        let agent =
            self.engine
                .agent(self.session_id)
                .ok_or(crate::HistoryError::SessionNotFound {
                    session_id: self.session_id,
                })?;
        agent.catalog_session(self.session_id).await
    }
}
