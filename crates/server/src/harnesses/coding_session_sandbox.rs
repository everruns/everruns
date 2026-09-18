//! Coding (Session Sandbox) harness.
//!
//! Inherits from Generic. Adds the feature-flagged `session_sandbox` capability
//! configured to use Daytona as the managed provider.
//!
//! Behavior comes from the shared [`coding_prompt`](super::coding_prompt); what
//! the sandbox is and what it can do is derived from the bound target rather
//! than written here (EVE-1042).

use everruns_platform::{BuiltInCapabilityDefinition, BuiltInHarnessDefinition};
pub fn definition() -> BuiltInHarnessDefinition {
    BuiltInHarnessDefinition::new(
        "coding-session-sandbox",
        "Coding (Session Sandbox)",
        "Coding harness with one managed session-owned sandbox. Uses provider-neutral sandbox tools backed by Daytona and intentionally omits a local shell.",
        super::coding_prompt::CODING_SYSTEM_PROMPT,
    )
    .with_icon("terminal")
    .with_tags(["coding", "sandbox", "managed", "built-in"])
    .with_capabilities([
        BuiltInCapabilityDefinition::new("session_file_system"),
        BuiltInCapabilityDefinition::with_config(
            "web_fetch",
            serde_json::json!({"enable_file_download": true}),
        ),
        BuiltInCapabilityDefinition::new("session_storage"),
        BuiltInCapabilityDefinition::new("session"),
        BuiltInCapabilityDefinition::new("session_schedule"),
        BuiltInCapabilityDefinition::new("agent_instructions"),
        BuiltInCapabilityDefinition::new("skills"),
        BuiltInCapabilityDefinition::new("infinity_context"),
        BuiltInCapabilityDefinition::new("openai_tool_search"),
        BuiltInCapabilityDefinition::new("budgeting"),
        BuiltInCapabilityDefinition::new("self_budget"),
        BuiltInCapabilityDefinition::with_config(
            "parallel_tool_calls",
            serde_json::json!({"mode": "prefer"}),
        ),
        BuiltInCapabilityDefinition::with_config(
            "compaction",
            serde_json::json!({
                "strategy": "auto",
                "proactive": true,
                "budget_percent": 0.85
            }),
        ),
        BuiltInCapabilityDefinition::new("tool_output_persistence"),
        BuiltInCapabilityDefinition::with_config(
            "session_sandbox",
            serde_json::json!({
                "provider": "daytona",
                "auto_start": true,
                "idle_pause_after_seconds": 180,
                "provider_config": {
                    "size": "small",
                    "workspace_path": "/home/daytona/workspace",
                    "recovery": {
                        "enabled": true,
                        "volume_name": "everruns-recovery"
                    }
                }
            }),
        ),
    ])
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn coding_session_sandbox_harness_omits_bashkit_shell() {
        let harness = definition();
        let capability_ids: Vec<&str> = harness
            .capabilities
            .iter()
            .map(|cap| cap.capability_id())
            .collect();

        assert_eq!(harness.parent_name, None);
        assert!(capability_ids.contains(&"session_sandbox"));
        assert!(capability_ids.contains(&"session_file_system"));
        assert!(!capability_ids.contains(&"bashkit_shell"));
    }
}
