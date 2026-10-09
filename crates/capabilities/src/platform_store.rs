// Platform Store trait: the hosted seam behind agent delegation.
//
// Decision: Trait lives in this crate; implementations in server/worker crates
// Decision: PlatformMessage is a simplified view (role + text + timestamp)
// Decision: scope is the catalog-backed `platform` command surface plus the
//   narrow session/agent/harness reads delegation needs. The org-scoped CRUD
//   half was retired with `platform_management` (EVE-953); management flows
//   go through the domain-command catalog instead.

use async_trait::async_trait;
use everruns_contracts::error::Result;
use everruns_contracts::typed_id::SessionParticipantId;
use everruns_core::{AgentDefinition, ExecutionSession, HarnessDefinition};

use everruns_contracts::typed_id::{AgentId, HarnessId, SessionId};

// The narrow session-delegation DTOs live in `everruns-core` (they are part of
// the `SubagentSessionDelegate` contract portable code depends on); the full
// `PlatformStore` reuses them. See EVE-839.
pub use everruns_core::subagent_delegation::{PlatformCreateSessionRequest, PlatformMessage};

/// Hosted command and delegation services, phrased entirely in runtime views.
///
/// Hosts resolve persistence records, inheritance, and authorization at their
/// edge. Capability code never needs a control-plane agent, harness, or session.
#[async_trait]
pub trait PlatformStore: Send + Sync {
    fn for_execution(
        &self,
        _input_message_id: uuid::Uuid,
    ) -> Option<std::sync::Arc<dyn PlatformStore>> {
        None
    }

    // =========================================================================
    // Catalog-backed command surface
    // =========================================================================

    /// Search the authoritative domain-command catalog.
    async fn platform_discover(&self, _arguments: serde_json::Value) -> Result<String> {
        Err(everruns_contracts::error::AgentLoopError::config(
            "Platform command surface is not available in this host",
        ))
    }

    /// Execute a bounded script against read-only domain commands.
    async fn platform_query(&self, _arguments: serde_json::Value) -> Result<String> {
        Err(everruns_contracts::error::AgentLoopError::config(
            "Platform command surface is not available in this host",
        ))
    }

    /// Execute a bounded script against the full authorized command catalog.
    async fn platform_execute(&self, _arguments: serde_json::Value) -> Result<String> {
        Err(everruns_contracts::error::AgentLoopError::config(
            "Platform command surface is not available in this host",
        ))
    }

    /// Run one command of the full catalog by wire name with structured
    /// params: `platform_execute` for a caller that already parsed the line.
    async fn platform_run(&self, _command: &str, _params: serde_json::Value) -> Result<String> {
        Err(everruns_contracts::error::AgentLoopError::config(
            "Platform command surface is not available in this host",
        ))
    }

    // =========================================================================
    // Harness Operations
    // =========================================================================

    /// Resolve the effective, inheritance-folded harness configuration by ID.
    async fn get_harness(&self, id: HarnessId) -> Result<Option<HarnessDefinition>>;

    // =========================================================================
    // Agent Operations
    // =========================================================================

    /// Get an agent by public ID.
    async fn get_agent_by_id(&self, id: AgentId) -> Result<Option<AgentDefinition>>;

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
    ) -> Result<ExecutionSession>;

    /// Get a session by ID.
    async fn get_session_by_id(&self, id: SessionId) -> Result<Option<ExecutionSession>>;

    /// Add an agent as a member participant in an existing session.
    async fn add_agent_session_participant(
        &self,
        session_id: SessionId,
        agent_id: AgentId,
    ) -> Result<SessionParticipantId>;

    // =========================================================================
    // Messaging
    // =========================================================================

    /// Send a user message to a session, triggering a turn.
    async fn send_message(&self, session_id: SessionId, content: &str) -> Result<()>;

    /// Get the conversation of a session (most recent first): user messages
    /// and what the agent said, as defined by
    /// [`everruns_core::conversation::said_text`]. Agent commentary and
    /// tool-only messages are left out. Default limit is 10.
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

    /// Harness the agent record is bound to. `None` when the host keeps no
    /// such binding.
    async fn get_agent_harness_id(&self, _id: AgentId) -> Result<Option<HarnessId>> {
        Ok(None)
    }

    /// Archive (`true`) or restore (`false`) a session.
    async fn set_session_archived(&self, _session_id: SessionId, _archived: bool) -> Result<()> {
        Err(everruns_contracts::error::AgentLoopError::config(
            "Archiving sessions is not available in this host",
        ))
    }
}

/// Typed [`ToolContext`](everruns_core::tool_context::ToolContext) extension carrying the
/// hosted `PlatformStore` (EVE-839). Replaces the former
/// `ToolContext::platform_store` field; hosted capabilities resolve it via
/// `context.extension::<PlatformStoreExt>()`.
#[derive(Clone)]
pub struct PlatformStoreExt(pub std::sync::Arc<dyn PlatformStore>);

/// Adapter implementing core's narrow
/// [`SubagentSessionDelegate`](everruns_core::subagent_delegation::SubagentSessionDelegate)
/// over a runtime-only `PlatformStore`, so hosted subagent/handoff orchestration can
/// drive child sessions without depending on the hosted seam.
pub struct PlatformStoreSubagentDelegate(pub std::sync::Arc<dyn PlatformStore>);

#[async_trait]
impl everruns_core::subagent_delegation::SubagentSessionDelegate for PlatformStoreSubagentDelegate {
    async fn get_agent_by_id(&self, id: AgentId) -> Result<Option<everruns_core::AgentDefinition>> {
        self.0.get_agent_by_id(id).await
    }
    async fn get_harness(&self, id: HarnessId) -> Result<Option<HarnessDefinition>> {
        self.0.get_harness(id).await
    }
    async fn add_agent_session_participant(
        &self,
        session_id: SessionId,
        agent_id: AgentId,
    ) -> Result<SessionParticipantId> {
        self.0
            .add_agent_session_participant(session_id, agent_id)
            .await
    }
    async fn create_session_with_options(
        &self,
        request: PlatformCreateSessionRequest,
    ) -> Result<ExecutionSession> {
        self.0.create_session_with_options(request).await
    }
    async fn get_session_by_id(&self, id: SessionId) -> Result<Option<ExecutionSession>> {
        self.0.get_session_by_id(id).await
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
    async fn get_agent_harness_id(&self, id: AgentId) -> Result<Option<HarnessId>> {
        self.0.get_agent_harness_id(id).await
    }
    async fn set_session_archived(&self, session_id: SessionId, archived: bool) -> Result<()> {
        self.0.set_session_archived(session_id, archived).await
    }
}

#[cfg(test)]
pub mod tests {
    use super::*;
    use everruns_contracts::CapabilityRef as AgentCapabilityConfig;

    /// Mock PlatformStore for unit tests.
    ///
    /// Shared across test modules so that any test exercising
    /// platform management tools (directly or via ActAtom) uses
    /// the same mock. This prevents wiring bugs where a tool is
    /// registered but the store is not passed through.
    pub struct MockPlatformStore {
        pub harness: HarnessDefinition,
        pub extra_harnesses:
            std::sync::Mutex<std::collections::HashMap<HarnessId, HarnessDefinition>>,
        pub agent: AgentDefinition,
        pub session: ExecutionSession,
        pub extra_sessions:
            std::sync::Mutex<std::collections::HashMap<SessionId, ExecutionSession>>,
        pub joined_participants: std::sync::Mutex<Vec<(SessionId, AgentId)>>,
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
                harness: HarnessDefinition {
                    capabilities: vec![AgentCapabilityConfig::new("session")],
                    ..HarnessDefinition::new("test-harness", "You are helpful.")
                },
                extra_harnesses: std::sync::Mutex::new(std::collections::HashMap::new()),
                agent: AgentDefinition {
                    display_name: Some("Test Agent".to_string()),
                    description: Some("test agent".to_string()),
                    ..AgentDefinition::new(AgentId::new(), "test-agent", "You are helpful.")
                },
                session: {
                    let id = SessionId::new();
                    let mut session = ExecutionSession::new(
                        id,
                        everruns_contracts::typed_id::WorkspaceId::from_uuid(id.uuid()),
                        HarnessId::new(),
                    );
                    session.title = Some("Test Session".to_string());
                    session.status = everruns_core::session::SessionExecutionState::Idle;
                    session
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

        async fn get_harness(&self, id: HarnessId) -> Result<Option<HarnessDefinition>> {
            if let Some(harness) = self.extra_harnesses.lock().unwrap().get(&id).cloned() {
                return Ok(Some(harness));
            }
            Ok(Some(self.harness.clone()))
        }
        async fn get_agent_by_id(
            &self,
            _id: everruns_contracts::typed_id::AgentId,
        ) -> Result<Option<AgentDefinition>> {
            Ok(Some(self.agent.clone()))
        }
        async fn create_session_with_options(
            &self,
            request: PlatformCreateSessionRequest,
        ) -> Result<ExecutionSession> {
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
        async fn add_agent_session_participant(
            &self,
            session_id: SessionId,
            agent_id: AgentId,
        ) -> Result<SessionParticipantId> {
            self.joined_participants
                .lock()
                .unwrap()
                .push((session_id, agent_id));
            Ok(SessionParticipantId::new())
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
