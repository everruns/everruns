//! CodeSandbox Integration
//!
//! Cloud-based sandboxed code execution via CodeSandbox REST API.
//! Supports multiple sandboxes per session, each identified by sandbox_id.
//!
//! Decision: External integration crate, auto-registered via inventory plugin system
//! Decision: Use secrets store for all state (API key + per-sandbox pitcher tokens)
//! Decision: Two-tier API: Management API for lifecycle, Pint API for in-sandbox ops
//! Decision: Sync+async exec modes via `wait` parameter
//! Decision: session_storage dependency for API key and state persistence

pub mod client;
pub mod state;
pub mod tools;
pub mod types;

use everruns_core::capabilities::{Capability, CapabilityStatus, IntegrationPlugin};
use everruns_core::tools::Tool;

use tools::*;

// ----------------------------------------------------------------------------
// Integration Plugin Registration
// ----------------------------------------------------------------------------

inventory::submit! {
    IntegrationPlugin {
        experimental_only: false,
        factory: || Box::new(CodeSandboxCapability),
    }
}

// ----------------------------------------------------------------------------
// CodeSandboxCapability
// ----------------------------------------------------------------------------

pub struct CodeSandboxCapability;

impl Capability for CodeSandboxCapability {
    fn id(&self) -> &str {
        "codesandbox"
    }

    fn name(&self) -> &str {
        "CodeSandbox"
    }

    fn description(&self) -> &str {
        "Run code in cloud-based sandbox VMs powered by CodeSandbox. \
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
            r#"## CodeSandbox (Experimental)

Cloud-based sandbox VMs via CodeSandbox. Each sandbox is an isolated Firecracker microVM with full Linux and network access.

Prerequisite: CSB_API_KEY must be set in session secrets before using any sandbox tool.

Tools:
- `csb_create_sandbox` - Create and start a new sandbox VM
- `csb_exec` - Run a shell command (`wait: true` for output, `wait: false` for async)
- `csb_exec_status` - Poll async execution status/output
- `csb_read_file` / `csb_write_file` - Read/write files in sandbox
- `csb_download_workspace` - Download sandbox workspace to session storage
- `csb_list_sandboxes` - List session sandboxes
- `csb_manage_sandbox` - Shutdown, hibernate, or delete a sandbox
- `csb_git_clone` - Clone a git repository into a sandbox (auto-uses connected GitHub credentials)

Git cloning: Use `csb_git_clone` to clone repositories into `/sandbox/owner/repo`.
If the user has connected their GitHub account (Settings > Connections), private repos
are automatically authenticated. For public repos, no credentials are needed.
Supports "user/repo" shorthand. Working directory is `/sandbox`.

All tools except `csb_create_sandbox` and `csb_list_sandboxes` require a `sandbox_id`.
Sandboxes auto-hibernate after 5 minutes of inactivity.
Always DELETE sandboxes when done (shutdown/hibernate leave them on the dashboard)."#,
        )
    }

    fn tools(&self) -> Vec<Box<dyn Tool>> {
        vec![
            Box::new(CsbCreateSandboxTool),
            Box::new(CsbExecTool),
            Box::new(CsbExecStatusTool),
            Box::new(CsbReadFileTool),
            Box::new(CsbWriteFileTool),
            Box::new(CsbDownloadWorkspaceTool),
            Box::new(CsbListSandboxesTool),
            Box::new(CsbManageSandboxTool),
            Box::new(CsbGitCloneTool),
        ]
    }

    fn dependencies(&self) -> Vec<&'static str> {
        vec!["session_storage"]
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_capability_metadata() {
        let cap = CodeSandboxCapability;
        assert_eq!(cap.id(), "codesandbox");
        assert_eq!(cap.name(), "CodeSandbox");
        assert_eq!(cap.status(), CapabilityStatus::Available);
        assert_eq!(cap.icon(), Some("cloud"));
        assert_eq!(cap.category(), Some("Execution"));
    }

    #[test]
    fn test_capability_has_all_tools() {
        let cap = CodeSandboxCapability;
        let tools = cap.tools();
        assert_eq!(tools.len(), 9);

        let names: Vec<&str> = tools.iter().map(|t| t.name()).collect();
        assert!(names.contains(&"csb_create_sandbox"));
        assert!(names.contains(&"csb_exec"));
        assert!(names.contains(&"csb_exec_status"));
        assert!(names.contains(&"csb_read_file"));
        assert!(names.contains(&"csb_write_file"));
        assert!(names.contains(&"csb_download_workspace"));
        assert!(names.contains(&"csb_list_sandboxes"));
        assert!(names.contains(&"csb_manage_sandbox"));
        assert!(names.contains(&"csb_git_clone"));
    }

    #[test]
    fn test_capability_has_system_prompt() {
        let cap = CodeSandboxCapability;
        let prompt = cap.system_prompt_addition().unwrap();
        assert!(prompt.contains("csb_create_sandbox"));
        assert!(prompt.contains("CSB_API_KEY"));
        assert!(prompt.contains("Experimental"));
        assert!(prompt.contains("csb_exec"));
        assert!(prompt.contains("csb_download_workspace"));
        assert!(prompt.contains("csb_git_clone"));
        assert!(prompt.contains("Prerequisite"));
        // Should NOT duplicate workflow steps (that's the agent's job)
        assert!(!prompt.contains("Set API key (once per session)"));
    }

    #[test]
    fn test_all_tools_require_context() {
        let cap = CodeSandboxCapability;
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
        let cap = CodeSandboxCapability;
        assert_eq!(cap.dependencies(), vec!["session_storage"]);
    }
}
