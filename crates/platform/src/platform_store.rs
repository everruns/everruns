// Platform Store trait: the hosted seam behind agent delegation.
//
// Decision: Trait lives in this crate; implementations in server/worker crates
// Decision: PlatformMessage is a simplified view (role + text + timestamp)
// Decision: scope is the catalog-backed `platform` command surface plus the
//   narrow session/agent/harness reads delegation needs. The org-scoped CRUD
//   half was retired with `platform_management` (EVE-953); management flows
//   go through the domain-command catalog instead.

use crate::agent::Agent;
use crate::harness::{Harness, resolve_execution_harness};
use async_trait::async_trait;
use everruns_provider::error::Result;
use everruns_provider::typed_id::SessionParticipantId;

use crate::session::{Session, SessionParticipant};
use everruns_provider::typed_id::{AgentId, HarnessId, SessionId};
use std::collections::HashSet;

// The narrow session-delegation DTOs live in `everruns-core` (they are part of
// the `SubagentSessionDelegate` contract portable code depends on); the full
// `PlatformStore` reuses them. See EVE-839.
pub use everruns_core::subagent_delegation::{PlatformCreateSessionRequest, PlatformMessage};

/// Trait for platform-level management operations.
///
/// Provides the catalog-backed `platform` surface plus legacy org-scoped CRUD.
/// The CRUD half outlived the retired `platform_management` capability because
/// the delegation capabilities (`subagents`, `agent_handoff`,
/// `a2a_agent_delegation`) and session tooling still call parts of it.
#[async_trait]
pub trait PlatformStore: Send + Sync {
    // =========================================================================
    // Catalog-backed command surface
    // =========================================================================

    /// Search the authoritative domain-command catalog.
    async fn platform_discover(&self, _arguments: serde_json::Value) -> Result<String> {
        Err(everruns_provider::error::AgentLoopError::config(
            "Platform command surface is not available in this host",
        ))
    }

    /// Execute a bounded script against read-only domain commands.
    async fn platform_query(&self, _arguments: serde_json::Value) -> Result<String> {
        Err(everruns_provider::error::AgentLoopError::config(
            "Platform command surface is not available in this host",
        ))
    }

    /// Execute a bounded script against the full authorized command catalog.
    async fn platform_execute(&self, _arguments: serde_json::Value) -> Result<String> {
        Err(everruns_provider::error::AgentLoopError::config(
            "Platform command surface is not available in this host",
        ))
    }

    // =========================================================================
    // Harness Operations
    // =========================================================================

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
                return Err(everruns_provider::error::AgentLoopError::tool(format!(
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

    // =========================================================================
    // Agent Operations
    // =========================================================================

    /// Get an agent by public ID.
    async fn get_agent_by_id(&self, id: AgentId) -> Result<Option<Agent>>;

    // =========================================================================
    // App Operations
    // =========================================================================

    // =========================================================================
    // Session Operations
    // =========================================================================

    /// Create a session for delegation (subagents, handoff, A2A).
    ///
    /// Required rather than defaulted: the flat `create_session` it used to
    /// delegate to was legacy CRUD with no remaining callers, and every
    /// implementation already overrode this method to honour the fields that
    /// default could not express (goal, lineage, budget root, seed mode).
    async fn create_session_with_options(
        &self,
        request: PlatformCreateSessionRequest,
    ) -> Result<Session>;

    /// Get a session by ID.
    async fn get_session_by_id(&self, id: SessionId) -> Result<Option<Session>>;

    /// Add an agent as a member participant in an existing session.
    async fn add_agent_session_participant(
        &self,
        session_id: SessionId,
        agent_id: AgentId,
    ) -> Result<SessionParticipant>;

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

    // =========================================================================
    // UI Links
    // =========================================================================
}

/// Typed [`ToolContext`](everruns_core::tool_context::ToolContext) extension carrying the
/// hosted `PlatformStore` (EVE-839). Replaces the former
/// `ToolContext::platform_store` field; hosted capabilities resolve it via
/// `context.extension::<PlatformStoreExt>()`.
#[derive(Clone)]
pub struct PlatformStoreExt(pub std::sync::Arc<dyn PlatformStore>);

/// Adapter implementing core's narrow
/// [`SubagentSessionDelegate`](everruns_core::subagent_delegation::SubagentSessionDelegate)
/// over a full `PlatformStore`, so hosted subagent/handoff orchestration can
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
    use crate::harness::HarnessStatus;
    use crate::session::{Session, SessionStatus};
    use everruns_capability::CapabilityRef as AgentCapabilityConfig;

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
                    icon: None,
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
                    public_id: everruns_provider::typed_id::AgentId::new(),
                    internal_id: uuid::Uuid::now_v7(),
                    name: "test-agent".to_string(),
                    display_name: Some("Test Agent".to_string()),
                    description: Some("test agent".to_string()),
                    system_prompt: "You are helpful.".to_string(),
                    default_model_id: None,

                    harness_id: everruns_provider::typed_id::HarnessId::from_uuid(uuid::Uuid::nil()),
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
                session: {
                    let session_id = SessionId::new();
                    Session {
                        source: Default::default(),
                        activity: Default::default(),
                        run_summary: None,
                        // Default 1:1 session<->workspace: workspace.id mirrors the session id.
                        id: session_id,
                        workspace_id: everruns_provider::typed_id::WorkspaceId::from_uuid(
                            session_id.uuid(),
                        ),
                        organization_id: "org_00000000000000000000000000000001".to_string(),
                        harness_id: HarnessId::new(),
                        agent_id: None,
                        agent_version_id: None,
                        agent_identity_id: None,
                        owner_principal_id: everruns_provider::typed_id::PrincipalId::from_seed(1),
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
                        archived_at: None,
                        active_schedule_count: None,
                        event_count: None,
                        task_count: None,
                        file_count: None,
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

        async fn get_harness(&self, id: HarnessId) -> Result<Option<Harness>> {
            if let Some(harness) = self.extra_harnesses.lock().unwrap().get(&id).cloned() {
                return Ok(Some(harness));
            }
            // An unregistered id resolves to the default harness *under the
            // requested id*, so inheritance resolution (EVE-881) terminates at
            // the harness the caller asked for instead of reporting a chain
            // mismatch.
            let mut harness = self.harness.clone();
            harness.id = id;
            Ok(Some(harness))
        }
        async fn get_agent_by_id(
            &self,
            _id: everruns_provider::typed_id::AgentId,
        ) -> Result<Option<Agent>> {
            Ok(Some(self.agent.clone()))
        }
        async fn create_session_with_options(
            &self,
            request: PlatformCreateSessionRequest,
        ) -> Result<Session> {
            self.created_session_budget_roots
                .lock()
                .expect("budget root recorder")
                .push(request.budget_root_session_id);
            if let Ok(mut recorder) = self.created_session_harness_ids.lock() {
                recorder.push(request.harness_id);
            }
            let mut session = self.session.clone();
            session.id = SessionId::new();
            session.harness_id = request.harness_id;
            session.agent_id = request.agent_id;
            session.title = request.title.clone();
            session.locale = request.locale.clone();
            session.blueprint_id = request.blueprint_id.clone();
            session.blueprint_config = request.blueprint_config.clone();
            session.parent_session_id = request.parent_session_id;
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
                id: everruns_provider::typed_id::SessionParticipantId::new(),
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
    }
}
