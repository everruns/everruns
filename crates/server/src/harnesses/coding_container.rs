//! Coding (Container) harness — coding agent with self-hosted container sandboxes.
//!
//! Inherits from Generic. Adds the `container_sandbox` capability for real
//! filesystem, full process execution, and network access via Docker Engine,
//! plus `github_scout` for read-only GitHub repository exploration subagents.
//!
//! Behavior comes from the shared [`coding_prompt`](super::coding_prompt); what
//! the container is and what it can do is derived from the bound target rather
//! than written here (EVE-1042).
//!
//! See EVE-279 for design rationale.

use everruns_platform::{BuiltInCapabilityDefinition, BuiltInHarnessDefinition};
pub fn definition() -> BuiltInHarnessDefinition {
    BuiltInHarnessDefinition::new(
        "coding-container",
        "Coding (Container)",
        "Coding harness with self-hosted container sandboxes. Provides real filesystem, full process execution, network access, GitHub Scout subagents, and all Generic capabilities for software development tasks.",
        super::coding_prompt::CODING_SYSTEM_PROMPT,
    )
    .with_icon("container")
    .with_parent_name("generic")
    .with_tags(["coding", "container", "built-in"])
    .with_capabilities([
        BuiltInCapabilityDefinition::new("container_sandbox"),
        BuiltInCapabilityDefinition::new("github_scout"),
    ])
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn coding_container_includes_github_scout() {
        let harness = definition();
        let capability_ids: Vec<&str> = harness
            .capabilities
            .iter()
            .map(|cap| cap.capability_id())
            .collect();

        assert!(capability_ids.contains(&"container_sandbox"));
        assert!(capability_ids.contains(&"github_scout"));
    }
}
