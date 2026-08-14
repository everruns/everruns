// Dependency blocker detection
//
// The neutral reason value stays in core. Store-backed lifecycle probes live
// in everruns-host.

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
