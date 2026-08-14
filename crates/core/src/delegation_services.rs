//! Neutral delegation governance and durable spawn contracts.

use crate::error::Result;
use crate::typed_id::SessionId;
use async_trait::async_trait;
use std::sync::Arc;

/// Default maximum child depth for subagent delegation. Top-level sessions are
/// depth 0, their children are depth 1, and grandchildren are depth 2.
pub const DEFAULT_MAX_SUBAGENT_DEPTH: u32 = 2;
pub const DEFAULT_MAX_ACTIVE_DESCENDANT_SUBAGENT_TASKS: u32 = 16;
pub const DEFAULT_MAX_TOTAL_DESCENDANT_SUBAGENT_TASKS: u32 = 200;
/// Governance for detached peer spawns (EVE-767): a detached spawn resets depth
/// (it is a peer, not a lifecycle child) but is still counted against the origin
/// subagent tree's root so a loop of `spawn_agent(lifetime=detached)` cannot run
/// unbounded (TM-DOS). Detached peers are full independent sessions, so the
/// default ceiling is tighter than the subagent descendant caps.
pub const DEFAULT_MAX_ACTIVE_DETACHED_TASKS: u32 = 8;
pub const DEFAULT_MAX_TOTAL_DETACHED_TASKS: u32 = 50;

/// Resolved subagent spawn governance policy for a tool execution context.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct SubagentNestingPolicy {
    pub platform_default: u32,
    pub org_override: Option<u32>,
    pub agent_override: Option<u32>,
    pub platform_default_max_active_descendant_tasks: u32,
    pub org_override_max_active_descendant_tasks: Option<u32>,
    pub agent_override_max_active_descendant_tasks: Option<u32>,
    pub platform_default_max_total_descendant_tasks: u32,
    pub org_override_max_total_descendant_tasks: Option<u32>,
    pub agent_override_max_total_descendant_tasks: Option<u32>,
    pub platform_default_max_active_detached_tasks: u32,
    pub org_override_max_active_detached_tasks: Option<u32>,
    pub agent_override_max_active_detached_tasks: Option<u32>,
    pub platform_default_max_total_detached_tasks: u32,
    pub org_override_max_total_detached_tasks: Option<u32>,
    pub agent_override_max_total_detached_tasks: Option<u32>,
}

impl Default for SubagentNestingPolicy {
    fn default() -> Self {
        Self {
            platform_default: DEFAULT_MAX_SUBAGENT_DEPTH,
            org_override: None,
            agent_override: None,
            platform_default_max_active_descendant_tasks:
                DEFAULT_MAX_ACTIVE_DESCENDANT_SUBAGENT_TASKS,
            org_override_max_active_descendant_tasks: None,
            agent_override_max_active_descendant_tasks: None,
            platform_default_max_total_descendant_tasks:
                DEFAULT_MAX_TOTAL_DESCENDANT_SUBAGENT_TASKS,
            org_override_max_total_descendant_tasks: None,
            agent_override_max_total_descendant_tasks: None,
            platform_default_max_active_detached_tasks: DEFAULT_MAX_ACTIVE_DETACHED_TASKS,
            org_override_max_active_detached_tasks: None,
            agent_override_max_active_detached_tasks: None,
            platform_default_max_total_detached_tasks: DEFAULT_MAX_TOTAL_DETACHED_TASKS,
            org_override_max_total_detached_tasks: None,
            agent_override_max_total_detached_tasks: None,
        }
    }
}

impl SubagentNestingPolicy {
    pub fn max_subagent_depth(self) -> u32 {
        self.agent_override
            .or(self.org_override)
            .unwrap_or(self.platform_default)
    }

    pub fn max_active_descendant_tasks(self) -> u32 {
        self.agent_override_max_active_descendant_tasks
            .or(self.org_override_max_active_descendant_tasks)
            .unwrap_or(self.platform_default_max_active_descendant_tasks)
    }

    pub fn max_total_descendant_tasks(self) -> u32 {
        self.agent_override_max_total_descendant_tasks
            .or(self.org_override_max_total_descendant_tasks)
            .unwrap_or(self.platform_default_max_total_descendant_tasks)
    }

    pub fn max_active_detached_tasks(self) -> u32 {
        self.agent_override_max_active_detached_tasks
            .or(self.org_override_max_active_detached_tasks)
            .unwrap_or(self.platform_default_max_active_detached_tasks)
    }

    pub fn max_total_detached_tasks(self) -> u32 {
        self.agent_override_max_total_detached_tasks
            .or(self.org_override_max_total_detached_tasks)
            .unwrap_or(self.platform_default_max_total_detached_tasks)
    }

    pub fn with_platform_default(mut self, depth: u32) -> Self {
        self.platform_default = depth;
        self
    }

    pub fn with_org_override(mut self, depth: Option<u32>) -> Self {
        self.org_override = depth;
        self
    }

    pub fn with_agent_override(mut self, depth: Option<u32>) -> Self {
        self.agent_override = depth;
        self
    }

    pub fn with_agent_task_caps_override(
        mut self,
        max_active: Option<u32>,
        max_total: Option<u32>,
    ) -> Self {
        self.agent_override_max_active_descendant_tasks = max_active;
        self.agent_override_max_total_descendant_tasks = max_total;
        self
    }

    pub fn with_agent_detached_task_caps_override(
        mut self,
        max_active: Option<u32>,
        max_total: Option<u32>,
    ) -> Self {
        self.agent_override_max_active_detached_tasks = max_active;
        self.agent_override_max_total_detached_tasks = max_total;
        self
    }
}

/// Host-provided authority for creating detached peer sessions.
///
/// The host resolves the current session owner and evaluates session-management
/// permission. Keeping this outside model-authored arguments prevents a tool
/// call from choosing or forging its authorization identity.
#[async_trait]
pub trait SessionCreationAuthority: Send + Sync {
    /// Authorize creation and return the org-validated budget root for the
    /// current session. Returning the root from the authority keeps detached
    /// chains linked without exposing internal root metadata to model input.
    async fn authorize_session_creation(&self, session_id: SessionId) -> Result<SessionId>;
}

/// Result of attempting to claim a subagent spawn slot.
#[derive(Debug)]
pub enum SpawnClaimResult {
    /// First claim — child session does not yet exist.
    /// Proceed to create the child, then call `register_child_session`.
    Claimed {
        spawn_handle_id: uuid::Uuid,
        claim_token: uuid::Uuid,
    },
    /// Row exists but `child_session_id` was never registered (crash between
    /// claim and `register_child_session`). Re-create the child and call
    /// `register_child_session` — same flow as `Claimed`.
    ClaimedPendingChild {
        spawn_handle_id: uuid::Uuid,
        claim_token: uuid::Uuid,
    },
    /// Child session was created and is still running.
    /// Reattach: wait for the existing child and settle with the stored claim_token.
    AlreadyRunning {
        child_session_id: crate::typed_id::SessionId,
        /// Stored claim token — must be used for `settle_spawn` on this replay.
        claim_token: uuid::Uuid,
    },
    /// Child already finished on a previous execution.
    /// Fast-path: return the stored result immediately without waiting.
    AlreadySettled {
        child_session_id: crate::typed_id::SessionId,
        /// The `wait_for_idle` return value from the original execution.
        terminal_status: String,
        terminal_result: String,
    },
}

/// Durable spawn handle store for subagent idempotency (EVE-535).
///
/// Maps `(parent_session_id, tool_call_id) → child_session_id` so that when
/// a parent's `act` is reclaimed mid-`wait_for_idle`, the tool can reattach
/// to the existing child instead of spawning a duplicate.
///
/// Lifecycle: claim → register_child_session → settle_spawn.
#[async_trait]
pub trait SubagentSpawnStore: Send + Sync + 'static {
    /// Attempt to claim a spawn slot for `(parent_session_id, tool_call_id)`.
    ///
    /// Does NOT accept `child_session_id` — the child session does not exist yet.
    /// Call `register_child_session` with the actual child ID after creating it.
    async fn try_claim_spawn(
        &self,
        parent_session_id: crate::typed_id::SessionId,
        tool_call_id: &str,
        claim_token: uuid::Uuid,
    ) -> Result<SpawnClaimResult>;

    /// Register the actual child session ID after it has been created.
    ///
    /// Must be called after `try_claim_spawn` returns `Claimed` or
    /// `ClaimedPendingChild`, before waiting for the child to complete.
    async fn register_child_session(
        &self,
        spawn_handle_id: uuid::Uuid,
        claim_token: uuid::Uuid,
        child_session_id: crate::typed_id::SessionId,
    ) -> Result<()>;

    /// Record the terminal result once the child has completed.
    ///
    /// `claim_token` must match the stored token. `terminal_status` is the
    /// `wait_for_idle` return value ("idle", "error", "timeout", etc.) and
    /// `terminal_result` is the last agent message.
    async fn settle_spawn(
        &self,
        parent_session_id: crate::typed_id::SessionId,
        tool_call_id: &str,
        claim_token: uuid::Uuid,
        terminal_status: &str,
        terminal_result: &str,
    ) -> Result<()>;
}

/// Blanket impl: `Arc<S>` delegates to the inner store.
#[async_trait]
impl<S: SubagentSpawnStore + ?Sized> SubagentSpawnStore for Arc<S> {
    async fn try_claim_spawn(
        &self,
        parent_session_id: crate::typed_id::SessionId,
        tool_call_id: &str,
        claim_token: uuid::Uuid,
    ) -> Result<SpawnClaimResult> {
        (**self)
            .try_claim_spawn(parent_session_id, tool_call_id, claim_token)
            .await
    }

    async fn register_child_session(
        &self,
        spawn_handle_id: uuid::Uuid,
        claim_token: uuid::Uuid,
        child_session_id: crate::typed_id::SessionId,
    ) -> Result<()> {
        (**self)
            .register_child_session(spawn_handle_id, claim_token, child_session_id)
            .await
    }

    async fn settle_spawn(
        &self,
        parent_session_id: crate::typed_id::SessionId,
        tool_call_id: &str,
        claim_token: uuid::Uuid,
        terminal_status: &str,
        terminal_result: &str,
    ) -> Result<()> {
        (**self)
            .settle_spawn(
                parent_session_id,
                tool_call_id,
                claim_token,
                terminal_status,
                terminal_result,
            )
            .await
    }
}
