//! Container Sandbox — self-hosted container execution via Docker Engine REST API.
//!
//! This platform module owns deployment infrastructure rather than an external
//! service integration.
//!
//! Decision: Docker Engine REST API directly, no `docker` CLI binary dependency.
//! Workers can be containerized themselves — talking to the Docker Engine API
//! over HTTP/TCP removes the need for Docker-in-Docker.
//!
//! Decision: One sandbox per session (container name derived from session ID).
//! Multi-sandbox per session can be added later if needed.

pub mod client;
pub mod config;
pub mod state;
mod tools;

use everruns_core::LEASED_RESOURCES_FEATURE;
use everruns_core::capabilities::{
    Capability, CapabilityLocalization, CapabilityStatus, IntegrationPlugin, RiskLevel,
};
use everruns_core::tools::Tool;
use everruns_host::compute::{
    ComputeCapabilities, ComputeKind, Containment, Durability, NetworkPolicy,
};
use everruns_host::environment_preamble::{EnvironmentFacts, environment_preamble};
use std::sync::LazyLock;

use tools::{
    SandboxCreateTool, SandboxDownloadTool, SandboxExecTool, SandboxListTool, SandboxManageTool,
    SandboxReadFileTool, SandboxUploadTool, SandboxWriteFileTool,
};

// ============================================================================
// Plugin Registration
// ============================================================================

/// Capability plugins this module contributes to the hosted catalog.
pub const CAPABILITY_PLUGINS: &[IntegrationPlugin] = &[IntegrationPlugin {
    experimental_only: false,
    feature_flag: Some("container_sandbox"),
    factory: || Box::new(ContainerSandboxCapability),
}];

// ============================================================================
// System Prompt
// ============================================================================

/// The facts this capability's sandboxes actually have (EVE-1042).
///
/// Stated once, here, and rendered into prose by the shared derivation rather
/// than described by hand. A sentence written about the sandbox can disagree
/// with the sandbox; a sentence derived from these cannot.
fn environment_facts() -> EnvironmentFacts {
    EnvironmentFacts {
        kind: Some(ComputeKind::Container),
        capabilities: ComputeCapabilities::full_machine(),
        containment: Containment::isolated().network(NetworkPolicy::Allow),
        // A container is removed when the session is done and nothing archives
        // its filesystem, so nothing recovers it.
        durability: Durability::None,
    }
}

static SYSTEM_PROMPT: LazyLock<String> = LazyLock::new(|| {
    // The environment half is derived; what remains is this capability's own
    // lifecycle and tool guidance, which is the tool contract rather than a
    // description of the world and so is correctly written here.
    let mut prompt = String::from("## Container Sandbox\n\n");
    if let Some(preamble) = environment_preamble(&environment_facts()) {
        // The derivation emits its own "## Environment" heading; nested under
        // the capability block it reads as a subsection.
        prompt.push_str(&preamble);
        prompt.push_str("\n\n");
    }
    prompt.push_str(
        r#"Lifecycle: sandbox_create → sandbox_exec / sandbox_read_file / sandbox_write_file → sandbox_manage(remove)

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

    fn localizations(&self) -> Vec<CapabilityLocalization> {
        vec![CapabilityLocalization::text(
            "uk",
            "Контейнерна пісочниця",
            "Запускайте код у самостійно розгорнутих контейнерах Docker. Створюйте ізольовані \
             середовища Linux з налаштовуваними лімітами ресурсів, виконуйте команди та \
             керуйте файлами.",
        )]
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

    /// EVE-1042: the environment half of this prompt is derived from the facts
    /// above, so it cannot drift from what the sandbox actually is.
    #[test]
    fn the_prompt_describes_the_environment_from_its_facts() {
        let prompt = ContainerSandboxCapability
            .system_prompt_addition()
            .expect("a prompt");
        let derived = environment_preamble(&environment_facts()).expect("a preamble");
        assert!(
            prompt.contains(&derived),
            "the capability prompt must carry the derived preamble verbatim:\n{prompt}"
        );
        // And the facts themselves reach the model.
        assert!(prompt.contains("container"), "{prompt}");
        assert!(prompt.contains("Nothing recovers"), "{prompt}");
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
