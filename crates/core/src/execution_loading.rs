//! Neutral contracts for loading turn execution inputs.

use crate::agent_definition::AgentDefinition;
use crate::error::Result;
use crate::harness_definition::HarnessDefinition;
use crate::session::ExecutionSession;
use crate::typed_id::{AgentId, HarnessId, SessionId};
use async_trait::async_trait;

/// Narrow agent-loading seam for turn execution (EVE-872, EVE-877).
///
/// Implementations project their stored agent records into the portable
/// [`AgentDefinition`] — the authored execution configuration. Stored
/// `Agent`/`AgentVersion` persistence records live in `everruns-platform` and
/// never cross this boundary.
///
/// Contract: records that exist but cannot execute (archived or deleted) must
/// yield an error from [`AgentStore::get_agent`], so lifecycle validation is
/// enforced at the loading seam before host execution begins.
#[async_trait]
pub trait AgentStore: Send + Sync {
    /// Get the portable execution definition for an agent by public id.
    async fn get_agent(&self, agent_id: AgentId) -> Result<Option<AgentDefinition>>;

    /// Execution-availability probe for dependency-blocker detection.
    ///
    /// Returns `None` when the agent exists and can execute. Hosted stores
    /// override this to report [`DependencyBlocker::AgentArchived`] /
    /// [`DependencyBlocker::AgentDeleted`] from the stored lifecycle status;
    /// the default treats a missing record as deleted.
    async fn get_agent_blocker(
        &self,
        agent_id: AgentId,
    ) -> Result<Option<crate::dependency_blocker::DependencyBlocker>> {
        Ok(match self.get_agent(agent_id).await? {
            Some(_) => None,
            None => Some(crate::dependency_blocker::DependencyBlocker::AgentDeleted),
        })
    }
}

#[async_trait]
impl<T: AgentStore + ?Sized> AgentStore for std::sync::Arc<T> {
    async fn get_agent(&self, agent_id: AgentId) -> Result<Option<AgentDefinition>> {
        (**self).get_agent(agent_id).await
    }

    async fn get_agent_blocker(
        &self,
        agent_id: AgentId,
    ) -> Result<Option<crate::dependency_blocker::DependencyBlocker>> {
        (**self).get_agent_blocker(agent_id).await
    }
}

// ============================================================================
// HarnessStore - For retrieving harness execution configurations
// ============================================================================

/// Narrow harness-loading seam for turn execution (EVE-872, EVE-881).
///
/// Implementations project their stored harness records into the portable
/// [`HarnessDefinition`] — the effective execution environment configuration.
/// Parent-chain inheritance is resolved *behind* this seam (root-to-leaf, via
/// the same overlay merge semantics the runtime uses), so callers receive one
/// effective layer. Stored `Harness` persistence records live in
/// `everruns-platform` and never cross this boundary.
///
/// Contract: records that exist but cannot execute (archived or deleted) must
/// yield an error from [`HarnessStore::get_harness`], so lifecycle validation
/// is enforced at the loading seam before host execution begins.
#[async_trait]
pub trait HarnessStore: Send + Sync {
    /// Get the effective (inheritance-resolved) execution definition for a
    /// harness by id. Returns `Ok(None)` if the harness does not exist.
    async fn get_harness(&self, harness_id: HarnessId) -> Result<Option<HarnessDefinition>>;

    /// Execution-availability probe for dependency-blocker detection.
    ///
    /// Returns `None` when the harness exists and can execute. Hosted stores
    /// override this to report [`DependencyBlocker::HarnessArchived`] /
    /// [`DependencyBlocker::HarnessDeleted`] from the stored lifecycle status;
    /// the default treats a missing record as deleted.
    ///
    /// [`DependencyBlocker::HarnessArchived`]: crate::dependency_blocker::DependencyBlocker::HarnessArchived
    /// [`DependencyBlocker::HarnessDeleted`]: crate::dependency_blocker::DependencyBlocker::HarnessDeleted
    async fn get_harness_blocker(
        &self,
        harness_id: HarnessId,
    ) -> Result<Option<crate::dependency_blocker::DependencyBlocker>> {
        Ok(match self.get_harness(harness_id).await? {
            Some(_) => None,
            None => Some(crate::dependency_blocker::DependencyBlocker::HarnessDeleted),
        })
    }
}

#[async_trait]
impl<T: HarnessStore + ?Sized> HarnessStore for std::sync::Arc<T> {
    async fn get_harness(&self, harness_id: HarnessId) -> Result<Option<HarnessDefinition>> {
        (**self).get_harness(harness_id).await
    }

    async fn get_harness_blocker(
        &self,
        harness_id: HarnessId,
    ) -> Result<Option<crate::dependency_blocker::DependencyBlocker>> {
        (**self).get_harness_blocker(harness_id).await
    }
}

// ============================================================================
// SessionStore - For retrieving session information
// ============================================================================

/// Narrow session-loading seam for turn execution (EVE-872, EVE-882).
///
/// Implementations project their stored session records into the portable
/// [`ExecutionSession`] — the session correlation values, per-session
/// configuration layer, and neutral execution state a turn consumes. The
/// persisted `Session` aggregate (facets, participants, ownership summaries,
/// timestamps, UI metadata) lives in `everruns-platform` and never crosses
/// this boundary.
#[async_trait]
pub trait SessionStore: Send + Sync {
    /// Get the portable execution view for a session by ID.
    async fn get_session(&self, session_id: SessionId) -> Result<Option<ExecutionSession>>;
}

#[async_trait]
impl<T: SessionStore + ?Sized> SessionStore for std::sync::Arc<T> {
    async fn get_session(&self, session_id: SessionId) -> Result<Option<ExecutionSession>> {
        (**self).get_session(session_id).await
    }
}

// EVE-897: `SessionMutator` moved to `everruns-platform`. Mutating stored
// session metadata is a hosted control-plane service; the capability that
// uses it resolves `SessionMutatorExt` from the typed extension bag.
