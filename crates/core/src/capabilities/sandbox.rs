//! Sandbox Capability - for sandboxed code execution (coming soon)

use super::{Capability, CapabilityId, CapabilityStatus};

/// Sandbox capability - for sandboxed code execution (coming soon)
pub struct SandboxCapability;

impl Capability for SandboxCapability {
    fn id(&self) -> &str {
        CapabilityId::SANDBOX
    }

    fn name(&self) -> &str {
        "Sandboxed Execution"
    }

    fn description(&self) -> &str {
        "Enables sandboxed code execution environment for running code safely."
    }

    fn status(&self) -> CapabilityStatus {
        CapabilityStatus::ComingSoon
    }

    fn icon(&self) -> Option<&str> {
        Some("box")
    }

    fn category(&self) -> Option<&str> {
        Some("Execution")
    }

    fn system_prompt_addition(&self) -> Option<&str> {
        Some(
            "You can execute code in a sandboxed environment. Use the execute_code tool to run code safely.",
        )
    }
}
