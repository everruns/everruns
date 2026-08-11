//! E2B cloud sandboxes for Everruns agents.
//!
//! `everruns-integrations-e2b` is part of the [Everruns](https://everruns.com)
//! ecosystem. It gives agents cloud sandboxes backed by [E2B](https://e2b.dev),
//! so they can create isolated environments, run processes, and read or write
//! files without touching the host. Sandboxes are managed per session with
//! leased-resource cleanup, and authenticate with a user-supplied E2B API key.
//!
//! # Example
//!
//! ```
//! use everruns_core::capabilities::Capability;
//! use everruns_integrations_e2b::E2BCapability;
//!
//! let capability = E2BCapability;
//! assert_eq!(capability.id(), "e2b");
//! ```
//!
//! # Design notes
//!
//! - External integration crate, auto-registered via the inventory plugin system.
//! - Bring-your-own API-key connection model: credentials resolve from the user
//!   connection only and fail with `ConnectionRequired` if not configured.
//! - Session-scoped sandbox state with leased-resource cleanup, similar to
//!   Daytona.

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

use connection::E2BConnector;
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

inventory::submit! {
    ConnectorPlugin {
        experimental_only: false,
        factory: || Box::new(E2BConnector),
    }
}

pub const E2B_API_BASE: &str = "https://api.e2b.app";
pub const E2B_SANDBOX_SECRET_PREFIX: &str = "e2b_sandbox:";
pub const E2B_DEFAULT_TEMPLATE: &str = "base";
pub const E2B_DEFAULT_TIMEOUT_SECS: u64 = 60 * 60;
pub const E2B_DEFAULT_WORKSPACE_PATH: &str = "/home/user";
pub const E2B_ENVD_PORT: u16 = 49_983;

static SYSTEM_PROMPT: LazyLock<String> = LazyLock::new(|| {
    let mut prompt = String::from(
        "E2B sandboxes are isolated networked Linux environments. Create or select a sandbox before sandbox-scoped operations, prefer `/home/user` as workspace root, and pause or delete sandboxes when done.",
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

    fn localizations(&self) -> Vec<CapabilityLocalization> {
        vec![CapabilityLocalization::text(
            "uk",
            "E2B",
            "Запускайте код у хмарних пісочницях на основі E2B. Створюйте ізольовані \
             середовища Linux, виконуйте команди та керуйте файлами пісочниці.",
        )]
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
        Some("Sandboxes")
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
        assert_eq!(cap.category(), Some("Sandboxes"));
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

    #[tokio::test]
    async fn system_prompt_within_budget() {
        let cap = E2BCapability;
        let ctx = everruns_core::capabilities::SystemPromptContext::without_file_store(
            everruns_core::SessionId::new(),
        );
        let prompt = cap.system_prompt_contribution(&ctx).await.unwrap();
        // Bumped 1000 → 1300: EVE-778 grew the shared EXEC_OUTPUT_HINT with the
        // single-read/contextual-search policy (+438 bytes), taking this
        // contribution to 1255 bytes.
        assert!(prompt.len() <= 1300, "prompt is {} bytes", prompt.len());
    }
}
