// Platform Store trait for org-scoped management operations
//
// Decision: Single trait covers harness, agent, session CRUD + messaging
// Decision: Trait lives in core; implementations in server/worker crates
// Decision: Tool results include UI links via base_url()
// Decision: PlatformMessage is a simplified view (role + text + timestamp)

use crate::agent::Agent;
use crate::app::{App, AppChannel, ChannelType};
use crate::harness::{Harness, resolve_execution_harness};
use async_trait::async_trait;
use everruns_core::SessionContextReport;
use everruns_core::capability_dto::CapabilityInfo;
use everruns_core::error::Result;
use everruns_core::session::SessionSeedMode;
use everruns_core::typed_id::SessionParticipantId;

use crate::session::{Session, SessionParticipant};
use everruns_core::typed_id::{
    AgentId, AgentIdentityId, AppChannelId, AppId, HarnessId, SessionId,
};
use std::collections::HashSet;

// The narrow session-delegation DTOs live in `everruns-core` (they are part of
// the `SubagentSessionDelegate` contract portable code depends on); the full
// `PlatformStore` reuses them. See EVE-839.
pub use everruns_core::subagent_delegation::{PlatformCreateSessionRequest, PlatformMessage};

/// Trait for platform-level management operations.
///
/// Provides the catalog-backed `platform` surface plus legacy org-scoped CRUD
/// used by `platform_management` compatibility tools.
#[async_trait]
pub trait PlatformStore: Send + Sync {
    // =========================================================================
    // Catalog-backed command surface
    // =========================================================================

    /// Search the authoritative domain-command catalog.
    async fn platform_discover(&self, _arguments: serde_json::Value) -> Result<String> {
        Err(everruns_core::error::AgentLoopError::config(
            "Platform command surface is not available in this host",
        ))
    }

    /// Execute a bounded script against read-only domain commands.
    async fn platform_query(&self, _arguments: serde_json::Value) -> Result<String> {
        Err(everruns_core::error::AgentLoopError::config(
            "Platform command surface is not available in this host",
        ))
    }

    /// Execute a bounded script against the full authorized command catalog.
    async fn platform_execute(&self, _arguments: serde_json::Value) -> Result<String> {
        Err(everruns_core::error::AgentLoopError::config(
            "Platform command surface is not available in this host",
        ))
    }

    // =========================================================================
    // Harness Operations
    // =========================================================================

    /// List all harnesses in the organization.
    async fn list_harnesses(&self) -> Result<Vec<Harness>>;

    /// Get a harness by ID.
    async fn get_harness(&self, id: HarnessId) -> Result<Option<Harness>>;

    /// Get the effective harness chain from root to the requested harness.
    ///
    /// Platform management lookups return the raw harness row. Runtime assembly
    /// applies inherited parent harnesses first, so security-sensitive
    /// compatibility checks must use this chain rather than a single raw row.
    async fn get_harness_chain(&self, id: HarnessId) -> Result<Vec<Harness>> {
        let mut chain = Vec::new();
        let mut current_id = Some(id);
        let mut seen = HashSet::new();

        while let Some(harness_id) = current_id {
            if !seen.insert(harness_id) {
                return Err(everruns_core::error::AgentLoopError::tool(format!(
                    "Harness inheritance cycle detected at {harness_id}"
                )));
            }
            let Some(harness) = self.get_harness(harness_id).await? else {
                return Ok(Vec::new());
            };
            current_id = harness.parent_harness_id;
            chain.push(harness);
        }

        chain.reverse();
        Ok(chain)
    }

    /// Create a new harness.
    ///
    /// `system_prompt` is optional: `None` creates a harness with no base
    /// prompt of its own (it inherits from the parent harness, if any, and
    /// composes with agent/session/capability layers at runtime).
    async fn create_harness(
        &self,
        name: &str,
        display_name: Option<&str>,
        description: Option<&str>,
        system_prompt: Option<&str>,
        parent_harness_id: Option<HarnessId>,
        capabilities: &[String],
    ) -> Result<Harness>;

    /// Update a harness (only provided fields are changed).
    async fn update_harness(
        &self,
        id: HarnessId,
        name: Option<&str>,
        display_name: Option<&str>,
        description: Option<&str>,
        system_prompt: Option<&str>,
        parent_harness_id: Option<Option<HarnessId>>,
    ) -> Result<Harness>;

    /// Delete (archive) a harness.
    async fn delete_harness(&self, id: HarnessId) -> Result<()>;

    /// Copy a harness, optionally with a new name.
    async fn copy_harness(&self, id: HarnessId, new_name: Option<&str>) -> Result<Harness>;

    // =========================================================================
    // Agent Operations
    // =========================================================================

    /// List all agents in the organization.
    async fn list_agents(&self) -> Result<Vec<Agent>>;

    /// Get an agent by public ID.
    async fn get_agent_by_id(&self, id: AgentId) -> Result<Option<Agent>>;

    /// Create a new agent.
    async fn create_agent(
        &self,
        name: &str,
        display_name: Option<&str>,
        description: Option<&str>,
        system_prompt: &str,
        capabilities: &[String],
    ) -> Result<Agent>;

    /// Update an agent (only provided fields are changed).
    async fn update_agent(
        &self,
        id: AgentId,
        name: Option<&str>,
        display_name: Option<&str>,
        description: Option<&str>,
        system_prompt: Option<&str>,
    ) -> Result<Agent>;

    /// Delete (archive) an agent.
    async fn delete_agent(&self, id: AgentId) -> Result<()>;

    // =========================================================================
    // App Operations
    // =========================================================================

    /// List all apps in the organization.
    async fn list_apps(&self, search: Option<&str>, include_archived: bool) -> Result<Vec<App>>;

    /// Get an app by ID.
    async fn get_app(&self, id: AppId) -> Result<Option<App>>;

    /// Create a new app.
    #[allow(clippy::too_many_arguments)]
    async fn create_app(
        &self,
        name: &str,
        description: Option<&str>,
        harness_id: HarnessId,
        agent_id: Option<AgentId>,
        agent_identity_id: Option<AgentIdentityId>,
        channel_type: Option<ChannelType>,
        channel_config: Option<&serde_json::Value>,
    ) -> Result<App>;

    /// Update an app (only provided fields are changed).
    #[allow(clippy::too_many_arguments)]
    async fn update_app(
        &self,
        id: AppId,
        name: Option<&str>,
        description: Option<&str>,
        harness_id: Option<HarnessId>,
        agent_id: Option<AgentId>,
        agent_identity_id: Option<Option<AgentIdentityId>>,
    ) -> Result<App>;

    /// Archive an app.
    async fn delete_app(&self, id: AppId) -> Result<()>;

    /// Permanently destroy an archived app.
    async fn destroy_app(&self, id: AppId) -> Result<()>;

    /// Publish an app.
    async fn publish_app(&self, id: AppId) -> Result<App>;

    /// Unpublish an app back to draft.
    async fn unpublish_app(&self, id: AppId) -> Result<App>;

    /// Add a channel to an app.
    async fn add_app_channel(
        &self,
        app_id: AppId,
        channel_type: ChannelType,
        channel_config: Option<&serde_json::Value>,
        enabled: Option<bool>,
    ) -> Result<AppChannel>;

    /// Update a channel on an app.
    async fn update_app_channel(
        &self,
        app_id: AppId,
        channel_id: AppChannelId,
        channel_type: Option<ChannelType>,
        channel_config: Option<&serde_json::Value>,
        enabled: Option<bool>,
    ) -> Result<AppChannel>;

    /// Delete a channel from an app.
    async fn delete_app_channel(&self, app_id: AppId, channel_id: AppChannelId) -> Result<()>;

    // =========================================================================
    // Session Operations
    // =========================================================================

    /// List sessions, optionally filtered by agent.
    async fn list_sessions(
        &self,
        limit: Option<usize>,
        agent_id: Option<AgentId>,
    ) -> Result<Vec<Session>>;

    /// Create a new session.
    ///
    /// When `blueprint_id` is set, the session runs a blueprint agent instead
    /// of inheriting from `harness_id`/`agent_id`. `blueprint_config` is
    /// validated config for the blueprint (JSON, optional).
    /// `parent_session_id` links child subagent/handoff sessions to their parent
    /// (used to compute governed subagent delegation depth).
    #[allow(clippy::too_many_arguments)]
    async fn create_session(
        &self,
        harness_id: HarnessId,
        agent_id: Option<AgentId>,
        title: Option<&str>,
        locale: Option<&str>,
        blueprint_id: Option<&str>,
        blueprint_config: Option<&serde_json::Value>,
        parent_session_id: Option<SessionId>,
    ) -> Result<Session>;

    async fn create_session_with_options(
        &self,
        request: PlatformCreateSessionRequest,
    ) -> Result<Session> {
        if request.goal.is_some()
            || request.forked_from_session_id.is_some()
            || request.budget_root_session_id.is_some()
            || request.seed != SessionSeedMode::Fresh
        {
            return Err(everruns_core::error::AgentLoopError::tool(
                "platform store does not support goal, lineage, budget-root override, or seeded session creation",
            ));
        }
        self.create_session(
            request.harness_id,
            request.agent_id,
            request.title.as_deref(),
            request.locale.as_deref(),
            request.blueprint_id.as_deref(),
            request.blueprint_config.as_ref(),
            request.parent_session_id,
        )
        .await
    }

    /// Get a session by ID.
    async fn get_session_by_id(&self, id: SessionId) -> Result<Option<Session>>;

    /// Add an agent as a member participant in an existing session.
    async fn add_agent_session_participant(
        &self,
        session_id: SessionId,
        agent_id: AgentId,
    ) -> Result<SessionParticipant>;

    /// Get the latest estimated context breakdown for a session.
    async fn get_session_context_report(&self, id: SessionId) -> Result<SessionContextReport>;

    /// Delete (archive) a session.
    async fn delete_session(&self, id: SessionId) -> Result<()>;

    // =========================================================================
    // Messaging
    // =========================================================================

    /// Send a user message to a session, triggering a turn.
    async fn send_message(&self, session_id: SessionId, content: &str) -> Result<()>;

    /// Get messages from a session (most recent first).
    /// Default limit is 10.
    async fn get_messages(
        &self,
        session_id: SessionId,
        limit: Option<usize>,
    ) -> Result<Vec<PlatformMessage>>;

    // =========================================================================
    // Turn Management
    // =========================================================================

    /// Wait for a session to become idle (turn completed).
    /// Returns the final session status as a string.
    /// Default timeout is 120 seconds.
    async fn wait_for_idle(
        &self,
        session_id: SessionId,
        timeout_secs: Option<u64>,
    ) -> Result<String>;

    // =========================================================================
    // Capabilities
    // =========================================================================

    /// List all available capabilities (built-in + MCP servers + skills).
    ///
    /// Optionally filter by a search query (case-insensitive match against
    /// name, description, category, and capability ID).
    async fn list_capabilities(&self, search: Option<&str>) -> Result<Vec<CapabilityInfo>>;

    // =========================================================================
    // UI Links
    // =========================================================================

    /// Base URL for constructing UI links (e.g., "http://localhost:9300").
    fn base_url(&self) -> &str;
}

/// Typed [`ToolContext`](everruns_core::ToolContext) extension carrying the
/// hosted `PlatformStore` (EVE-839). Replaces the former
/// `ToolContext::platform_store` field; hosted capabilities resolve it via
/// `context.extension::<PlatformStoreExt>()`.
#[derive(Clone)]
pub struct PlatformStoreExt(pub std::sync::Arc<dyn PlatformStore>);

/// Adapter implementing core's narrow
/// [`SubagentSessionDelegate`](everruns_core::subagent_delegation::SubagentSessionDelegate)
/// over a full `PlatformStore`, so portable subagent/handoff orchestration can
/// drive child sessions without depending on the hosted seam.
pub struct PlatformStoreSubagentDelegate(pub std::sync::Arc<dyn PlatformStore>);

#[async_trait]
impl everruns_core::subagent_delegation::SubagentSessionDelegate for PlatformStoreSubagentDelegate {
    async fn get_agent_by_id(&self, id: AgentId) -> Result<Option<everruns_core::AgentDefinition>> {
        // Plain (status-agnostic) projection: handoff/spawn target validation
        // historically saw the stored record regardless of lifecycle status;
        // execution loading seams use `Agent::execution_definition` instead.
        Ok(self
            .0
            .get_agent_by_id(id)
            .await?
            .map(|agent| agent.definition()))
    }
    async fn get_harness(&self, id: HarnessId) -> Result<Option<everruns_core::HarnessDefinition>> {
        // EVE-881: the delegate surfaces the effective (inheritance-resolved)
        // execution configuration; the stored chain stays behind this adapter.
        let chain = self.0.get_harness_chain(id).await?;
        if chain.is_empty() {
            return Ok(None);
        }
        resolve_execution_harness(&chain, id).map(Some)
    }
    async fn add_agent_session_participant(
        &self,
        session_id: SessionId,
        agent_id: AgentId,
    ) -> Result<SessionParticipantId> {
        // The stored participant record stays behind this adapter (EVE-882);
        // portable orchestration only needs the correlation id.
        Ok(self
            .0
            .add_agent_session_participant(session_id, agent_id)
            .await?
            .id)
    }
    async fn create_session_with_options(
        &self,
        request: PlatformCreateSessionRequest,
    ) -> Result<everruns_core::ExecutionSession> {
        // Project the stored aggregate into the portable execution view at
        // the platform seam (EVE-882).
        Ok(self
            .0
            .create_session_with_options(request)
            .await?
            .execution_session())
    }
    async fn get_session_by_id(
        &self,
        id: SessionId,
    ) -> Result<Option<everruns_core::ExecutionSession>> {
        Ok(self
            .0
            .get_session_by_id(id)
            .await?
            .map(|session| session.execution_session()))
    }
    async fn send_message(&self, session_id: SessionId, content: &str) -> Result<()> {
        self.0.send_message(session_id, content).await
    }
    async fn get_messages(
        &self,
        session_id: SessionId,
        limit: Option<usize>,
    ) -> Result<Vec<PlatformMessage>> {
        self.0.get_messages(session_id, limit).await
    }
    async fn wait_for_idle(
        &self,
        session_id: SessionId,
        timeout_secs: Option<u64>,
    ) -> Result<String> {
        self.0.wait_for_idle(session_id, timeout_secs).await
    }
}

#[cfg(test)]
pub mod tests {
    use super::*;
    use crate::agent::{Agent, AgentStatus};
    use crate::app::{App, AppChannel, AppStatus, ChannelType};
    use crate::harness::HarnessStatus;
    use crate::session::{Session, SessionStatus};
    use everruns_core::AgentCapabilityConfig;

    /// Mock PlatformStore for unit tests.
    ///
    /// Shared across test modules so that any test exercising
    /// platform management tools (directly or via ActAtom) uses
    /// the same mock. This prevents wiring bugs where a tool is
    /// registered but the store is not passed through.
    pub struct MockPlatformStore {
        pub harness: Harness,
        pub extra_harnesses: std::sync::Mutex<std::collections::HashMap<HarnessId, Harness>>,
        pub agent: Agent,
        pub app: App,
        pub app_channel: AppChannel,
        pub session: Session,
        pub extra_sessions: std::sync::Mutex<std::collections::HashMap<SessionId, Session>>,
        pub joined_participants: std::sync::Mutex<Vec<SessionParticipant>>,
        /// Records the `harness_id` argument of every `create_session`
        /// call so tests can assert which harness a child session was
        /// created against. See `start_handoff_uses_target_harness_not_parent`.
        pub created_session_harness_ids: std::sync::Mutex<Vec<HarnessId>>,
        /// Records internal budget-root overrides supplied to session creation.
        pub created_session_budget_roots: std::sync::Mutex<Vec<Option<SessionId>>>,
        /// Status returned by `wait_for_idle` ("idle" by default). Tests set
        /// a terminal turn status (e.g. "completed") to exercise settle paths.
        pub wait_for_idle_status: std::sync::Mutex<String>,
        /// Records every `send_message` call as `(session_id, content)` so
        /// tests can assert which session was signaled (e.g. a detached peer
        /// receiving a cooperative-cancel message).
        pub sent_messages: std::sync::Mutex<Vec<(SessionId, String)>>,
    }

    impl Default for MockPlatformStore {
        fn default() -> Self {
            Self::new()
        }
    }

    impl MockPlatformStore {
        pub fn new() -> Self {
            Self {
                harness: Harness {
                    id: HarnessId::new(),
                    name: "test-harness".to_string(),
                    display_name: Some("Test Harness".to_string()),
                    description: Some("test harness".to_string()),
                    system_prompt: Some("You are helpful.".to_string()),
                    parent_harness_id: None,
                    default_model_id: None,
                    tags: vec![],
                    capabilities: vec![AgentCapabilityConfig::new("session")],
                    initial_files: vec![],
                    network_access: None,
                    parallel_tool_calls: None,
                    mcp_servers: Default::default(),
                    embedder_metadata: Default::default(),
                    is_built_in: false,
                    status: HarnessStatus::Active,
                    created_at: chrono::Utc::now(),
                    updated_at: chrono::Utc::now(),
                    archived_at: None,
                    deleted_at: None,
                },
                extra_harnesses: std::sync::Mutex::new(std::collections::HashMap::new()),
                agent: Agent {
                    public_id: everruns_core::typed_id::AgentId::new(),
                    internal_id: uuid::Uuid::now_v7(),
                    name: "test-agent".to_string(),
                    display_name: Some("Test Agent".to_string()),
                    description: Some("test agent".to_string()),
                    system_prompt: "You are helpful.".to_string(),
                    default_model_id: None,

                    harness_id: everruns_core::typed_id::HarnessId::from_uuid(uuid::Uuid::nil()),
                    default_version_id: None,
                    forked_from_agent_id: None,
                    forked_from_version_id: None,
                    root_agent_id: None,
                    tags: vec![],
                    capabilities: vec![],
                    initial_files: vec![],
                    network_access: None,
                    max_iterations: None,
                    parallel_tool_calls: None,
                    tools: vec![],
                    mcp_servers: Default::default(),
                    status: AgentStatus::Active,
                    created_at: chrono::Utc::now(),
                    updated_at: chrono::Utc::now(),
                    archived_at: None,
                    deleted_at: None,
                    usage: None,
                },
                app_channel: AppChannel {
                    public_id: AppChannelId::new(),
                    internal_id: uuid::Uuid::now_v7(),
                    channel_type: ChannelType::Webhook,
                    channel_config: serde_json::json!({
                        "token": "secret-1",
                        "session_mode": "shared_session",
                        "message": "Run checks for {{payload.repo.name}}"
                    }),
                    enabled: true,
                    created_at: chrono::Utc::now(),
                    updated_at: chrono::Utc::now(),
                },
                app: App {
                    public_id: AppId::new(),
                    internal_id: uuid::Uuid::now_v7(),
                    org_id: 1,
                    name: "test-app".to_string(),
                    description: Some("test app".to_string()),
                    harness_id: HarnessId::new(),
                    agent_id: Some(everruns_core::typed_id::AgentId::new()),
                    agent_version_policy: crate::app::AgentVersionPolicy::Default,
                    agent_version_id: None,
                    agent_identity_id: Some(everruns_core::typed_id::AgentIdentityId::new()),
                    owner_principal_id: everruns_core::PrincipalId::from_seed(1),
                    resolved_owner_user_id: None,
                    owner: None,
                    effective_owner: None,
                    channels: vec![],
                    status: AppStatus::Draft,
                    published_at: None,
                    created_at: chrono::Utc::now(),
                    updated_at: chrono::Utc::now(),
                    archived_at: None,
                    deleted_at: None,
                },
                session: {
                    let session_id = SessionId::new();
                    Session {
                        source: Default::default(),
                        activity: Default::default(),
                        // Default 1:1 session<->workspace: workspace.id mirrors the session id.
                        id: session_id,
                        workspace_id: everruns_core::WorkspaceId::from_uuid(session_id.uuid()),
                        organization_id: "org_00000000000000000000000000000001".to_string(),
                        harness_id: HarnessId::new(),
                        agent_id: None,
                        agent_version_id: None,
                        agent_identity_id: None,
                        owner_principal_id: everruns_core::PrincipalId::from_seed(1),
                        resolved_owner_user_id: None,
                        owner: None,
                        effective_owner: None,
                        title: Some("Test Session".to_string()),
                        goal: None,
                        locale: None,
                        preview: None,
                        output_preview: None,
                        tags: vec![],
                        model_id: None,
                        capabilities: vec![],
                        tools: vec![],
                        mcp_servers: Default::default(),
                        system_prompt: None,
                        initial_files: vec![],
                        hints: None,
                        network_access: None,
                        max_iterations: None,
                        parallel_tool_calls: None,
                        status: SessionStatus::Idle,
                        created_at: chrono::Utc::now(),
                        updated_at: chrono::Utc::now(),
                        started_at: None,
                        finished_at: None,
                        usage: None,
                        is_pinned: None,
                        active_schedule_count: None,
                        features: vec![],
                        parent_session_id: None,
                        forked_from_session_id: None,
                        forked_from_sequence: None,
                        blueprint_id: None,
                        blueprint_config: None,
                    }
                },
                extra_sessions: std::sync::Mutex::new(std::collections::HashMap::new()),
                joined_participants: std::sync::Mutex::new(Vec::new()),
                created_session_harness_ids: std::sync::Mutex::new(Vec::new()),
                created_session_budget_roots: std::sync::Mutex::new(Vec::new()),
                wait_for_idle_status: std::sync::Mutex::new("idle".to_string()),
                sent_messages: std::sync::Mutex::new(Vec::new()),
            }
        }
    }

    #[async_trait]
    impl PlatformStore for MockPlatformStore {
        async fn platform_discover(&self, arguments: serde_json::Value) -> Result<String> {
            Ok(arguments.to_string())
        }

        async fn platform_query(&self, arguments: serde_json::Value) -> Result<String> {
            Ok(arguments.to_string())
        }

        async fn platform_execute(&self, arguments: serde_json::Value) -> Result<String> {
            Ok(arguments.to_string())
        }

        async fn list_harnesses(&self) -> Result<Vec<Harness>> {
            Ok(vec![self.harness.clone()])
        }
        async fn get_harness(&self, id: HarnessId) -> Result<Option<Harness>> {
            if let Some(harness) = self.extra_harnesses.lock().unwrap().get(&id).cloned() {
                return Ok(Some(harness));
            }
            Ok(Some(self.harness.clone()))
        }
        async fn create_harness(
            &self,
            name: &str,
            display_name: Option<&str>,
            _desc: Option<&str>,
            _prompt: Option<&str>,
            parent_harness_id: Option<HarnessId>,
            _caps: &[String],
        ) -> Result<Harness> {
            let mut h = self.harness.clone();
            h.name = name.to_string();
            h.display_name = display_name.map(|s| s.to_string());
            h.parent_harness_id = parent_harness_id;
            Ok(h)
        }
        async fn update_harness(
            &self,
            _id: HarnessId,
            name: Option<&str>,
            display_name: Option<&str>,
            _desc: Option<&str>,
            _prompt: Option<&str>,
            parent_harness_id: Option<Option<HarnessId>>,
        ) -> Result<Harness> {
            let mut h = self.harness.clone();
            if let Some(n) = name {
                h.name = n.to_string();
            }
            if let Some(dn) = display_name {
                h.display_name = Some(dn.to_string());
            }
            if let Some(parent_harness_id) = parent_harness_id {
                h.parent_harness_id = parent_harness_id;
            }
            Ok(h)
        }
        async fn delete_harness(&self, _id: HarnessId) -> Result<()> {
            Ok(())
        }
        async fn copy_harness(&self, _id: HarnessId, new_name: Option<&str>) -> Result<Harness> {
            let mut h = self.harness.clone();
            h.id = HarnessId::new();
            h.name = new_name.unwrap_or("copy").to_string();
            Ok(h)
        }
        async fn list_agents(&self) -> Result<Vec<Agent>> {
            Ok(vec![self.agent.clone()])
        }
        async fn get_agent_by_id(
            &self,
            _id: everruns_core::typed_id::AgentId,
        ) -> Result<Option<Agent>> {
            Ok(Some(self.agent.clone()))
        }
        async fn create_agent(
            &self,
            name: &str,
            display_name: Option<&str>,
            _desc: Option<&str>,
            _prompt: &str,
            _caps: &[String],
        ) -> Result<Agent> {
            let mut a = self.agent.clone();
            a.name = name.to_string();
            a.display_name = display_name.map(|s| s.to_string());
            Ok(a)
        }
        async fn update_agent(
            &self,
            _id: everruns_core::typed_id::AgentId,
            name: Option<&str>,
            display_name: Option<&str>,
            _desc: Option<&str>,
            _prompt: Option<&str>,
        ) -> Result<Agent> {
            let mut a = self.agent.clone();
            if let Some(n) = name {
                a.name = n.to_string();
            }
            if let Some(dn) = display_name {
                a.display_name = Some(dn.to_string());
            }
            Ok(a)
        }
        async fn delete_agent(&self, _id: everruns_core::typed_id::AgentId) -> Result<()> {
            Ok(())
        }
        async fn list_apps(
            &self,
            _search: Option<&str>,
            _include_archived: bool,
        ) -> Result<Vec<App>> {
            let mut app = self.app.clone();
            app.channels = vec![self.app_channel.clone()];
            Ok(vec![app])
        }
        async fn get_app(&self, _id: AppId) -> Result<Option<App>> {
            let mut app = self.app.clone();
            app.channels = vec![self.app_channel.clone()];
            Ok(Some(app))
        }
        async fn create_app(
            &self,
            name: &str,
            description: Option<&str>,
            harness_id: HarnessId,
            agent_id: Option<AgentId>,
            agent_identity_id: Option<AgentIdentityId>,
            channel_type: Option<ChannelType>,
            channel_config: Option<&serde_json::Value>,
        ) -> Result<App> {
            let mut app = self.app.clone();
            app.name = name.to_string();
            app.description = description.map(|value| value.to_string());
            app.harness_id = harness_id;
            app.agent_id = agent_id;
            app.agent_identity_id = agent_identity_id;
            app.channels = channel_type
                .map(|channel_type| {
                    let mut channel = self.app_channel.clone();
                    channel.channel_type = channel_type;
                    if let Some(channel_config) = channel_config {
                        channel.channel_config = channel_config.clone();
                    }
                    vec![channel]
                })
                .unwrap_or_default();
            Ok(app)
        }
        async fn update_app(
            &self,
            _id: AppId,
            name: Option<&str>,
            description: Option<&str>,
            harness_id: Option<HarnessId>,
            agent_id: Option<AgentId>,
            agent_identity_id: Option<Option<AgentIdentityId>>,
        ) -> Result<App> {
            let mut app = self.app.clone();
            app.channels = vec![self.app_channel.clone()];
            if let Some(name) = name {
                app.name = name.to_string();
            }
            if let Some(description) = description {
                app.description = Some(description.to_string());
            }
            if let Some(harness_id) = harness_id {
                app.harness_id = harness_id;
            }
            if let Some(agent_id) = agent_id {
                app.agent_id = Some(agent_id);
            }
            if let Some(agent_identity_id) = agent_identity_id {
                app.agent_identity_id = agent_identity_id;
            }
            Ok(app)
        }
        async fn delete_app(&self, _id: AppId) -> Result<()> {
            Ok(())
        }
        async fn destroy_app(&self, _id: AppId) -> Result<()> {
            Ok(())
        }
        async fn publish_app(&self, _id: AppId) -> Result<App> {
            let mut app = self.app.clone();
            app.channels = vec![self.app_channel.clone()];
            app.status = AppStatus::Published;
            app.published_at = Some(chrono::Utc::now());
            Ok(app)
        }
        async fn unpublish_app(&self, _id: AppId) -> Result<App> {
            let mut app = self.app.clone();
            app.channels = vec![self.app_channel.clone()];
            app.status = AppStatus::Draft;
            app.published_at = None;
            Ok(app)
        }
        async fn add_app_channel(
            &self,
            _app_id: AppId,
            channel_type: ChannelType,
            channel_config: Option<&serde_json::Value>,
            enabled: Option<bool>,
        ) -> Result<AppChannel> {
            let mut channel = self.app_channel.clone();
            channel.channel_type = channel_type;
            if let Some(channel_config) = channel_config {
                channel.channel_config = channel_config.clone();
            }
            if let Some(enabled) = enabled {
                channel.enabled = enabled;
            }
            Ok(channel)
        }
        async fn update_app_channel(
            &self,
            _app_id: AppId,
            _channel_id: AppChannelId,
            channel_type: Option<ChannelType>,
            channel_config: Option<&serde_json::Value>,
            enabled: Option<bool>,
        ) -> Result<AppChannel> {
            let mut channel = self.app_channel.clone();
            if let Some(channel_type) = channel_type {
                channel.channel_type = channel_type;
            }
            if let Some(channel_config) = channel_config {
                channel.channel_config = channel_config.clone();
            }
            if let Some(enabled) = enabled {
                channel.enabled = enabled;
            }
            Ok(channel)
        }
        async fn delete_app_channel(
            &self,
            _app_id: AppId,
            _channel_id: AppChannelId,
        ) -> Result<()> {
            Ok(())
        }
        async fn list_sessions(
            &self,
            _limit: Option<usize>,
            _agent_id: Option<everruns_core::typed_id::AgentId>,
        ) -> Result<Vec<Session>> {
            Ok(vec![self.session.clone()])
        }
        async fn create_session(
            &self,
            hid: HarnessId,
            aid: Option<everruns_core::typed_id::AgentId>,
            title: Option<&str>,
            locale: Option<&str>,
            blueprint_id: Option<&str>,
            blueprint_config: Option<&serde_json::Value>,
            parent_session_id: Option<SessionId>,
        ) -> Result<Session> {
            if let Ok(mut recorder) = self.created_session_harness_ids.lock() {
                recorder.push(hid);
            }
            let mut s = self.session.clone();
            s.id = SessionId::new();
            s.harness_id = hid;
            s.agent_id = aid;
            s.title = title.map(|t| t.to_string());
            s.locale = locale.map(|value| value.to_string());
            s.blueprint_id = blueprint_id.map(|b| b.to_string());
            s.blueprint_config = blueprint_config.cloned();
            s.parent_session_id = parent_session_id;
            if let Ok(mut sessions) = self.extra_sessions.lock() {
                sessions.insert(s.id, s.clone());
            }
            Ok(s)
        }
        async fn create_session_with_options(
            &self,
            request: PlatformCreateSessionRequest,
        ) -> Result<Session> {
            self.created_session_budget_roots
                .lock()
                .expect("budget root recorder")
                .push(request.budget_root_session_id);
            let mut session = self
                .create_session(
                    request.harness_id,
                    request.agent_id,
                    request.title.as_deref(),
                    request.locale.as_deref(),
                    request.blueprint_id.as_deref(),
                    request.blueprint_config.as_ref(),
                    request.parent_session_id,
                )
                .await?;
            session.goal = request.goal;
            session.forked_from_session_id = request.forked_from_session_id;
            if let Ok(mut sessions) = self.extra_sessions.lock() {
                sessions.insert(session.id, session.clone());
            }
            Ok(session)
        }
        async fn get_session_by_id(&self, id: SessionId) -> Result<Option<Session>> {
            if id == self.session.id {
                return Ok(Some(self.session.clone()));
            }
            if let Some(session) = self
                .extra_sessions
                .lock()
                .ok()
                .and_then(|sessions| sessions.get(&id).cloned())
            {
                return Ok(Some(session));
            }
            Ok(Some(self.session.clone()))
        }
        async fn add_agent_session_participant(
            &self,
            session_id: SessionId,
            agent_id: AgentId,
        ) -> Result<SessionParticipant> {
            let participant = SessionParticipant {
                id: everruns_core::typed_id::SessionParticipantId::new(),
                session_id,
                kind: crate::session::SessionParticipantKind::Agent,
                agent_id: Some(agent_id),
                agent_version_id: self.agent.default_version_id,
                principal_id: self.session.owner_principal_id,
                display_name: None,
                role: crate::session::SessionParticipantRole::Member,
                joined_at: chrono::Utc::now(),
                left_at: None,
            };
            if let Ok(mut participants) = self.joined_participants.lock() {
                participants.push(participant.clone());
            }
            Ok(participant)
        }
        async fn get_session_context_report(&self, id: SessionId) -> Result<SessionContextReport> {
            Ok(SessionContextReport {
                session_id: id.to_string(),
                model: "llmsim".to_string(),
                context_window_tokens: Some(128_000),
                estimated_input_tokens: 42,
                sections: vec![everruns_core::ContextReportSection {
                    key: "conversation".to_string(),
                    label: "Conversation".to_string(),
                    tokens: 42,
                    items: 1,
                }],
                contributions: vec![],
                cumulative_usage: None,
            })
        }
        async fn delete_session(&self, _id: SessionId) -> Result<()> {
            Ok(())
        }
        async fn send_message(&self, id: SessionId, content: &str) -> Result<()> {
            self.sent_messages
                .lock()
                .unwrap()
                .push((id, content.to_string()));
            Ok(())
        }
        async fn get_messages(
            &self,
            _id: SessionId,
            _limit: Option<usize>,
        ) -> Result<Vec<PlatformMessage>> {
            Ok(vec![
                PlatformMessage {
                    role: "user".into(),
                    content: "Hello".into(),
                    created_at: chrono::Utc::now(),
                },
                PlatformMessage {
                    role: "agent".into(),
                    content: "Hi!".into(),
                    created_at: chrono::Utc::now(),
                },
            ])
        }
        async fn wait_for_idle(&self, _id: SessionId, _t: Option<u64>) -> Result<String> {
            Ok(self.wait_for_idle_status.lock().unwrap().clone())
        }
        async fn list_capabilities(&self, search: Option<&str>) -> Result<Vec<CapabilityInfo>> {
            let registry = everruns_core::capabilities::CapabilityRegistry::with_builtins();
            let mut caps: Vec<CapabilityInfo> = registry
                .list()
                .iter()
                .map(|c| CapabilityInfo::from_core(c.as_ref()))
                .collect();
            if let Some(q) = search {
                caps.retain(|c| c.matches_search(q));
            }
            caps.sort_by(|a, b| a.name.cmp(&b.name));
            Ok(caps)
        }
        fn base_url(&self) -> &str {
            "http://localhost:9300"
        }
    }
}
