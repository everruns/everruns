//! Base harness — empty, no capabilities. Blank canvas for custom configurations.

use everruns_core::{BuiltInHarnessDefinition, BuiltInHarnessRole};

pub fn definition() -> BuiltInHarnessDefinition {
    BuiltInHarnessDefinition::new(
        "base",
        "Base",
        "Empty harness with no capabilities. Provides a blank canvas for custom configurations.",
        "You are a helpful assistant.",
    )
    .with_seed_id(crate::org_init::BASE_HARNESS_ID)
    .with_tags(["base", "built-in"])
    .with_roles([BuiltInHarnessRole::Base])
}
