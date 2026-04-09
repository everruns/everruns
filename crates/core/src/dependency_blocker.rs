// Dependency blocker detection
//
// Decision: Shared module in core so both workers and activities don't
// duplicate harness/agent status checks and error messages.

use crate::error::Result;
use crate::traits::{AgentStore, HarnessStore};
use crate::typed_id::{AgentId, HarnessId};
use crate::{AgentStatus, HarnessStatus};

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
    let harness_chain = harness_store.get_harness_chain(harness_id).await?;
    match harness_chain.last() {
        Some(leaf) => match leaf.status {
            HarnessStatus::Active => {}
            HarnessStatus::Archived => return Ok(Some(DependencyBlocker::HarnessArchived)),
            HarnessStatus::Deleted => return Ok(Some(DependencyBlocker::HarnessDeleted)),
        },
        None => return Ok(Some(DependencyBlocker::HarnessDeleted)),
    }

    if let Some(agent_id) = agent_id {
        match agent_store.get_agent(agent_id).await? {
            Some(agent) => match agent.status {
                AgentStatus::Active => {}
                AgentStatus::Archived => return Ok(Some(DependencyBlocker::AgentArchived)),
                AgentStatus::Deleted => return Ok(Some(DependencyBlocker::AgentDeleted)),
            },
            None => return Ok(Some(DependencyBlocker::AgentDeleted)),
        }
    }

    Ok(None)
}
