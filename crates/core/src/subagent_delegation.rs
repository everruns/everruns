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
use crate::session::{Session, SessionSeedMode};
use crate::typed_id::{AgentId, HarnessId, SessionId};
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

/// The narrow set of child-session operations portable subagent orchestration
/// needs. Implemented by the hosted platform adapter (over `PlatformStore`);
/// carried on [`ToolContext`](crate::ToolContext) as an optional service.
#[async_trait]
pub trait SubagentSessionDelegate: Send + Sync {
    /// Look up an agent by id (target validation for handoff/spawn).
    async fn get_agent_by_id(&self, id: AgentId) -> Result<Option<Agent>>;

    /// Look up a harness by id (parent harness resolution).
    async fn get_harness(&self, id: HarnessId) -> Result<Option<Harness>>;

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
