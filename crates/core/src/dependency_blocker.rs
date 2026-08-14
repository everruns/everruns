// Dependency blocker detection
//
// Decision: Shared module in core so both workers and activities don't
// duplicate harness/agent status checks and error messages.

use crate::error::Result;
use crate::typed_id::{AgentId, HarnessId};
use crate::{execution_loading::AgentStore, execution_loading::HarnessStore};

/// Reason why execution was blocked before it started.
#[derive(Debug, Clone, Copy)]
pub enum DependencyBlocker {
    HarnessArchived,
    HarnessDeleted,
    AgentArchived,
    AgentDeleted,
}

impl DependencyBlocker {
    /// User-facing message explaining why execution stopped.
    pub fn message(self) -> &'static str {
        match self {
            DependencyBlocker::HarnessArchived => {
                "Execution stopped because the assigned harness was archived."
            }
            DependencyBlocker::HarnessDeleted => {
                "Execution stopped because the assigned harness was deleted."
            }
            DependencyBlocker::AgentArchived => {
                "Execution stopped because the assigned agent was archived."
            }
            DependencyBlocker::AgentDeleted => {
                "Execution stopped because the assigned agent was deleted."
            }
        }
    }

    /// Error code for events.
    pub fn error_code(self) -> &'static str {
        "dependency_unavailable"
    }
}

/// Check if a harness/agent dependency is available for execution.
///
/// Returns `Some(blocker)` if execution should be blocked.
pub async fn detect_dependency_blocker(
    harness_store: &dyn HarnessStore,
    agent_store: &dyn AgentStore,
    harness_id: HarnessId,
    agent_id: Option<AgentId>,
) -> Result<Option<DependencyBlocker>> {
    // EVE-881: lifecycle status lives on the stored platform record, so hosted
    // stores answer the availability probe from their own status column instead
    // of exposing the record here.
    if let Some(blocker) = harness_store.get_harness_blocker(harness_id).await? {
        return Ok(Some(blocker));
    }

    if let Some(agent_id) = agent_id {
        // EVE-877: lifecycle status lives on the stored platform record, so
        // hosted stores answer the availability probe from their own status
        // column instead of exposing the record here.
        if let Some(blocker) = agent_store.get_agent_blocker(agent_id).await? {
            return Ok(Some(blocker));
        }
    }

    Ok(None)
}
