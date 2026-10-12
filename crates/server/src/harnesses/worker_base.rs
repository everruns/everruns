//! Deprecated Worker Base, retained for existing bindings.
//!
//! The level was retired when the tree split workers by environment. The row
//! keeps its stable name, ID, parent and capabilities so agents, sessions and
//! adopted examples bound to it keep working. New work picks Worker, Bashkit
//! Worker or Sandbox Worker.

use crate::domains::harnesses::record::BuiltInHarnessDefinition;

pub fn definition() -> BuiltInHarnessDefinition {
    BuiltInHarnessDefinition::new(
        "worker-base",
        "Worker Base — deprecated",
        "Deprecated, kept for existing agents. Choose Worker for no shell, Bashkit Worker for a Bashkit shell, or Sandbox Worker for a full sandbox.",
        "",
    )
    .with_icon("terminal")
    .with_parent_name("conversation")
    .with_tags(["worker-base", "deprecated", "built-in"])
    .with_capabilities(everruns_contracts::capability::legacy_worker_base_capabilities())
}
