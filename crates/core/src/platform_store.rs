// Platform Store trait for org-scoped management operations
//
// Decision: Single trait covers harness, agent, session CRUD + messaging
// Decision: Trait lives in core; implementations in server/worker crates
// Decision: Tool results include UI links via base_url()
// Decision: PlatformMessage is a simplified view (role + text + timestamp)

use crate::agent::Agent;
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
        capabilities: &[String],
    ) -> Result<Harness>;

    /// Update a harness (only provided fields are changed).
    async fn update_harness(
        &self,
        id: HarnessId,
        name: Option<&str>,
        description: Option<&str>,
        system_prompt: Option<&str>,
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
    // UI Links
    // =========================================================================

    /// Base URL for constructing UI links (e.g., "http://localhost:3000").
    fn base_url(&self) -> &str;
}
