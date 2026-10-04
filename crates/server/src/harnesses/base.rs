//! Base harness — empty, no capabilities. Blank canvas for custom configurations.

use everruns_platform::{BuiltInHarnessDefinition, BuiltInHarnessRole};
pub fn definition() -> BuiltInHarnessDefinition {
    BuiltInHarnessDefinition::new(
        "base",
        "Base",
        "A blank start. Add only the capabilities this agent should have.",
        "You are a helpful assistant.",
    )
    .with_icon("square-dashed")
    .with_tags(["base", "built-in"])
    .with_roles([BuiltInHarnessRole::Base])
}
