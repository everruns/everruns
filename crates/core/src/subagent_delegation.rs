//! Narrow, host-neutral session-delegation contract.
//!
//! Hosted delegation capabilities drive child sessions — create, message,
//! wait, read — through this narrow `everruns-core`-owned trait instead of the
//! full hosted [`PlatformStore`](https://docs.rs/everruns-platform) seam. The
//! platform crate implements it by delegating to `PlatformStore`, so core owns
//! only the execution contract while server/worker keep identical behavior.
//!
//! The request/message DTOs live here (not in `everruns-platform`) because the
//! trait signature needs them and core cannot depend on platform.

use crate::agent_definition::AgentDefinition;
use crate::error::Result;
use crate::harness_definition::HarnessDefinition;
use crate::session::{ExecutionSession, SessionSeedMode};
use crate::typed_id::{AgentId, HarnessId, SessionId, SessionParticipantId};
use async_trait::async_trait;
use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};

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

/// The narrow set of child-session operations a delegation provider needs.
/// Implemented by a host adapter and carried on [`ToolContext`](crate::ToolContext)
/// as an optional service.
#[async_trait]
pub trait SubagentSessionDelegate: Send + Sync {
    /// Look up an agent's execution definition by id (target validation for
    /// handoff/spawn). Stored persistence records stay behind the hosted
    /// platform adapter (EVE-877).
    async fn get_agent_by_id(&self, id: AgentId) -> Result<Option<AgentDefinition>>;

    /// Look up a harness's effective (inheritance-resolved) execution
    /// configuration by id. Parent-chain walking, cycle guarding, and the
    /// stored persistence record stay behind the hosted platform adapter
    /// (EVE-881).
    async fn get_harness(&self, id: HarnessId) -> Result<Option<HarnessDefinition>>;

    /// Add an agent as a member participant in an existing session.
    ///
    /// Returns the new participant row's id (a neutral correlation value);
    /// the stored participant record stays behind the platform seam (EVE-882).
    async fn add_agent_session_participant(
        &self,
        session_id: SessionId,
        agent_id: AgentId,
    ) -> Result<SessionParticipantId>;

    /// Create a child session with the given options. Returns the portable
    /// execution view of the new session (EVE-882).
    async fn create_session_with_options(
        &self,
        request: PlatformCreateSessionRequest,
    ) -> Result<ExecutionSession>;

    /// Get a session's portable execution view by id.
    async fn get_session_by_id(&self, id: SessionId) -> Result<Option<ExecutionSession>>;

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
    use crate::session::SessionExecutionState;

    /// Mock [`SubagentSessionDelegate`] for neutral delegation-contract tests.
    ///
    /// Carries the same simulated harness/agent/session state the former
    /// `MockPlatformStore` provided, restricted to the narrow delegate surface
    /// (EVE-839). The full hosted mock still lives in `everruns-platform`.
    pub struct MockSubagentDelegate {
        pub harness: HarnessDefinition,
        pub extra_harnesses:
            std::sync::Mutex<std::collections::HashMap<HarnessId, HarnessDefinition>>,
        pub agent: AgentDefinition,
        pub session: ExecutionSession,
        pub extra_sessions:
            std::sync::Mutex<std::collections::HashMap<SessionId, ExecutionSession>>,
        pub joined_participants: std::sync::Mutex<Vec<(SessionId, Option<AgentId>)>>,
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
                harness: HarnessDefinition {
                    capabilities: vec![AgentCapabilityConfig::new("session")],
                    ..HarnessDefinition::new("test-harness", "You are helpful.")
                },
                extra_harnesses: std::sync::Mutex::new(std::collections::HashMap::new()),
                agent: AgentDefinition {
                    display_name: Some("Test Agent".to_string()),
                    description: Some("test agent".to_string()),
                    ..AgentDefinition::new(
                        crate::typed_id::AgentId::new(),
                        "test-agent",
                        "You are helpful.",
                    )
                },
                session: ExecutionSession {
                    title: Some("Test Session".to_string()),
                    status: SessionExecutionState::Idle,
                    ..ExecutionSession::with_own_workspace(SessionId::new(), HarnessId::new())
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
        ) -> Result<ExecutionSession> {
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
        async fn get_agent_by_id(
            &self,
            _id: crate::typed_id::AgentId,
        ) -> Result<Option<AgentDefinition>> {
            Ok(Some(self.agent.clone()))
        }

        async fn add_agent_session_participant(
            &self,
            session_id: SessionId,
            agent_id: AgentId,
        ) -> Result<SessionParticipantId> {
            if let Ok(mut participants) = self.joined_participants.lock() {
                participants.push((session_id, Some(agent_id)));
            }
            Ok(SessionParticipantId::new())
        }

        async fn get_harness(&self, id: HarnessId) -> Result<Option<HarnessDefinition>> {
            if let Some(harness) = self.extra_harnesses.lock().unwrap().get(&id).cloned() {
                return Ok(Some(harness));
            }
            Ok(Some(self.harness.clone()))
        }

        async fn create_session_with_options(
            &self,
            request: PlatformCreateSessionRequest,
        ) -> Result<ExecutionSession> {
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

        async fn get_session_by_id(&self, id: SessionId) -> Result<Option<ExecutionSession>> {
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
