//! Deno cloud sandboxes for Everruns agents.
//!
//! This integration contributes tools for creating, managing, and using Deno
//! sandboxes to the [Everruns](https://everruns.com) ecosystem.
//!
//! # Example
//!
//! ```
//! use everruns_integrations_deno::DenoCapability;
//!
//! let capability = DenoCapability;
//! # let _ = capability;
//! ```

pub mod client;
pub mod connection;
pub mod state;
mod tools;

use everruns_core::LEASED_RESOURCES_FEATURE;
use everruns_core::capabilities::{
    Capability, CapabilityLocalization, CapabilityStatus, IntegrationPlugin, RiskLevel,
};
use everruns_core::tools::Tool;
use everruns_platform::connector::ConnectorPlugin;
use std::sync::LazyLock;
use std::time::Duration;

use connection::DenoConnector;
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
    ConnectorPlugin {
        experimental_only: true,
        factory: || Box::new(DenoConnector),
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
const DENO_WORKSPACE_PATH: &str = "/home/app";

static SYSTEM_PROMPT: LazyLock<String> = LazyLock::new(|| {
    let mut prompt = String::from(
        "Deno sandboxes are isolated networked microVMs. Create or select a sandbox before sandbox-scoped operations, use `/home/app` as the default workspace, and delete sandboxes when done to avoid leaks.",
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

    fn localizations(&self) -> Vec<CapabilityLocalization> {
        vec![CapabilityLocalization::text(
            "uk",
            "Пісочниці Deno",
            "Запускайте код у хмарних пісочницях Deno. Створюйте ізольовані мікровіртуальні \
             машини Linux, виконуйте команди та керуйте файлами. ЕКСПЕРИМЕНТАЛЬНО: ця \
             можливість може змінитися.",
        )]
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
        Some("Sandboxes")
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
        assert_eq!(cap.category(), Some("Sandboxes"));
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

    #[tokio::test]
    async fn system_prompt_within_budget() {
        let cap = DenoCapability;
        let ctx = everruns_core::capabilities::SystemPromptContext::without_file_store(
            everruns_core::SessionId::new(),
        );
        let prompt = cap.system_prompt_contribution(&ctx).await.unwrap();
        // Bumped 1000 → 1300: EVE-778 grew the shared EXEC_OUTPUT_HINT with the
        // single-read/contextual-search policy (+438 bytes), taking this
        // contribution to 1256 bytes.
        assert!(prompt.len() <= 1300, "prompt is {} bytes", prompt.len());
    }
}
