//! Narrow session-delegation contract for portable subagent/handoff orchestration.
//!
//! EVE-839: portable capabilities (`subagents`, `agent_handoff`) drive child
//! sessions — create, message, wait, read — without depending on the full
//! hosted [`PlatformStore`](https://docs.rs/everruns-platform) seam. They use
//! this narrow, `everruns-core`-owned trait instead. The hosted platform crate
//! implements it by delegating to its `PlatformStore`, so core carries no
//! `PlatformStore` symbol while server/worker keep identical behavior.
//!
//! The request/message DTOs live here (not in `everruns-platform`) because the
//! trait signature needs them and core cannot depend on platform.

use crate::agent::Agent;
use crate::error::Result;
use crate::harness::Harness;
use crate::session::{Session, SessionParticipant, SessionSeedMode};
use crate::typed_id::{AgentId, HarnessId, SessionId};
use async_trait::async_trait;
use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use std::collections::HashSet;

/// Simplified message representation for subagent/handoff result collection.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PlatformMessage {
    pub role: String,
    pub content: String,
    pub created_at: DateTime<Utc>,
}

/// Options for delegate-backed child-session creation from model-facing tools.
#[derive(Debug, Clone)]
pub struct PlatformCreateSessionRequest {
    pub harness_id: HarnessId,
    pub agent_id: Option<AgentId>,
    pub title: Option<String>,
    pub goal: Option<String>,
    pub locale: Option<String>,
    pub blueprint_id: Option<String>,
    pub blueprint_config: Option<serde_json::Value>,
    pub parent_session_id: Option<SessionId>,
    pub forked_from_session_id: Option<SessionId>,
    /// Internal-only override for the budget/delegation root. Detached spawns
    /// set this explicitly; ordinary forks must leave it unset.
    pub budget_root_session_id: Option<SessionId>,
    pub seed: SessionSeedMode,
}

/// The narrow set of child-session operations portable subagent orchestration
/// needs. Implemented by the hosted platform adapter (over `PlatformStore`);
/// carried on [`ToolContext`](crate::ToolContext) as an optional service.
#[async_trait]
pub trait SubagentSessionDelegate: Send + Sync {
    /// Look up an agent by id (target validation for handoff/spawn).
    async fn get_agent_by_id(&self, id: AgentId) -> Result<Option<Agent>>;

    /// Look up a harness by id (parent harness resolution).
    async fn get_harness(&self, id: HarnessId) -> Result<Option<Harness>>;

    /// Resolve the inheritance chain (root-first) for a harness. Default impl
    /// walks `parent_harness_id` via [`Self::get_harness`], guarding cycles.
    async fn get_harness_chain(&self, id: HarnessId) -> Result<Vec<Harness>> {
        let mut chain = Vec::new();
        let mut current_id = Some(id);
        let mut seen = HashSet::new();

        while let Some(harness_id) = current_id {
            if !seen.insert(harness_id) {
                return Err(crate::error::AgentLoopError::tool(format!(
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

    /// Add an agent as a member participant in an existing session.
    async fn add_agent_session_participant(
        &self,
        session_id: SessionId,
        agent_id: AgentId,
    ) -> Result<SessionParticipant>;

    /// Create a child session with the given options.
    async fn create_session_with_options(
        &self,
        request: PlatformCreateSessionRequest,
    ) -> Result<Session>;

    /// Get a session by id.
    async fn get_session_by_id(&self, id: SessionId) -> Result<Option<Session>>;

    /// Send a user message to a session, triggering a turn.
    async fn send_message(&self, session_id: SessionId, content: &str) -> Result<()>;

    /// Get messages from a session (most recent first). Default limit is 10.
    async fn get_messages(
        &self,
        session_id: SessionId,
        limit: Option<usize>,
    ) -> Result<Vec<PlatformMessage>>;

    /// Wait for a session to become idle; returns the final status string.
    async fn wait_for_idle(
        &self,
        session_id: SessionId,
        timeout_secs: Option<u64>,
    ) -> Result<String>;
}

#[cfg(test)]
pub mod tests {
    use super::*;
    use crate::AgentCapabilityConfig;
    use crate::agent::AgentStatus;
    use crate::harness::HarnessStatus;
    use crate::session::{SessionParticipant, SessionStatus};

    /// Mock [`SubagentSessionDelegate`] for portable subagent/handoff tests.
    ///
    /// Carries the same simulated harness/agent/session state the former
    /// `MockPlatformStore` provided, restricted to the narrow delegate surface
    /// (EVE-839). The full hosted mock still lives in `everruns-platform`.
    pub struct MockSubagentDelegate {
        pub harness: Harness,
        pub extra_harnesses: std::sync::Mutex<std::collections::HashMap<HarnessId, Harness>>,
        pub agent: Agent,
        pub session: Session,
        pub extra_sessions: std::sync::Mutex<std::collections::HashMap<SessionId, Session>>,
        pub joined_participants: std::sync::Mutex<Vec<SessionParticipant>>,
        pub created_session_harness_ids: std::sync::Mutex<Vec<HarnessId>>,
        pub created_session_budget_roots: std::sync::Mutex<Vec<Option<SessionId>>>,
        pub wait_for_idle_status: std::sync::Mutex<String>,
        pub sent_messages: std::sync::Mutex<Vec<(SessionId, String)>>,
    }

    impl Default for MockSubagentDelegate {
        fn default() -> Self {
            Self::new()
        }
    }

    impl MockSubagentDelegate {
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
                    public_id: crate::typed_id::AgentId::new(),
                    internal_id: uuid::Uuid::now_v7(),
                    name: "test-agent".to_string(),
                    display_name: Some("Test Agent".to_string()),
                    description: Some("test agent".to_string()),
                    system_prompt: "You are helpful.".to_string(),
                    default_model_id: None,
                    harness_id: crate::typed_id::HarnessId::from_uuid(uuid::Uuid::nil()),
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

        #[allow(clippy::too_many_arguments)]
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
            if let Ok(mut sessions) = self.extra_sessions.lock() {
                sessions.insert(s.id, s.clone());
            }
            Ok(s)
        }
    }

    #[async_trait]
    impl SubagentSessionDelegate for MockSubagentDelegate {
        async fn get_agent_by_id(&self, _id: crate::typed_id::AgentId) -> Result<Option<Agent>> {
            Ok(Some(self.agent.clone()))
        }

        async fn add_agent_session_participant(
            &self,
            session_id: SessionId,
            agent_id: AgentId,
        ) -> Result<SessionParticipant> {
            let participant = SessionParticipant {
                id: crate::typed_id::SessionParticipantId::new(),
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

        async fn get_harness(&self, id: HarnessId) -> Result<Option<Harness>> {
            if let Some(harness) = self.extra_harnesses.lock().unwrap().get(&id).cloned() {
                return Ok(Some(harness));
            }
            Ok(Some(self.harness.clone()))
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
