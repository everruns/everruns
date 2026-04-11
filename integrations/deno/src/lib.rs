//! Deno Sandbox Integration.
//!
//! Cloud-based sandboxed code execution via Deno Sandboxes.
//! Supports multiple sandboxes per session, each identified by `sandbox_id`.
//!
//! Decision: model the public tool surface after Daytona so agents can switch
//! between cloud sandboxes with minimal prompt changes.
//! Decision: prefer user connections, but allow `DENO_DEPLOY_TOKEN` /
//! `DENO_DEPLOY_ORG` env fallbacks for operator-managed deployments and live tests.
//! Decision: create sandboxes with a fixed timeout instead of Deno's default
//! `session` lifetime because Everruns closes the creator websocket after each tool.

pub mod client;
pub mod connection;
pub mod state;
mod tools;

use everruns_core::LEASED_RESOURCES_FEATURE;
use everruns_core::capabilities::{Capability, CapabilityStatus, IntegrationPlugin, RiskLevel};
use everruns_core::connection_provider::ConnectionProviderPlugin;
use everruns_core::tools::Tool;
use std::sync::LazyLock;
use std::time::Duration;

use connection::DenoConnectionProvider;
use tools::{
    DenoCreateSandboxTool, DenoExecTool, DenoListSandboxesTool, DenoManageSandboxTool,
    DenoReadFileTool, DenoWriteFileTool,
};

inventory::submit! {
    IntegrationPlugin {
        experimental_only: false,
        feature_flag: None,
        factory: || Box::new(DenoCapability),
    }
}

inventory::submit! {
    ConnectionProviderPlugin {
        experimental_only: true,
        factory: || Box::new(DenoConnectionProvider),
    }
}

const DENO_CONSOLE_API_BASE: &str = "https://console.deno.com";
const DENO_SANDBOX_BASE_DOMAIN: &str = "sandbox-api.deno.net";
const DENO_SANDBOX_SECRET_PREFIX: &str = "deno_sandbox:";
const DENO_SANDBOX_TIMEOUT: &str = "20m";
const DENO_DEFAULT_MEMORY_MB: u64 = 1_280;
const DENO_MAX_MEMORY_MB: u64 = 16 * 1_024;
const DENO_RPC_TIMEOUT: Duration = Duration::from_secs(30);
const DENO_STREAM_IDLE_TIMEOUT: Duration = Duration::from_secs(2);
const DENO_WORKSPACE_PATH: &str = "/home/sandbox";

static SYSTEM_PROMPT: LazyLock<String> = LazyLock::new(|| {
    let mut prompt = String::from(
        r#"## Deno Sandboxes

Cloud-based sandboxes via Deno. Each sandbox is an isolated Linux microVM with network access.

Authentication: Deno access token is resolved automatically from Settings > Connections > Deno Deploy.
If not configured, guide the user to set it up in Settings > Connections.

All tools except `deno_create_sandbox` and `deno_list_sandboxes` require a `sandbox_id`.
Sandboxes are created with a fixed timeout because Everruns closes the create connection after each tool.
Always DELETE sandboxes when done to avoid resource leaks.

Use `deno_exec` for shell commands, `deno_write_file` / `deno_read_file` for files, and `deno_manage_sandbox` with action=`delete` for cleanup."#,
    );
    prompt.push_str(everruns_core::tool_output_sanitizer::EXEC_OUTPUT_HINT);
    prompt
});

pub struct DenoCapability;

impl Capability for DenoCapability {
    fn id(&self) -> &str {
        "deno"
    }

    fn name(&self) -> &str {
        "Deno Sandboxes"
    }

    fn description(&self) -> &str {
        "Run code in cloud-based Deno sandboxes. Create isolated Linux microVMs, execute commands, and manage files. EXPERIMENTAL: This capability may change."
    }

    fn status(&self) -> CapabilityStatus {
        CapabilityStatus::Available
    }

    fn risk_level(&self) -> RiskLevel {
        RiskLevel::High
    }

    fn icon(&self) -> Option<&str> {
        Some("server")
    }

    fn category(&self) -> Option<&str> {
        Some("Execution")
    }

    fn system_prompt_addition(&self) -> Option<&str> {
        Some(&SYSTEM_PROMPT)
    }

    fn tools(&self) -> Vec<Box<dyn Tool>> {
        vec![
            Box::new(DenoCreateSandboxTool),
            Box::new(DenoExecTool),
            Box::new(DenoReadFileTool),
            Box::new(DenoWriteFileTool),
            Box::new(DenoListSandboxesTool),
            Box::new(DenoManageSandboxTool),
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
    use everruns_core::capabilities::CapabilityStatus;

    #[test]
    fn test_capability_metadata() {
        let cap = DenoCapability;
        assert_eq!(cap.id(), "deno");
        assert_eq!(cap.name(), "Deno Sandboxes");
        assert_eq!(cap.status(), CapabilityStatus::Available);
        assert_eq!(cap.icon(), Some("server"));
        assert_eq!(cap.category(), Some("Execution"));
    }

    #[test]
    fn test_capability_has_all_tools() {
        let cap = DenoCapability;
        let tools = cap.tools();
        assert_eq!(tools.len(), 6);

        let names: Vec<&str> = tools.iter().map(|t| t.name()).collect();
        assert!(names.contains(&"deno_create_sandbox"));
        assert!(names.contains(&"deno_exec"));
        assert!(names.contains(&"deno_read_file"));
        assert!(names.contains(&"deno_write_file"));
        assert!(names.contains(&"deno_list_sandboxes"));
        assert!(names.contains(&"deno_manage_sandbox"));
    }
}
