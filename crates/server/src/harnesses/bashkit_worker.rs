//! Managed Worker with a sealed Bashkit primary Sandbox.

use crate::records::BuiltInHarnessDefinition;

pub const NAME: &str = "bashkit-worker";

pub fn definition() -> BuiltInHarnessDefinition {
    BuiltInHarnessDefinition::new(
        NAME,
        "Bashkit Worker",
        "General-purpose Worker with a fixed recoverable Bashkit virtual workspace.",
        "",
    )
    .with_icon("terminal")
    .with_parent_name("worker")
    .with_tags([NAME, "built-in", "environment-fixed"])
}
