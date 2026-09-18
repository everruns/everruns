//! Coding (Daytona) harness — coding agent with Daytona cloud sandboxes.
//!
//! Inherits from Generic. Adds the `daytona` capability for real filesystem,
//! full process execution, and git integration, plus `github_scout` for
//! read-only GitHub repository exploration subagents.
//!
//! Behavior comes from the shared [`coding_prompt`](super::coding_prompt); what
//! the sandbox is and what it can do is derived from the bound target rather
//! than written here (EVE-1042).
//!
//! See `knowledge/harnesses/coding-daytona-harness.md` for design rationale.

use everruns_platform::{BuiltInCapabilityDefinition, BuiltInHarnessDefinition};
pub fn definition() -> BuiltInHarnessDefinition {
    BuiltInHarnessDefinition::new(
        "coding-daytona",
        "Coding (Daytona)",
        "Coding harness with Daytona cloud sandboxes. Provides real filesystem, full process execution, git integration, GitHub Scout subagents, and all Generic capabilities for software development tasks.",
        super::coding_prompt::CODING_SYSTEM_PROMPT,
    )
    .with_icon("daytona")
    .with_parent_name("generic")
    .with_tags(["coding", "daytona", "built-in"])
    .with_capabilities([
        BuiltInCapabilityDefinition::new("daytona"),
        BuiltInCapabilityDefinition::new("github_scout"),
    ])
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn coding_daytona_includes_github_scout() {
        let harness = definition();
        let capability_ids: Vec<&str> = harness
            .capabilities
            .iter()
            .map(|cap| cap.capability_id())
            .collect();

        assert!(capability_ids.contains(&"daytona"));
        assert!(capability_ids.contains(&"github_scout"));
    }
}
