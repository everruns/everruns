//! Daytona Integration (Experimental)
//!
//! Cloud-based sandboxed code execution via Daytona REST API.
//! Supports multiple sandboxes per session, each identified by sandbox_id.
//!
//! Decision: External integration crate, auto-registered via inventory plugin system
//! Decision: Use secrets store for all state (API key + per-sandbox connection info)
//! Decision: Two-tier API: Management API for lifecycle, Toolbox API for in-sandbox ops
//! Decision: Sync exec via toolbox process/execute endpoint
//! Decision: session_storage dependency for API key and state persistence

pub mod client;
pub mod state;
mod tools;

use everruns_core::capabilities::{Capability, CapabilityStatus, IntegrationPlugin};
use everruns_core::tools::Tool;

use std::time::Duration;

use tools::{
    DaytonaCreateSandboxTool, DaytonaDownloadWorkspaceTool, DaytonaExecTool, DaytonaGitCloneTool,
    DaytonaListSandboxesTool, DaytonaManageSandboxTool, DaytonaReadFileTool, DaytonaWriteFileTool,
};

// ============================================================================
// Integration Plugin Registration
// ============================================================================

inventory::submit! {
    IntegrationPlugin {
        experimental_only: true,
        factory: || Box::new(DaytonaCapability),
    }
}

// ============================================================================
// Constants
// ============================================================================

const DAYTONA_API_BASE: &str = "https://app.daytona.io/api";
const DAYTONA_TOOLBOX_BASE: &str = "https://proxy.app.daytona.io/toolbox";
const DAYTONA_API_KEY_SECRET: &str = "DAYTONA_API_KEY";
const DAYTONA_SANDBOX_SECRET_PREFIX: &str = "daytona_sandbox:";
const EXEC_TIMEOUT_MS: u64 = 120_000;
const SANDBOX_READY_POLL_INTERVAL: Duration = Duration::from_secs(2);
const SANDBOX_READY_MAX_WAIT: Duration = Duration::from_secs(60);
/// Auto-stop after 5 minutes of inactivity (safety net)
const AUTO_STOP_INTERVAL_MINUTES: u64 = 5;

// ============================================================================
// DaytonaCapability
// ============================================================================

pub struct DaytonaCapability;

impl Capability for DaytonaCapability {
    fn id(&self) -> &str {
        "daytona"
    }

    fn name(&self) -> &str {
        "[Experimental] Daytona"
    }

    fn description(&self) -> &str {
        "Run code in cloud-based sandboxes powered by Daytona. \
         Create multiple isolated Linux environments per session, execute commands, \
         manage files, and download results. EXPERIMENTAL: This capability may change."
    }

    fn status(&self) -> CapabilityStatus {
        CapabilityStatus::Available
    }

    fn icon(&self) -> Option<&str> {
        Some("cloud")
    }

    fn category(&self) -> Option<&str> {
        Some("Execution")
    }

    fn system_prompt_addition(&self) -> Option<&str> {
        Some(
            r#"## Daytona (Experimental)

Cloud-based sandboxes via Daytona. Each sandbox is an isolated environment with full Linux and network access.

Prerequisite: DAYTONA_API_KEY must be set in session secrets before using any sandbox tool.

Tools:
- `daytona_create_sandbox` - Create and start a new sandbox
- `daytona_exec` - Run a shell command (synchronous, returns output)
- `daytona_read_file` / `daytona_write_file` - Read/write files in sandbox
- `daytona_download_workspace` - Download sandbox workspace to session storage
- `daytona_list_sandboxes` - List session sandboxes
- `daytona_manage_sandbox` - Stop or delete a sandbox
- `daytona_git_clone` - Clone a git repository into a sandbox (auto-uses connected GitHub credentials)

All tools except `daytona_create_sandbox` and `daytona_list_sandboxes` require a `sandbox_id`.
Sandboxes auto-stop after 5 minutes of inactivity.
Always DELETE sandboxes when done (stop leaves them on the dashboard).

Git cloning: Use `daytona_git_clone` to clone repositories. If the user has connected
their GitHub account (Settings > Connections), private repos are automatically
authenticated. For public repos, no credentials are needed. Supports "user/repo" shorthand."#,
        )
    }

    fn tools(&self) -> Vec<Box<dyn Tool>> {
        vec![
            Box::new(DaytonaCreateSandboxTool),
            Box::new(DaytonaExecTool),
            Box::new(DaytonaReadFileTool),
            Box::new(DaytonaWriteFileTool),
            Box::new(DaytonaDownloadWorkspaceTool),
            Box::new(DaytonaListSandboxesTool),
            Box::new(DaytonaManageSandboxTool),
            Box::new(DaytonaGitCloneTool),
        ]
    }

    fn dependencies(&self) -> Vec<&'static str> {
        vec!["session_storage"]
    }
}

// ============================================================================
// Tests
// ============================================================================

#[cfg(test)]
mod tests {
    use super::*;
    use everruns_core::capabilities::CapabilityStatus;

    #[test]
    fn test_capability_metadata() {
        let cap = DaytonaCapability;
        assert_eq!(cap.id(), "daytona");
        assert_eq!(cap.name(), "[Experimental] Daytona");
        assert_eq!(cap.status(), CapabilityStatus::Available);
        assert_eq!(cap.icon(), Some("cloud"));
        assert_eq!(cap.category(), Some("Execution"));
    }

    #[test]
    fn test_capability_has_all_tools() {
        let cap = DaytonaCapability;
        let tools = cap.tools();
        assert_eq!(tools.len(), 8);

        let names: Vec<&str> = tools.iter().map(|t| t.name()).collect();
        assert!(names.contains(&"daytona_create_sandbox"));
        assert!(names.contains(&"daytona_exec"));
        assert!(names.contains(&"daytona_read_file"));
        assert!(names.contains(&"daytona_write_file"));
        assert!(names.contains(&"daytona_download_workspace"));
        assert!(names.contains(&"daytona_list_sandboxes"));
        assert!(names.contains(&"daytona_manage_sandbox"));
        assert!(names.contains(&"daytona_git_clone"));
    }

    #[test]
    fn test_capability_has_system_prompt() {
        let cap = DaytonaCapability;
        let prompt = cap.system_prompt_addition().unwrap();
        assert!(prompt.contains("daytona_create_sandbox"));
        assert!(prompt.contains("daytona_git_clone"));
        assert!(prompt.contains("DAYTONA_API_KEY"));
        assert!(prompt.contains("Experimental"));
    }

    #[test]
    fn test_all_tools_require_context() {
        let cap = DaytonaCapability;
        for tool in cap.tools() {
            assert!(
                tool.requires_context(),
                "Tool {} should require context",
                tool.name()
            );
        }
    }

    #[test]
    fn test_capability_dependencies() {
        let cap = DaytonaCapability;
        assert_eq!(cap.dependencies(), vec!["session_storage"]);
    }
}
