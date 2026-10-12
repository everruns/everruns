//! Worker that requires a full sandbox.
//!
//! Hosted only: the Framework expresses the same need as an Environment
//! requirement. It adds no capability of its own. The Agent's sandbox policy
//! supplies the shell (Daytona, E2B, Modal, a container), and Session creation
//! rejects it without one instead of silently falling back to Bashkit.

use crate::domains::harnesses::record::{BuiltInHarnessDefinition, HarnessExecution};

pub const NAME: &str = "sandbox-worker";

pub fn definition() -> BuiltInHarnessDefinition {
    BuiltInHarnessDefinition::new(
        NAME,
        "Sandbox Worker",
        "Worker with a full sandbox shell: real processes and packages. The Agent sandbox policy picks a container or managed provider.",
        "",
    )
    .with_icon("container")
    .with_parent_name("worker")
    .with_tags([NAME, "built-in", "environment-required"])
    .with_execution(HarnessExecution::FullSandbox)
}
