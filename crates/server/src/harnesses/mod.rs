//! Built-in harness definitions.
//!
//! Each submodule defines one built-in harness (system prompt, capabilities,
//! tags, roles). The top-level `built_in_harnesses()` function collects them
//! into the ordered list consumed by `oss_built_in_harnesses()` in platform.rs.

mod base;
mod coding_container;
mod coding_daytona;
mod coding_session_sandbox;
mod data_analyst;
mod generic;
mod platform_chat;

use everruns_core::BuiltInHarnessDefinition;

/// All built-in harness definitions in provisioning order.
pub fn built_in_harnesses() -> Vec<BuiltInHarnessDefinition> {
    let internal_flags = everruns_core::InternalFeatureFlags::from_env();
    let mut harnesses = vec![
        base::definition(),
        generic::definition(),
        data_analyst::definition(),
        coding_daytona::definition(),
    ];
    if internal_flags.container_sandbox {
        harnesses.push(coding_container::definition());
    }
    harnesses.push(platform_chat::definition());
    if internal_flags.session_sandbox {
        harnesses.push(coding_session_sandbox::definition());
    }
    harnesses
}
