//! Application-owned execution engines.

use std::collections::HashMap;
use std::fmt;
use std::sync::{Arc, Mutex, Weak};

use async_trait::async_trait;

use crate::{Agent, ResumeError, Session, SessionId};

/// Why an engine rejected an agent before it created any session state.
///
/// Distributed engines use this typed boundary to fail closed when a facade
/// [`Agent`] contains process-local code. The component and registration path
/// fields are intentionally stable so applications can present actionable
/// migration errors without parsing display text.
#[derive(Clone, Debug, PartialEq, Eq, thiserror::Error)]
#[non_exhaustive]
pub enum EngineCreateError {
    /// This engine accepts a different, explicitly portable submission type.
    #[error("{engine} rejected non-portable {component}; submit through {registration_path}")]
    PortableAgentRequired {
        /// Engine implementation that rejected the value.
        engine: &'static str,
        /// Component that cannot cross the persistence boundary.
        component: &'static str,
        /// Accepted registration/submission path.
        registration_path: &'static str,
    },
    /// One component was not represented by a stable registry reference.
    #[error("non-portable {component} at registration path {registration_path}: {reason}")]
    NonPortableComponent {
        /// Human-readable component kind and identity.
        component: String,
        /// Builder or registry path that must be made portable.
        registration_path: String,
        /// Concise rejection reason.
        reason: String,
    },
    /// The engine could not create its session state.
    #[error("engine session persistence is unavailable: {message}")]
    Unavailable {
        /// Backend-safe failure summary.
        message: String,
    },
}

/// Object-safe binding between a public [`Session`] and its owning engine.
///
/// The binding deliberately exposes only Framework identity. Backend, store,
/// host-runtime, and platform transport values remain private to each engine.
pub trait SessionExecution: Send + Sync + fmt::Debug {
    /// The stable Framework session identity owned by this binding.
    fn session_id(&self) -> SessionId;

    /// Return the immutable behavior snapshot selected by the engine.
    ///
    /// Engine implementations should resolve this from their own catalog; a
    /// Session does not store the Agent value itself.
    #[doc(hidden)]
    fn agent_snapshot(&self) -> Agent;
}

/// Application-facing service-provider interface for session execution.
#[async_trait]
pub trait Engine: Send + Sync {
    /// Validate and create a new engine-owned session.
    ///
    /// The default keeps existing process-local engines source compatible.
    /// Scale implementations override this method to reject local code before
    /// any session or workflow is submitted.
    fn try_create(&self, agent: Agent) -> Result<Session, EngineCreateError> {
        Ok(self.create(agent))
    }

    /// Create a new engine-owned session from an immutable Agent snapshot.
    ///
    /// Prefer [`Engine::try_create`] when the selected engine may impose a
    /// portability boundary. This convenience method remains infallible for
    /// embedded engines.
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

    /// Reattach a known session identity to an immutable agent snapshot.
    ///
    /// This is a host-integration seam for engines that persist their own
    /// catalog and reconstruct the process-local runtime after restart. It
    /// does not restore events or environment bindings by itself; those remain
    /// the responsibility of the Agent's configured backends.
    #[doc(hidden)]
    pub fn restore(&self, session_id: SessionId, agent: Agent) -> Session {
        agent.remember_session(session_id);
        self.attach(session_id, agent);
        let session = Session::new(self.binding(session_id), None);
        self.remember_state(session_id, &session);
        session
    }

    pub(crate) fn agent(&self, session_id: SessionId) -> Option<Agent> {
        self.inner
            .sessions
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
            .get(&session_id)
            .map(|entry| entry.agent.clone())
    }

    pub(crate) fn attach(&self, session_id: SessionId, agent: Agent) {
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
        Arc::new(InMemorySessionExecution {
            engine: self.clone(),
            session_id,
        })
    }
}

#[async_trait]
impl Engine for InMemoryEngine {
    fn create(&self, agent: Agent) -> Session {
        let session_id = SessionId::new();
        agent.remember_session(session_id);
        self.attach(session_id, agent);
        let session = Session::new(self.binding(session_id), None);
        self.remember_state(session_id, &session);
        session
    }

    async fn resume(&self, session_id: SessionId) -> Result<Session, ResumeError> {
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

impl SessionExecution for InMemorySessionExecution {
    fn session_id(&self) -> SessionId {
        self.session_id
    }

    fn agent_snapshot(&self) -> Agent {
        self.engine
            .agent(self.session_id)
            .expect("engine binding references a cataloged session")
    }
}
