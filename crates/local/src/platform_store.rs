// Local PlatformStore.
//
// Scope (EVE-594): implement the subagent-critical core honestly, backed by a
// caller-supplied `LocalSessionRunner` (the embedder wires this to its
// `InProcessRuntime` so `create_session` / `send_message` / `wait_for_idle`
// actually create and drive real local sessions, and `get_messages` /
// `get_session_by_id` / `list_sessions` read real local state). The
// platform-management-only operations (harness/agent/app CRUD, publish,
// channels, capability listing, delete/context-report) return an explicit
// unsupported error rather than half-implementing them — local embedded hosts
// manage those entities in code, not through this tool surface.

use async_trait::async_trait;
use everruns_core::capability_dto::CapabilityInfo;
use everruns_core::error::{AgentLoopError, Result};
use everruns_core::session::ExecutionSession;
use everruns_core::typed_id::PrincipalId;
use everruns_core::typed_id::{
    AgentId, AgentIdentityId, AppChannelId, AppId, HarnessId, SessionId,
};
use everruns_platform::Agent;
use everruns_platform::Harness;
use everruns_platform::app::{App, AppChannel, ChannelType};
use everruns_platform::{PlatformCreateSessionRequest, PlatformMessage, PlatformStore};
use everruns_platform::{Session, SessionParticipant};
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
    base_url: String,
}

impl LocalPlatformStore {
    pub fn new(runner: Arc<dyn LocalSessionRunner>, base_url: impl Into<String>) -> Self {
        Self {
            runner,
            base_url: base_url.into(),
        }
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

    async fn create_session(
        &self,
        harness_id: HarnessId,
        agent_id: Option<AgentId>,
        title: Option<&str>,
        locale: Option<&str>,
        blueprint_id: Option<&str>,
        _blueprint_config: Option<&serde_json::Value>,
        parent_session_id: Option<SessionId>,
    ) -> Result<Session> {
        if blueprint_id.is_some() {
            return Err(unsupported("create_session(blueprint)"));
        }
        Ok(self.lift(
            self.runner
                .create_session(harness_id, agent_id, title, locale, parent_session_id)
                .await?,
        ))
    }

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

    async fn list_sessions(
        &self,
        limit: Option<usize>,
        agent_id: Option<AgentId>,
    ) -> Result<Vec<Session>> {
        Ok(self
            .runner
            .list_sessions(limit, agent_id)
            .await?
            .into_iter()
            .map(|session| self.lift(session))
            .collect())
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

    fn base_url(&self) -> &str {
        &self.base_url
    }

    // ---- Platform-management-only: explicit unsupported ---------------------

    async fn list_harnesses(&self) -> Result<Vec<Harness>> {
        Err(unsupported("list_harnesses"))
    }
    async fn get_harness(&self, _id: HarnessId) -> Result<Option<Harness>> {
        Err(unsupported("get_harness"))
    }
    async fn create_harness(
        &self,
        _name: &str,
        _display_name: Option<&str>,
        _description: Option<&str>,
        _system_prompt: Option<&str>,
        _parent_harness_id: Option<HarnessId>,
        _capabilities: &[String],
    ) -> Result<Harness> {
        Err(unsupported("create_harness"))
    }
    async fn update_harness(
        &self,
        _id: HarnessId,
        _name: Option<&str>,
        _display_name: Option<&str>,
        _description: Option<&str>,
        _system_prompt: Option<&str>,
        _parent_harness_id: Option<Option<HarnessId>>,
    ) -> Result<Harness> {
        Err(unsupported("update_harness"))
    }
    async fn delete_harness(&self, _id: HarnessId) -> Result<()> {
        Err(unsupported("delete_harness"))
    }
    async fn copy_harness(&self, _id: HarnessId, _new_name: Option<&str>) -> Result<Harness> {
        Err(unsupported("copy_harness"))
    }
    async fn list_agents(&self) -> Result<Vec<Agent>> {
        Err(unsupported("list_agents"))
    }
    async fn get_agent_by_id(&self, _id: AgentId) -> Result<Option<Agent>> {
        Err(unsupported("get_agent_by_id"))
    }
    async fn create_agent(
        &self,
        _name: &str,
        _display_name: Option<&str>,
        _description: Option<&str>,
        _system_prompt: &str,
        _capabilities: &[String],
    ) -> Result<Agent> {
        Err(unsupported("create_agent"))
    }
    async fn update_agent(
        &self,
        _id: AgentId,
        _name: Option<&str>,
        _display_name: Option<&str>,
        _description: Option<&str>,
        _system_prompt: Option<&str>,
    ) -> Result<Agent> {
        Err(unsupported("update_agent"))
    }
    async fn delete_agent(&self, _id: AgentId) -> Result<()> {
        Err(unsupported("delete_agent"))
    }
    async fn list_apps(&self, _search: Option<&str>, _include_archived: bool) -> Result<Vec<App>> {
        Err(unsupported("list_apps"))
    }
    async fn get_app(&self, _id: AppId) -> Result<Option<App>> {
        Err(unsupported("get_app"))
    }
    async fn create_app(
        &self,
        _name: &str,
        _description: Option<&str>,
        _harness_id: HarnessId,
        _agent_id: Option<AgentId>,
        _agent_identity_id: Option<AgentIdentityId>,
        _channel_type: Option<ChannelType>,
        _channel_config: Option<&serde_json::Value>,
    ) -> Result<App> {
        Err(unsupported("create_app"))
    }
    async fn update_app(
        &self,
        _id: AppId,
        _name: Option<&str>,
        _description: Option<&str>,
        _harness_id: Option<HarnessId>,
        _agent_id: Option<AgentId>,
        _agent_identity_id: Option<Option<AgentIdentityId>>,
    ) -> Result<App> {
        Err(unsupported("update_app"))
    }
    async fn delete_app(&self, _id: AppId) -> Result<()> {
        Err(unsupported("delete_app"))
    }
    async fn destroy_app(&self, _id: AppId) -> Result<()> {
        Err(unsupported("destroy_app"))
    }
    async fn publish_app(&self, _id: AppId) -> Result<App> {
        Err(unsupported("publish_app"))
    }
    async fn unpublish_app(&self, _id: AppId) -> Result<App> {
        Err(unsupported("unpublish_app"))
    }
    async fn add_app_channel(
        &self,
        _app_id: AppId,
        _channel_type: ChannelType,
        _channel_config: Option<&serde_json::Value>,
        _enabled: Option<bool>,
    ) -> Result<AppChannel> {
        Err(unsupported("add_app_channel"))
    }
    async fn update_app_channel(
        &self,
        _app_id: AppId,
        _channel_id: AppChannelId,
        _channel_type: Option<ChannelType>,
        _channel_config: Option<&serde_json::Value>,
        _enabled: Option<bool>,
    ) -> Result<AppChannel> {
        Err(unsupported("update_app_channel"))
    }
    async fn delete_app_channel(&self, _app_id: AppId, _channel_id: AppChannelId) -> Result<()> {
        Err(unsupported("delete_app_channel"))
    }
    async fn get_session_context_report(
        &self,
        _id: SessionId,
    ) -> Result<everruns_core::SessionContextReport> {
        Err(unsupported("get_session_context_report"))
    }
    async fn delete_session(&self, _id: SessionId) -> Result<()> {
        Err(unsupported("delete_session"))
    }
    async fn list_capabilities(&self, _search: Option<&str>) -> Result<Vec<CapabilityInfo>> {
        Err(unsupported("list_capabilities"))
    }
}
