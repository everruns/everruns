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
pub mod connection;
pub mod state;
mod tools;

use everruns_core::LEASED_RESOURCES_FEATURE;
use everruns_core::capabilities::{Capability, CapabilityStatus, IntegrationPlugin, RiskLevel};
use everruns_core::connection_provider::ConnectionProviderPlugin;
use everruns_core::tools::Tool;

use connection::DaytonaConnectionProvider;
use std::time::Duration;

use tools::{
    DaytonaCreateSandboxTool, DaytonaDownloadWorkspaceTool, DaytonaExecTool, DaytonaGitCloneTool,
    DaytonaGitCredentialsTool, DaytonaListSandboxesTool, DaytonaManageSandboxTool,
    DaytonaReadFileTool, DaytonaWriteFileTool,
};

// ============================================================================
// Plugin Registration
// ============================================================================

inventory::submit! {
    IntegrationPlugin {
        experimental_only: false,
        factory: || Box::new(DaytonaCapability),
    }
}

inventory::submit! {
    ConnectionProviderPlugin {
        experimental_only: true,
        factory: || Box::new(DaytonaConnectionProvider),
    }
}

// ============================================================================
// Constants
// ============================================================================

const DAYTONA_API_BASE: &str = "https://app.daytona.io/api";
const DAYTONA_TOOLBOX_BASE: &str = "https://proxy.app.daytona.io/toolbox";
const DAYTONA_SANDBOX_SECRET_PREFIX: &str = "daytona_sandbox:";
const EXEC_TIMEOUT_MS: u64 = 120_000;
const SANDBOX_READY_POLL_INTERVAL: Duration = Duration::from_secs(2);
const SANDBOX_READY_MAX_WAIT: Duration = Duration::from_secs(60);
/// Auto-stop after 5 minutes of inactivity (safety net)
const AUTO_STOP_INTERVAL_MINUTES: u64 = 5;
/// Default workspace path inside Daytona sandboxes
const DAYTONA_WORKSPACE_PATH: &str = "/home/daytona";

// ============================================================================
// DaytonaCapability
// ============================================================================

pub struct DaytonaCapability;

impl Capability for DaytonaCapability {
    fn id(&self) -> &str {
        "daytona"
    }

    fn name(&self) -> &str {
        "Daytona"
    }

    fn description(&self) -> &str {
        "Run code in cloud-based sandboxes powered by Daytona. \
         Create multiple isolated Linux environments per session, execute commands, \
         manage files, and download results. EXPERIMENTAL: This capability may change."
    }

    fn status(&self) -> CapabilityStatus {
        CapabilityStatus::Available
    }

    fn risk_level(&self) -> RiskLevel {
        RiskLevel::High
    }

    fn icon(&self) -> Option<&str> {
        Some("daytona")
    }

    fn category(&self) -> Option<&str> {
        Some("Execution")
    }

    fn system_prompt_addition(&self) -> Option<&str> {
        // Tool names and descriptions are NOT listed here — they are already
        // provided via the tools parameter in the API request.  Duplicating
        // them in the system prompt wastes tokens and can confuse models
        // (especially GPT-5.4+ with tool_search).
        Some(
            r#"## Daytona

Cloud-based sandboxes via Daytona. Each sandbox is an isolated environment with full Linux and network access.

Authentication: Daytona API key is resolved automatically from Settings > Connections > Daytona.
If not configured, guide the user to set up their key in Settings > Connections.

All tools except `daytona_create_sandbox` and `daytona_list_sandboxes` require a `sandbox_id`.
Sandboxes auto-stop after 5 minutes of inactivity.
Always DELETE sandboxes when done (stop leaves them on the dashboard).
Active sandboxes also appear in the session Resources tab so users can see what
may be cleaned automatically later.

Git cloning: Use `daytona_git_clone` to clone repositories into `/home/daytona/owner/repo`.
If the user has connected their GitHub account (Settings > Connections), private repos
are automatically authenticated. For public repos, no credentials are needed.
Supports "user/repo" shorthand. Working directory is `/home/daytona`.

Git push/pull/fetch: After cloning, call `daytona_git_credentials` once to configure
credentials in the sandbox. Then use `daytona_exec` for any git command (push, pull,
fetch, rebase, etc.) — they authenticate automatically. Call again to refresh (~1h expiry)."#,
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
            Box::new(DaytonaGitCredentialsTool),
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
    use everruns_core::capabilities::CapabilityStatus;

    #[test]
    fn test_capability_metadata() {
        let cap = DaytonaCapability;
        assert_eq!(cap.id(), "daytona");
        assert_eq!(cap.name(), "Daytona");
        assert_eq!(cap.status(), CapabilityStatus::Available);
        assert_eq!(cap.icon(), Some("daytona"));
        assert_eq!(cap.category(), Some("Execution"));
    }

    #[test]
    fn test_capability_has_all_tools() {
        let cap = DaytonaCapability;
        let tools = cap.tools();
        assert_eq!(tools.len(), 9);

        let names: Vec<&str> = tools.iter().map(|t| t.name()).collect();
        assert!(names.contains(&"daytona_create_sandbox"));
        assert!(names.contains(&"daytona_exec"));
        assert!(names.contains(&"daytona_read_file"));
        assert!(names.contains(&"daytona_write_file"));
        assert!(names.contains(&"daytona_download_workspace"));
        assert!(names.contains(&"daytona_list_sandboxes"));
        assert!(names.contains(&"daytona_manage_sandbox"));
        assert!(names.contains(&"daytona_git_clone"));
        assert!(names.contains(&"daytona_git_credentials"));
    }

    #[test]
    fn test_capability_has_system_prompt() {
        let cap = DaytonaCapability;
        let prompt = cap.system_prompt_addition().unwrap();
        assert!(prompt.contains("Daytona"));
        assert!(prompt.contains("Settings > Connections"));
        // Tool names referenced in usage context (not as a tool list)
        assert!(prompt.contains("daytona_create_sandbox"));
        assert!(prompt.contains("daytona_git_clone"));
        assert!(prompt.contains("daytona_git_credentials"));
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
