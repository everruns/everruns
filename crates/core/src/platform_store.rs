// Platform Store trait for org-scoped management operations
//
// Decision: Single trait covers harness, agent, session CRUD + messaging
// Decision: Trait lives in core; implementations in server/worker crates
// Decision: Tool results include UI links via base_url()
// Decision: PlatformMessage is a simplified view (role + text + timestamp)

use crate::SessionContextReport;
use crate::agent::Agent;
use crate::app::{App, AppChannel, ChannelType};
use crate::capability_dto::CapabilityInfo;
use crate::error::Result;
use crate::harness::Harness;
use crate::session::Session;
use crate::typed_id::{AgentId, AgentIdentityId, AppChannelId, AppId, HarnessId, SessionId};
use async_trait::async_trait;
use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};

/// Simplified message representation for platform management tools.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PlatformMessage {
    pub role: String,
    pub content: String,
    pub created_at: DateTime<Utc>,
}

/// Trait for platform-level management operations.
///
/// Provides org-scoped CRUD for harnesses, agents, and sessions,
/// plus session messaging and turn management. Used by the
/// `platform_management` capability tools.
#[async_trait]
pub trait PlatformStore: Send + Sync {
    // =========================================================================
    // Harness Operations
    // =========================================================================

    /// List all harnesses in the organization.
    async fn list_harnesses(&self) -> Result<Vec<Harness>>;

    /// Get a harness by ID.
    async fn get_harness(&self, id: HarnessId) -> Result<Option<Harness>>;

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
    /// (used as the nesting guard in spawn_subagent).
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

    /// Get a session by ID.
    async fn get_session_by_id(&self, id: SessionId) -> Result<Option<Session>>;

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

#[cfg(test)]
pub mod tests {
    use super::*;
    use crate::AgentCapabilityConfig;
    use crate::agent::{Agent, AgentStatus};
    use crate::app::{App, AppChannel, AppStatus, ChannelType};
    use crate::harness::{Harness, HarnessStatus};
    use crate::session::{Session, SessionStatus};

    /// Mock PlatformStore for unit tests.
    ///
    /// Shared across test modules so that any test exercising
    /// platform management tools (directly or via ActAtom) uses
    /// the same mock. This prevents wiring bugs where a tool is
    /// registered but the store is not passed through.
    pub struct MockPlatformStore {
        pub harness: Harness,
        pub agent: Agent,
        pub app: App,
        pub app_channel: AppChannel,
        pub session: Session,
        /// Records the `harness_id` argument of every `create_session`
        /// call so tests can assert which harness a child session was
        /// created against. See `start_handoff_uses_target_harness_not_parent`.
        pub created_session_harness_ids: std::sync::Mutex<Vec<HarnessId>>,
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
                    mcp_servers: Default::default(),
                    embedder_metadata: Default::default(),
                    is_built_in: false,
                    status: HarnessStatus::Active,
                    created_at: chrono::Utc::now(),
                    updated_at: chrono::Utc::now(),
                    archived_at: None,
                    deleted_at: None,
                },
                agent: Agent {
                    public_id: crate::typed_id::AgentId::new(),
                    internal_id: uuid::Uuid::now_v7(),
                    name: "test-agent".to_string(),
                    display_name: Some("Test Agent".to_string()),
                    description: Some("test agent".to_string()),
                    system_prompt: "You are helpful.".to_string(),
                    default_model_id: None,
                    default_version_id: None,
                    forked_from_agent_id: None,
                    forked_from_version_id: None,
                    root_agent_id: None,
                    tags: vec![],
                    capabilities: vec![],
                    initial_files: vec![],
                    network_access: None,
                    max_iterations: None,
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
                    agent_id: Some(crate::typed_id::AgentId::new()),
                    agent_version_policy: crate::app::AgentVersionPolicy::Default,
                    agent_version_id: None,
                    agent_identity_id: Some(crate::typed_id::AgentIdentityId::new()),
                    owner_principal_id: crate::PrincipalId::from_seed(1),
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
                        // Default 1:1 session<->workspace: workspace.id mirrors the session id.
                        id: session_id,
                        workspace_id: crate::WorkspaceId::from_uuid(session_id.uuid()),
                        organization_id: "org_00000000000000000000000000000001".to_string(),
                        harness_id: HarnessId::new(),
                        agent_id: None,
                        agent_version_id: None,
                        agent_identity_id: None,
                        owner_principal_id: crate::PrincipalId::from_seed(1),
                        resolved_owner_user_id: None,
                        owner: None,
                        effective_owner: None,
                        title: Some("Test Session".to_string()),
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
                        blueprint_id: None,
                        blueprint_config: None,
                    }
                },
                created_session_harness_ids: std::sync::Mutex::new(Vec::new()),
            }
        }
    }

    #[async_trait]
    impl PlatformStore for MockPlatformStore {
        async fn list_harnesses(&self) -> Result<Vec<Harness>> {
            Ok(vec![self.harness.clone()])
        }
        async fn get_harness(&self, _id: HarnessId) -> Result<Option<Harness>> {
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
        async fn get_agent_by_id(&self, _id: crate::typed_id::AgentId) -> Result<Option<Agent>> {
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
            _id: crate::typed_id::AgentId,
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
        async fn delete_agent(&self, _id: crate::typed_id::AgentId) -> Result<()> {
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
            _agent_id: Option<crate::typed_id::AgentId>,
        ) -> Result<Vec<Session>> {
            Ok(vec![self.session.clone()])
        }
        async fn create_session(
            &self,
            hid: HarnessId,
            aid: Option<crate::typed_id::AgentId>,
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
            Ok(s)
        }
        async fn get_session_by_id(&self, _id: SessionId) -> Result<Option<Session>> {
            Ok(Some(self.session.clone()))
        }
        async fn get_session_context_report(&self, id: SessionId) -> Result<SessionContextReport> {
            Ok(SessionContextReport {
                session_id: id.to_string(),
                model: "llmsim".to_string(),
                context_window_tokens: Some(128_000),
                estimated_input_tokens: 42,
                sections: vec![crate::ContextReportSection {
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
        async fn send_message(&self, _id: SessionId, _content: &str) -> Result<()> {
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
            Ok("idle".to_string())
        }
        async fn list_capabilities(&self, search: Option<&str>) -> Result<Vec<CapabilityInfo>> {
            let registry = crate::capabilities::CapabilityRegistry::with_builtins();
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
