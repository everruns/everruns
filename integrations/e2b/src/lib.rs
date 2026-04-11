//! E2B Integration
//!
//! Cloud sandboxes backed by E2B. This integration intentionally uses a
//! platform-owned API key (`E2B_API_KEY`) from the server/worker environment
//! instead of user connections because E2B sandboxes are provisioned as shared
//! platform infrastructure, not per-user third-party accounts.
//! Decision: external integration crate, auto-registered via inventory.
//! Decision: global env-backed API key with optional per-session secret override.
//! Decision: session-scoped sandbox state + leased-resource cleanup, similar to Daytona.

pub mod client;
pub mod state;
mod tools;

use everruns_core::LEASED_RESOURCES_FEATURE;
use everruns_core::capabilities::{Capability, CapabilityStatus, IntegrationPlugin, RiskLevel};
use everruns_core::tools::Tool;

use std::sync::LazyLock;

use tools::{
    E2BCreateSandboxTool, E2BExecTool, E2BListSandboxesTool, E2BManageSandboxTool, E2BReadFileTool,
    E2BWriteFileTool,
};

inventory::submit! {
    IntegrationPlugin {
        experimental_only: false,
        feature_flag: None,
        factory: || Box::new(E2BCapability),
    }
}

pub const E2B_API_BASE: &str = "https://api.e2b.app";
pub const E2B_API_KEY_SECRET: &str = "E2B_API_KEY";
pub const E2B_SANDBOX_SECRET_PREFIX: &str = "e2b_sandbox:";
pub const E2B_DEFAULT_TEMPLATE: &str = "base";
pub const E2B_DEFAULT_TIMEOUT_SECS: u64 = 60 * 60;
pub const E2B_DEFAULT_WORKSPACE_PATH: &str = "/home/user";
pub const E2B_ENVD_PORT: u16 = 49_983;

static SYSTEM_PROMPT: LazyLock<String> = LazyLock::new(|| {
    let mut prompt = String::from(
        r#"## E2B

Cloud sandboxes via E2B. Each sandbox is an isolated Linux environment with network access.

Use `e2b_create_sandbox` first, then `e2b_exec`, `e2b_read_file`, and `e2b_write_file` against the returned `sandbox_id`.
Prefer `/home/user` as the workspace root.
Always pause or delete sandboxes when done. Deleted sandboxes are cleaned up immediately; paused sandboxes may still consume platform quota until resumed or deleted."#,
    );
    prompt.push_str(everruns_core::tool_output_sanitizer::EXEC_OUTPUT_HINT);
    prompt
});

pub struct E2BCapability;

impl Capability for E2BCapability {
    fn id(&self) -> &str {
        "e2b"
    }

    fn name(&self) -> &str {
        "E2B"
    }

    fn description(&self) -> &str {
        "Run code in cloud sandboxes powered by E2B. Create isolated Linux environments, execute commands, and manage sandbox files."
    }

    fn status(&self) -> CapabilityStatus {
        CapabilityStatus::Available
    }

    fn risk_level(&self) -> RiskLevel {
        RiskLevel::High
    }

    fn icon(&self) -> Option<&str> {
        Some("cloud")
    }

    fn category(&self) -> Option<&str> {
        Some("Execution")
    }

    fn system_prompt_addition(&self) -> Option<&str> {
        Some(&SYSTEM_PROMPT)
    }

    fn tools(&self) -> Vec<Box<dyn Tool>> {
        vec![
            Box::new(E2BCreateSandboxTool),
            Box::new(E2BExecTool),
            Box::new(E2BReadFileTool),
            Box::new(E2BWriteFileTool),
            Box::new(E2BListSandboxesTool),
            Box::new(E2BManageSandboxTool),
        ]
    }

    fn dependencies(&self) -> Vec<&'static str> {
        vec!["session_storage"]
    }

    fn features(&self) -> Vec<&'static str> {
        vec![LEASED_RESOURCES_FEATURE]
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn capability_metadata() {
        let cap = E2BCapability;
        assert_eq!(cap.id(), "e2b");
        assert_eq!(cap.name(), "E2B");
        assert_eq!(cap.icon(), Some("cloud"));
        assert_eq!(cap.category(), Some("Execution"));
        assert_eq!(cap.dependencies(), vec!["session_storage"]);
    }

    #[test]
    fn capability_tools() {
        let cap = E2BCapability;
        let names: Vec<_> = cap
            .tools()
            .into_iter()
            .map(|tool| tool.name().to_string())
            .collect();
        assert!(names.contains(&"e2b_create_sandbox".to_string()));
        assert!(names.contains(&"e2b_exec".to_string()));
        assert!(names.contains(&"e2b_read_file".to_string()));
        assert!(names.contains(&"e2b_write_file".to_string()));
        assert!(names.contains(&"e2b_list_sandboxes".to_string()));
        assert!(names.contains(&"e2b_manage_sandbox".to_string()));
    }
}
