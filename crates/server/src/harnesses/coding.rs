//! Provider-neutral coding harness.

use crate::domains::harnesses::record::{BuiltInCapabilityDefinition, BuiltInHarnessDefinition};

pub fn definition() -> BuiltInHarnessDefinition {
    BuiltInHarnessDefinition::new(
        "coding",
        "Coding",
        "Coding harness for software development on a full sandbox. The Agent's sandbox policy selects Daytona, E2B, Modal or a container without changing tool names.",
        super::coding_prompt::CODING_SYSTEM_PROMPT,
    )
    .with_icon("terminal")
    .with_parent_name("sandbox-worker")
    .with_tags(["coding", "environment", "built-in"])
    .with_capabilities([BuiltInCapabilityDefinition::new("github_scout"), BuiltInCapabilityDefinition::new("session_tasks")])
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn coding_is_provider_neutral() {
        let harness = definition();
        let capability_ids = harness
            .capabilities
            .iter()
            .map(|capability| capability.capability_id())
            .collect::<Vec<_>>();

        assert_eq!(harness.parent_name.as_deref(), Some("sandbox-worker"));
        assert_eq!(capability_ids, vec!["github_scout", "session_tasks"]);
    }
}
