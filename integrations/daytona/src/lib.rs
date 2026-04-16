//! Daytona Integration (Experimental)
//!
//! Cloud-based sandboxed code execution via Daytona REST API.
//! Supports multiple sandboxes per session, each identified by sandbox_id.
//!
//! Decision: External integration crate, auto-registered via inventory plugin system
//! Decision: Use secrets store for all state (API key + per-sandbox connection info)
//! Decision: Two-tier API: Management API for lifecycle, Toolbox API for in-sandbox ops
//! Decision: All exec goes through Session API (/process/session) — persistent shell,
//!           async execution, and log polling for real-time streaming output
//! Decision: session_storage dependency for API key and state persistence

pub mod client;
pub mod connection;
pub mod openapi_spec;
mod session_sandbox_provider;
pub mod state;
mod tools;

use everruns_core::LEASED_RESOURCES_FEATURE;
use everruns_core::capabilities::{
    Capability, CapabilityStatus, IntegrationPlugin, MountDirectoryBuilder, MountPoint, RiskLevel,
    SystemPromptContext,
};
use everruns_core::connection_provider::ConnectionProviderPlugin;
use everruns_core::tools::Tool;

use connection::DaytonaConnectionProvider;
use session_sandbox_provider::DaytonaSessionSandboxProvider;
use std::sync::LazyLock;
use std::time::Duration;

use tools::{
    DAYTONA_OPENAPI_MOUNT_PATH, DaytonaApiCallTool, DaytonaCreateSandboxTool,
    DaytonaDownloadWorkspaceTool, DaytonaExecTool, DaytonaGitCloneTool, DaytonaGitCredentialsTool,
    DaytonaListSandboxesTool, DaytonaListSnapshotsTool, DaytonaManageSandboxTool,
    DaytonaReadFileTool, DaytonaWriteFileTool,
};

// ============================================================================
// Plugin Registration
// ============================================================================

inventory::submit! {
    IntegrationPlugin {
        experimental_only: false,
        feature_flag: None,
        factory: || Box::new(DaytonaCapability),
    }
}

inventory::submit! {
    ConnectionProviderPlugin {
        experimental_only: true,
        factory: || Box::new(DaytonaConnectionProvider),
    }
}

inventory::submit! {
    everruns_core::SessionSandboxProviderPlugin {
        factory: || Box::new(DaytonaSessionSandboxProvider),
    }
}

// ============================================================================
// Constants
// ============================================================================

const DAYTONA_API_BASE: &str = "https://app.daytona.io/api";
const DAYTONA_TOOLBOX_BASE: &str = "https://proxy.app.daytona.io/toolbox";
const DAYTONA_SANDBOX_SECRET_PREFIX: &str = "daytona_sandbox:";
const EXEC_TIMEOUT_MS: u64 = 300_000;
/// Interval for polling streaming exec output from the sandbox.
const EXEC_POLL_INTERVAL: Duration = Duration::from_millis(1_000);
/// Number of consecutive stale polls (no exitCode, no new output) before
/// probing session health with a heartbeat command.
const SESSION_STALE_THRESHOLD: u32 = 30;
/// Timeout (ms) for the heartbeat probe command (`true`).
const SESSION_HEARTBEAT_TIMEOUT_MS: u64 = 15_000;
const SANDBOX_READY_POLL_INTERVAL: Duration = Duration::from_secs(2);
const SANDBOX_READY_MAX_WAIT: Duration = Duration::from_secs(60);
/// Auto-stop after 5 minutes of inactivity (safety net)
const AUTO_STOP_INTERVAL_MINUTES: u64 = 5;
/// Auto-archive after 30 minutes — archived sandboxes consume fewer Daytona resources
const AUTO_ARCHIVE_INTERVAL_MINUTES: u64 = 30;
/// Auto-delete after 60 minutes — Daytona-native safety net beyond our leased-resource cleanup
const AUTO_DELETE_INTERVAL_MINUTES: u64 = 60;
/// Default workspace path inside Daytona sandboxes
const DAYTONA_WORKSPACE_PATH: &str = "/home/daytona";
/// Heartbeat interval for renewing leases during long-running exec calls.
/// Set well below the auto-stop (5 min) so the sandbox stays alive.
const LEASE_HEARTBEAT_INTERVAL: Duration = Duration::from_secs(3 * 60);

// ============================================================================
// DaytonaCapability
// ============================================================================

static SYSTEM_PROMPT: LazyLock<String> = LazyLock::new(|| {
    let mut prompt = String::from(
        r#"## Daytona

Cloud-based sandboxes via Daytona. Each sandbox is an isolated environment with full Linux and network access.

Authentication: Daytona API key is resolved automatically from Settings > Connections > Daytona.
If not configured, guide the user to set up their key in Settings > Connections.

All tools except `daytona_create_sandbox`, `daytona_list_sandboxes`, and `daytona_list_snapshots` require a `sandbox_id`.
Use `daytona_list_snapshots` to discover available snapshots and their resource specs before creating sandboxes.
Sandboxes auto-stop after 5 min of inactivity, auto-archive after 30 min, and auto-delete after 60 min.
Stopped sandboxes are also cleaned up by the control plane after 20 min.
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
    );
    prompt.push_str(everruns_core::tool_output_sanitizer::EXEC_OUTPUT_HINT);
    prompt
});

/// Check if API calling is enabled in capability config.
///
/// When `enable_api_calling` is `true`, the `daytona_api_call` tool is added
/// and the OpenAPI spec mount path is referenced in the system prompt.
fn is_api_calling_enabled(config: &serde_json::Value) -> bool {
    config
        .get("enable_api_calling")
        .and_then(|v| v.as_bool())
        .unwrap_or(false)
}

pub struct DaytonaCapability;

#[async_trait::async_trait]
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
        Some(&SYSTEM_PROMPT)
    }

    fn tools(&self) -> Vec<Box<dyn Tool>> {
        vec![
            Box::new(DaytonaCreateSandboxTool),
            Box::new(DaytonaExecTool),
            Box::new(DaytonaReadFileTool),
            Box::new(DaytonaWriteFileTool),
            Box::new(DaytonaDownloadWorkspaceTool),
            Box::new(DaytonaListSnapshotsTool),
            Box::new(DaytonaListSandboxesTool),
            Box::new(DaytonaManageSandboxTool),
            Box::new(DaytonaGitCloneTool),
            Box::new(DaytonaGitCredentialsTool),
        ]
    }

    fn tools_with_config(&self, config: &serde_json::Value) -> Vec<Box<dyn Tool>> {
        let mut tools = self.tools();
        if is_api_calling_enabled(config) {
            tools.push(Box::new(DaytonaApiCallTool));
        }
        tools
    }

    fn mounts(&self) -> Vec<MountPoint> {
        // OpenAPI spec is always mounted so it's available if API calling is enabled.
        // The tool itself is only added when config has enable_api_calling: true.
        let daytona_dir = MountDirectoryBuilder::new()
            .file("openapi.yaml", openapi_spec::DAYTONA_OPENAPI_SPEC)
            .build();
        vec![MountPoint::readonly("/daytona", daytona_dir, self.id())]
    }

    async fn system_prompt_contribution_with_config(
        &self,
        _ctx: &SystemPromptContext,
        config: &serde_json::Value,
    ) -> Option<String> {
        let base = self.system_prompt_addition()?;
        if is_api_calling_enabled(config) {
            let api_calling_addition = format!(
                "\n\n### Direct API Access\n\n\
                 Direct Daytona API calling is enabled. Use `daytona_api_call` to call any \
                 Daytona REST API endpoint not covered by the dedicated tools.\n\
                 The full OpenAPI spec is mounted at `{DAYTONA_OPENAPI_MOUNT_PATH}` — \
                 read it to discover endpoints, parameters, and schemas.\n\
                 Resources created via `daytona_api_call` (e.g. POST /sandbox) are \
                 automatically tracked for cleanup."
            );
            Some(format!(
                "<capability id=\"{}\">\n{}{}\n</capability>",
                self.id(),
                base,
                api_calling_addition
            ))
        } else {
            Some(format!(
                "<capability id=\"{}\">\n{}\n</capability>",
                self.id(),
                base
            ))
        }
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
    use serde_json::json;

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
        assert_eq!(tools.len(), 10);

        let names: Vec<&str> = tools.iter().map(|t| t.name()).collect();
        assert!(names.contains(&"daytona_create_sandbox"));
        assert!(names.contains(&"daytona_exec"));
        assert!(names.contains(&"daytona_read_file"));
        assert!(names.contains(&"daytona_write_file"));
        assert!(names.contains(&"daytona_download_workspace"));
        assert!(names.contains(&"daytona_list_snapshots"));
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

    // --- API calling opt-in tests ---

    #[test]
    fn test_is_api_calling_enabled_default_false() {
        assert!(!is_api_calling_enabled(&json!({})));
        assert!(!is_api_calling_enabled(&json!(null)));
        assert!(!is_api_calling_enabled(
            &json!({"enable_api_calling": false})
        ));
    }

    #[test]
    fn test_is_api_calling_enabled_true() {
        assert!(is_api_calling_enabled(&json!({"enable_api_calling": true})));
    }

    #[test]
    fn test_tools_with_config_default_no_api_call() {
        let cap = DaytonaCapability;
        let tools = cap.tools_with_config(&json!({}));
        assert_eq!(tools.len(), 10);
        let names: Vec<&str> = tools.iter().map(|t| t.name()).collect();
        assert!(!names.contains(&"daytona_api_call"));
    }

    #[test]
    fn test_tools_with_config_api_calling_enabled() {
        let cap = DaytonaCapability;
        let tools = cap.tools_with_config(&json!({"enable_api_calling": true}));
        assert_eq!(tools.len(), 11);
        let names: Vec<&str> = tools.iter().map(|t| t.name()).collect();
        assert!(names.contains(&"daytona_api_call"));
    }

    #[test]
    fn test_api_call_tool_requires_context() {
        let cap = DaytonaCapability;
        let tools = cap.tools_with_config(&json!({"enable_api_calling": true}));
        let api_tool = tools
            .iter()
            .find(|t| t.name() == "daytona_api_call")
            .unwrap();
        assert!(api_tool.requires_context());
    }

    #[test]
    fn test_mounts_include_openapi_spec() {
        let cap = DaytonaCapability;
        let mounts = cap.mounts();
        assert_eq!(mounts.len(), 1);
        assert_eq!(mounts[0].path, "/daytona");
        assert!(mounts[0].is_readonly());
    }

    #[test]
    fn test_openapi_spec_is_valid_yaml() {
        let spec = openapi_spec::DAYTONA_OPENAPI_SPEC;
        assert!(spec.contains("openapi: 3.0.3"));
        assert!(spec.contains("/sandbox"));
        assert!(spec.contains("/toolbox/"));
    }

    #[tokio::test]
    async fn test_system_prompt_with_api_calling_enabled() {
        let cap = DaytonaCapability;
        let ctx = SystemPromptContext::without_file_store(everruns_core::SessionId::new());
        let prompt = cap
            .system_prompt_contribution_with_config(&ctx, &json!({"enable_api_calling": true}))
            .await
            .unwrap();
        assert!(prompt.contains("daytona_api_call"));
        assert!(prompt.contains(DAYTONA_OPENAPI_MOUNT_PATH));
        assert!(prompt.contains("Direct API Access"));
    }

    #[tokio::test]
    async fn test_system_prompt_without_api_calling() {
        let cap = DaytonaCapability;
        let ctx = SystemPromptContext::without_file_store(everruns_core::SessionId::new());
        let prompt = cap
            .system_prompt_contribution_with_config(&ctx, &json!({}))
            .await
            .unwrap();
        assert!(!prompt.contains("daytona_api_call"));
        assert!(!prompt.contains("Direct API Access"));
        // Still has the base prompt
        assert!(prompt.contains("Daytona"));
    }
}
