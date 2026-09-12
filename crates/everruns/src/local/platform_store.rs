// Local PlatformStore.
//
// Scope (EVE-594): implement the subagent-critical core honestly, backed by a
// caller-supplied `LocalSessionRunner` (the embedder wires this to its
// `InProcessRuntime` so `create_session_with_options` / `send_message` /
// `wait_for_idle` actually create and drive real local sessions, and
// `get_messages` / `get_session_by_id` read real local state).
//
// The platform-management-only operations this file used to reject one by one
// were removed from `PlatformStore` itself with the `platform_management`
// retirement (EVE-953), so there is nothing left to stub out. `get_harness`
// stays an explicit unsupported error: it is still on the trait for hosted
// delegation, and a local embedder has no stored harness catalog to answer
// from.

use async_trait::async_trait;
use everruns_core::session::ExecutionSession;
use everruns_platform::Agent;
use everruns_platform::Harness;
use everruns_platform::{PlatformCreateSessionRequest, PlatformMessage, PlatformStore};
use everruns_platform::{Session, SessionParticipant};
use everruns_provider::error::{AgentLoopError, Result};
use everruns_provider::typed_id::PrincipalId;
use everruns_provider::typed_id::{AgentId, HarnessId, SessionId};
use std::sync::Arc;

/// Drives real local sessions for the platform store. An embedder implements
/// this over its `InProcessRuntime` (or a thin wrapper) so subagent spawning
/// can create child sessions, run turns, and read back session state. This is
/// the seam that keeps the platform store honest without it owning the runtime.
#[async_trait]
pub trait LocalSessionRunner: Send + Sync {
    /// Sessions this host can currently route messages to.
    ///
    /// `None` means every session in the organization is routable. Embedded
    /// hosts with a narrower or dynamic route set must return `Some`, including
    /// an empty vector when no session is active, so schedule claims stay scoped
    /// to work the host can deliver.
    async fn routable_session_ids(&self) -> Result<Option<Vec<SessionId>>> {
        Ok(None)
    }

    /// Create a child/local session and persist it in the session catalog.
    ///
    /// Returns the portable execution view (EVE-882): embedded runners carry
    /// no stored Session persistence records.
    async fn create_session(
        &self,
        harness_id: HarnessId,
        agent_id: Option<AgentId>,
        title: Option<&str>,
        locale: Option<&str>,
        parent_session_id: Option<SessionId>,
    ) -> Result<ExecutionSession>;

    /// Create a session from the portable request when it uses local-supported
    /// fresh-session semantics.
    async fn create_session_with_options(
        &self,
        request: PlatformCreateSessionRequest,
    ) -> Result<ExecutionSession> {
        if request.forked_from_session_id.is_some()
            || request.budget_root_session_id.is_some()
            || request.seed != everruns_core::session::SessionSeedMode::Fresh
        {
            return Err(unsupported("create_session(seed)"));
        }
        let mut session = self
            .create_session(
                request.harness_id,
                request.agent_id,
                request.title.as_deref(),
                request.locale.as_deref(),
                request.parent_session_id,
            )
            .await?;
        session.goal = request.goal;
        Ok(session)
    }

    /// Deliver a user message and run a turn to completion.
    async fn send_message(&self, session_id: SessionId, content: &str) -> Result<()>;

    /// List sessions known to the runner. Optionally filtered by agent.
    async fn list_sessions(
        &self,
        limit: Option<usize>,
        agent_id: Option<AgentId>,
    ) -> Result<Vec<ExecutionSession>>;

    /// Look up a single session by id.
    async fn get_session(&self, session_id: SessionId) -> Result<Option<ExecutionSession>>;

    /// Read recent messages (most recent first) as platform messages.
    async fn get_messages(
        &self,
        session_id: SessionId,
        limit: Option<usize>,
    ) -> Result<Vec<PlatformMessage>>;

    /// Current session status string, or `None` if the session is unknown.
    async fn get_session_status(&self, session_id: SessionId) -> Result<Option<String>>;
}

fn unsupported(op: &str) -> AgentLoopError {
    AgentLoopError::tool(format!(
        "operation '{op}' is not supported by the local platform store; \
         manage this entity in embedder code"
    ))
}

/// Local platform store for one (org, session) scope, backed by a
/// [`LocalSessionRunner`].
#[derive(Clone)]
pub struct LocalPlatformStore {
    runner: Arc<dyn LocalSessionRunner>,
}

impl LocalPlatformStore {
    /// Bind a platform-store adapter to an embedder's session runner.
    ///
    /// No longer takes a base URL: it existed only to build UI links for the
    /// retired `platform_management` tool results (EVE-953).
    pub fn new(runner: Arc<dyn LocalSessionRunner>) -> Self {
        Self { runner }
    }

    /// Lift the runner's portable execution view into the stored platform
    /// record the `PlatformStore` seam speaks (EVE-882). The local host has no
    /// real session ownership catalog, so the record carries the fixed local
    /// principal and neutral product defaults.
    fn lift(&self, session: ExecutionSession) -> Session {
        Session::from_execution_session(session, PrincipalId::from_seed(1))
    }
}

#[async_trait]
impl PlatformStore for LocalPlatformStore {
    // ---- Honestly implemented: subagent-critical core -----------------------

    async fn create_session_with_options(
        &self,
        request: PlatformCreateSessionRequest,
    ) -> Result<Session> {
        if request.blueprint_id.is_some() {
            return Err(unsupported("create_session(blueprint)"));
        }
        Ok(self.lift(self.runner.create_session_with_options(request).await?))
    }

    async fn get_session_by_id(&self, id: SessionId) -> Result<Option<Session>> {
        Ok(self
            .runner
            .get_session(id)
            .await?
            .map(|session| self.lift(session)))
    }

    async fn add_agent_session_participant(
        &self,
        _session_id: SessionId,
        _agent_id: AgentId,
    ) -> Result<SessionParticipant> {
        Err(unsupported("add_agent_session_participant"))
    }

    async fn send_message(&self, session_id: SessionId, content: &str) -> Result<()> {
        self.runner.send_message(session_id, content).await
    }

    async fn get_messages(
        &self,
        session_id: SessionId,
        limit: Option<usize>,
    ) -> Result<Vec<PlatformMessage>> {
        self.runner.get_messages(session_id, limit).await
    }

    async fn wait_for_idle(
        &self,
        session_id: SessionId,
        _timeout_secs: Option<u64>,
    ) -> Result<String> {
        // `send_message` runs the turn synchronously to completion in the local
        // host, so by the time a caller polls, the session is already idle.
        self.runner
            .get_session_status(session_id)
            .await?
            .ok_or_else(|| AgentLoopError::session_not_found(session_id))
    }

    // ---- Platform-management-only: explicit unsupported ---------------------

    async fn get_harness(&self, _id: HarnessId) -> Result<Option<Harness>> {
        Err(unsupported("get_harness"))
    }
    async fn get_agent_by_id(&self, _id: AgentId) -> Result<Option<Agent>> {
        Err(unsupported("get_agent_by_id"))
    }
}
