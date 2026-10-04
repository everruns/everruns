//! Provider-neutral coding harness.

use everruns_platform::{BuiltInCapabilityDefinition, BuiltInHarnessDefinition};

pub fn definition() -> BuiltInHarnessDefinition {
    BuiltInHarnessDefinition::new(
        "coding",
        "Coding",
        "Coding harness for software development. The Agent's Environment profile selects Bashkit, Daytona, or another execution target without changing behavior or tool names.",
        super::coding_prompt::CODING_SYSTEM_PROMPT,
    )
    .with_icon("terminal")
    .with_parent_name("worker-base")
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

        assert_eq!(harness.parent_name.as_deref(), Some("worker-base"));
        assert_eq!(capability_ids, vec!["github_scout", "session_tasks"]);
    }
}
