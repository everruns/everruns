// Integration tests for SubagentCapability
//
// Direct subagent creation tools were retired in favor of the unified
// spawn_agent dispatcher; this file keeps capability metadata coverage.

use everruns_core::capabilities::{Capability, CapabilityStatus};
use everruns_platform::capabilities::SubagentCapability;

// =============================================================================
// 1. Capability registration
// =============================================================================

#[test]
fn test_subagent_capability_registration() {
    let cap = SubagentCapability;

    assert_eq!(cap.id(), "subagents");
    assert_eq!(cap.name(), "Subagents");
    assert_eq!(cap.status(), CapabilityStatus::Available);
    assert!(cap.tools().is_empty());
    assert!(cap.system_prompt_addition().is_some());
    assert_eq!(cap.features(), vec!["subagents"]);

    // Verify the system prompt instructs delegation (spawning subagents) and
    // mentions blueprints. Matches the prose wording of SUBAGENT_SYSTEM_PROMPT
    // ("Spawn subagents …"), which does not contain the literal tool name
    let prompt = cap.system_prompt_addition().unwrap();
    assert!(
        prompt.contains("Spawn subagents"),
        "prompt should instruct spawning subagents, got: {prompt}"
    );
    assert!(prompt.contains("blueprint"));
}

// =============================================================================
// Additional: capability metadata
// =============================================================================

#[test]
fn test_subagent_capability_metadata() {
    let cap = SubagentCapability;

    assert_eq!(cap.icon(), Some("git-branch"));
    assert_eq!(cap.category(), Some("Core"));
    assert_eq!(
        cap.description(),
        "Spawn and manage subagents for parallel task execution in isolated context windows."
    );
}
