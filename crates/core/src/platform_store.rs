// Platform Store trait for org-scoped management operations
//
// Decision: Single trait covers harness, agent, session CRUD + messaging
// Decision: Trait lives in core; implementations in server/worker crates
// Decision: Tool results include UI links via base_url()
// Decision: PlatformMessage is a simplified view (role + text + timestamp)

use crate::agent::Agent;
use crate::capability_dto::CapabilityInfo;
use crate::error::Result;
use crate::harness::Harness;
use crate::session::Session;
use crate::typed_id::{AgentId, HarnessId, SessionId};
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
    async fn create_harness(
        &self,
        name: &str,
        description: Option<&str>,
        system_prompt: &str,
        parent_harness_id: Option<HarnessId>,
        capabilities: &[String],
    ) -> Result<Harness>;

    /// Update a harness (only provided fields are changed).
    async fn update_harness(
        &self,
        id: HarnessId,
        name: Option<&str>,
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
        description: Option<&str>,
        system_prompt: &str,
        capabilities: &[String],
    ) -> Result<Agent>;

    /// Update an agent (only provided fields are changed).
    async fn update_agent(
        &self,
        id: AgentId,
        name: Option<&str>,
        description: Option<&str>,
        system_prompt: Option<&str>,
    ) -> Result<Agent>;

    /// Delete (archive) an agent.
    async fn delete_agent(&self, id: AgentId) -> Result<()>;

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
    async fn create_session(
        &self,
        harness_id: HarnessId,
        agent_id: Option<AgentId>,
        title: Option<&str>,
        locale: Option<&str>,
    ) -> Result<Session>;

    /// Get a session by ID.
    async fn get_session_by_id(&self, id: SessionId) -> Result<Option<Session>>;

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
        pub session: Session,
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
                    name: "Test Harness".to_string(),
                    description: Some("test harness".to_string()),
                    system_prompt: "You are helpful.".to_string(),
                    parent_harness_id: None,
                    default_model_id: None,
                    tags: vec![],
                    capabilities: vec![AgentCapabilityConfig::new("session")],
                    initial_files: vec![],
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
                    name: "Test Agent".to_string(),
                    description: Some("test agent".to_string()),
                    system_prompt: "You are helpful.".to_string(),
                    default_model_id: None,
                    tags: vec![],
                    capabilities: vec![],
                    initial_files: vec![],
                    tools: vec![],
                    status: AgentStatus::Active,
                    created_at: chrono::Utc::now(),
                    updated_at: chrono::Utc::now(),
                    archived_at: None,
                    deleted_at: None,
                    usage: None,
                },
                session: Session {
                    id: SessionId::new(),
                    organization_id: "org_00000000000000000000000000000001".to_string(),
                    harness_id: HarnessId::new(),
                    agent_id: None,
                    agent_identity_id: None,
                    title: Some("Test Session".to_string()),
                    locale: None,
                    preview: None,
                    output_preview: None,
                    tags: vec![],
                    model_id: None,
                    capabilities: vec![],
                    tools: vec![],
                    hints: None,
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
                    subagent_name: None,
                    subagent_task: None,
                    subagent_status: None,
                },
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
            _desc: Option<&str>,
            _prompt: &str,
            parent_harness_id: Option<HarnessId>,
            _caps: &[String],
        ) -> Result<Harness> {
            let mut h = self.harness.clone();
            h.name = name.to_string();
            h.parent_harness_id = parent_harness_id;
            Ok(h)
        }
        async fn update_harness(
            &self,
            _id: HarnessId,
            name: Option<&str>,
            _desc: Option<&str>,
            _prompt: Option<&str>,
            parent_harness_id: Option<Option<HarnessId>>,
        ) -> Result<Harness> {
            let mut h = self.harness.clone();
            if let Some(n) = name {
                h.name = n.to_string();
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
            h.name = new_name.unwrap_or("Copy").to_string();
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
            _desc: Option<&str>,
            _prompt: &str,
            _caps: &[String],
        ) -> Result<Agent> {
            let mut a = self.agent.clone();
            a.name = name.to_string();
            Ok(a)
        }
        async fn update_agent(
            &self,
            _id: crate::typed_id::AgentId,
            name: Option<&str>,
            _desc: Option<&str>,
            _prompt: Option<&str>,
        ) -> Result<Agent> {
            let mut a = self.agent.clone();
            if let Some(n) = name {
                a.name = n.to_string();
            }
            Ok(a)
        }
        async fn delete_agent(&self, _id: crate::typed_id::AgentId) -> Result<()> {
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
            _hid: HarnessId,
            _aid: Option<crate::typed_id::AgentId>,
            title: Option<&str>,
            locale: Option<&str>,
        ) -> Result<Session> {
            let mut s = self.session.clone();
            s.title = title.map(|t| t.to_string());
            s.locale = locale.map(|value| value.to_string());
            Ok(s)
        }
        async fn get_session_by_id(&self, _id: SessionId) -> Result<Option<Session>> {
            Ok(Some(self.session.clone()))
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
