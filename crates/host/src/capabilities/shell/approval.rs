//! The seam a host fills to put a human in front of a command.
//!
//! Containment decides what a command may touch; this decides whether it runs
//! at all. The two are separate because only one of them can be answered
//! without a person: an unattended worker has nobody to ask, and the honest
//! outcome there is a refusal, never a quiet escalation.
//!
//! A host installs a gate through the tool-context extension seam:
//!
//! ```no_run
//! use std::sync::Arc;
//! use everruns_host::capabilities::shell::{HostShellApproval, ShellApprovalGate};
//!
//! # fn install(gate: Arc<dyn ShellApprovalGate>, extensions: &mut everruns_core::tool_context::ToolContextExtensions) {
//! extensions.insert(Arc::new(HostShellApproval::new(gate)));
//! # }
//! ```
//!
//! With no gate installed, every policy that would ask instead refuses. That is
//! the fail-closed half of the same rule the containment layer follows.

use std::sync::Arc;

use async_trait::async_trait;

/// One command waiting on a decision.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ShellApprovalRequest {
    /// The script as the model wrote it.
    pub command: String,
    /// Why approval is being asked for, in words a person can act on.
    pub reason: String,
    /// Whether saying yes also drops containment for this one command.
    ///
    /// Worth surfacing differently in a prompt: approving a command is not the
    /// same as approving it with the boundary removed.
    pub full_access: bool,
}

/// A host's answer to [`ShellApprovalRequest`].
#[async_trait]
pub trait ShellApprovalGate: Send + Sync {
    /// Whether the command may run. `false` for anything the host cannot ask
    /// about, including a prompt that times out or is dismissed.
    async fn approve(&self, request: ShellApprovalRequest) -> bool;
}

/// The extension a host inserts to supply a [`ShellApprovalGate`].
///
/// A concrete wrapper because the extension bag keys on a concrete type.
pub struct HostShellApproval(Arc<dyn ShellApprovalGate>);

impl HostShellApproval {
    /// Wrap `gate` for installation into the tool context.
    pub fn new(gate: Arc<dyn ShellApprovalGate>) -> Self {
        Self(gate)
    }

    /// The wrapped gate.
    pub fn gate(&self) -> &Arc<dyn ShellApprovalGate> {
        &self.0
    }
}

/// A gate that refuses everything, used when a host installed none.
///
/// THREAT[TM-BASH-022]: an unattended host has nobody to ask, so the absence of
/// a gate denies rather than escalates.
pub(crate) struct DenyAll;

#[async_trait]
impl ShellApprovalGate for DenyAll {
    async fn approve(&self, _request: ShellApprovalRequest) -> bool {
        false
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    struct AllowAll;

    #[async_trait]
    impl ShellApprovalGate for AllowAll {
        async fn approve(&self, _request: ShellApprovalRequest) -> bool {
            true
        }
    }

    #[tokio::test]
    async fn the_default_gate_refuses() {
        let request = ShellApprovalRequest {
            command: "rm -rf /".to_string(),
            reason: "outside the trusted set".to_string(),
            full_access: false,
        };
        assert!(!DenyAll.approve(request).await);
    }

    #[tokio::test]
    async fn an_installed_gate_is_reachable_through_the_extension() {
        let extension = HostShellApproval::new(Arc::new(AllowAll));
        let request = ShellApprovalRequest {
            command: "cargo build".to_string(),
            reason: "escalation requested".to_string(),
            full_access: true,
        };
        assert!(extension.gate().approve(request).await);
    }
}
