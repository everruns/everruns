//! Container Sandbox — self-hosted container execution via Docker Engine REST API.
//!
//! Decision: Core crate (not integration) because container execution is infrastructure,
//! not an external service. Same pattern as `crates/session-sqldb/`.
//!
//! Decision: Docker Engine REST API directly, no `docker` CLI binary dependency.
//! Workers can be containerized themselves — talking to the Docker Engine API
//! over TCP/Unix socket removes the need for Docker-in-Docker.
//!
//! Decision: One sandbox per session (container name derived from session ID).
//! Multi-sandbox per session can be added later if needed.

pub mod client;
pub mod config;
pub mod state;
mod tools;

use everruns_core::LEASED_RESOURCES_FEATURE;
use everruns_core::capabilities::{Capability, CapabilityStatus, IntegrationPlugin, RiskLevel};
use everruns_core::tools::Tool;
use std::sync::LazyLock;

use tools::{
    SandboxCreateTool, SandboxDownloadTool, SandboxExecTool, SandboxListTool, SandboxManageTool,
    SandboxReadFileTool, SandboxUploadTool, SandboxWriteFileTool,
};

// ============================================================================
// Plugin Registration
// ============================================================================

inventory::submit! {
    IntegrationPlugin {
        experimental_only: false,
        feature_flag: Some("container_sandbox"),
        factory: || Box::new(ContainerSandboxCapability),
    }
}

// ============================================================================
// System Prompt
// ============================================================================

static SYSTEM_PROMPT: LazyLock<String> = LazyLock::new(|| {
    let mut prompt = String::from(
        r#"## Container Sandbox

Self-hosted container execution via Docker Engine. Each session gets an isolated
container with full Linux environment and configurable resource limits.

Lifecycle: sandbox_create → sandbox_exec / sandbox_read_file / sandbox_write_file → sandbox_manage(remove)

Working directory: /workspace (default). Use sandbox_exec for shell commands,
sandbox_read_file/sandbox_write_file for file I/O, and sandbox_upload/sandbox_download
for transferring files between session storage and the container.

Always remove sandboxes when done to free Docker resources."#,
    );
    prompt.push_str(everruns_core::tool_output_sanitizer::EXEC_OUTPUT_HINT);
    prompt
});

// ============================================================================
// ContainerSandboxCapability
// ============================================================================

pub struct ContainerSandboxCapability;

#[async_trait::async_trait]
impl Capability for ContainerSandboxCapability {
    fn id(&self) -> &str {
        "container_sandbox"
    }

    fn name(&self) -> &str {
        "Container Sandbox"
    }

    fn description(&self) -> &str {
        "Run code in self-hosted Docker containers. Create isolated Linux environments \
         with configurable resource limits, execute commands, and manage files."
    }

    fn status(&self) -> CapabilityStatus {
        CapabilityStatus::Available
    }

    fn risk_level(&self) -> RiskLevel {
        RiskLevel::High
    }

    fn icon(&self) -> Option<&str> {
        Some("container")
    }

    fn category(&self) -> Option<&str> {
        Some("Execution")
    }

    fn system_prompt_addition(&self) -> Option<&str> {
        Some(&SYSTEM_PROMPT)
    }

    fn tools(&self) -> Vec<Box<dyn Tool>> {
        vec![
            Box::new(SandboxCreateTool),
            Box::new(SandboxExecTool),
            Box::new(SandboxReadFileTool),
            Box::new(SandboxWriteFileTool),
            Box::new(SandboxUploadTool),
            Box::new(SandboxDownloadTool),
            Box::new(SandboxListTool),
            Box::new(SandboxManageTool),
        ]
    }

    fn dependencies(&self) -> Vec<&'static str> {
        vec!["session_storage"]
    }

    fn features(&self) -> Vec<&'static str> {
        vec![LEASED_RESOURCES_FEATURE]
    }
}

// ============================================================================
// Tests
// ============================================================================

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_capability_metadata() {
        let cap = ContainerSandboxCapability;
        assert_eq!(cap.id(), "container_sandbox");
        assert_eq!(cap.name(), "Container Sandbox");
        assert_eq!(cap.status(), CapabilityStatus::Available);
        assert_eq!(cap.risk_level(), RiskLevel::High);
        assert_eq!(cap.icon(), Some("container"));
        assert_eq!(cap.category(), Some("Execution"));
    }

    #[test]
    fn test_capability_has_all_tools() {
        let cap = ContainerSandboxCapability;
        let tools = cap.tools();
        assert_eq!(tools.len(), 8);
        let names: Vec<&str> = tools.iter().map(|t| t.name()).collect();
        assert!(names.contains(&"sandbox_create"));
        assert!(names.contains(&"sandbox_exec"));
        assert!(names.contains(&"sandbox_read_file"));
        assert!(names.contains(&"sandbox_write_file"));
        assert!(names.contains(&"sandbox_upload"));
        assert!(names.contains(&"sandbox_download"));
        assert!(names.contains(&"sandbox_list"));
        assert!(names.contains(&"sandbox_manage"));
    }

    #[test]
    fn test_system_prompt_exists() {
        let cap = ContainerSandboxCapability;
        let prompt = cap.system_prompt_addition().unwrap();
        assert!(prompt.contains("Container Sandbox"));
        assert!(prompt.contains("/workspace"));
    }

    #[test]
    fn test_dependencies() {
        let cap = ContainerSandboxCapability;
        assert_eq!(cap.dependencies(), vec!["session_storage"]);
    }

    #[test]
    fn test_features() {
        let cap = ContainerSandboxCapability;
        assert!(cap.features().contains(&LEASED_RESOURCES_FEATURE));
    }
}
