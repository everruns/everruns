//! Built-in harness definitions.
//!
//! Each submodule defines one built-in harness (system prompt, capabilities,
//! tags, roles). The top-level `built_in_harnesses()` function collects them
//! into the ordered list consumed by `oss_built_in_harnesses()` in platform.rs.

mod base;
mod coding_container;
mod coding_daytona;
mod generic;
mod platform_chat;

use everruns_core::BuiltInHarnessDefinition;

/// All built-in harness definitions in provisioning order.
pub fn built_in_harnesses() -> Vec<BuiltInHarnessDefinition> {
    vec![
        base::definition(),
        generic::definition(),
        coding_daytona::definition(),
        coding_container::definition(),
        platform_chat::definition(),
    ]
}
