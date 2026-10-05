//! Modal sandboxes for Everruns agents.
//!
//! [Modal](https://modal.com) runs sandboxes in two runtimes: full Linux
//! [VM sandboxes](https://modal.com/blog/vm-sandboxes-agent-computers) (their
//! own kernel, so Docker, FUSE and kernel features work) and lighter gVisor
//! containers. This module contributes the `modal` capability (create, exec,
//! files, snapshots, tunnels) and the `modal` connection provider.
//!
//! # Example
//!
//! ```
//! use everruns_contracts::runtime::capabilities::Capability;
//! use everruns_integrations::modal::{CAPABILITY_PLUGINS, ModalCapability};
//!
//! assert_eq!(ModalCapability.id(), "modal");
//! // Experimental: registered at dev grade only.
//! assert!(CAPABILITY_PLUGINS.iter().all(|plugin| plugin.experimental_only));
//! ```

pub mod client;
pub mod connection;
pub mod egress;
mod session_sandbox;
pub mod state;
mod tools;
mod transport;

use std::sync::LazyLock;

use everruns_contracts::connector::ConnectorPlugin;
use everruns_contracts::runtime::LEASED_RESOURCES_FEATURE;
use everruns_contracts::runtime::capabilities::{
    Capability, CapabilityLocalization, CapabilityStatus, IntegrationPlugin, RiskLevel,
};
use everruns_contracts::runtime::tools::Tool;

pub use client::{ModalClient, ModalCredentials};
pub use connection::ModalConnector;
pub use session_sandbox::ModalSessionSandboxProvider;

/// Generated protobuf types for the trimmed Modal API (`proto/modal/`).
#[allow(missing_docs, clippy::all, clippy::pedantic)]
pub mod proto {
    /// `modal.client`: the control-plane service.
    pub mod client {
        tonic::include_proto!("modal.client");
    }
    /// `modal.task_command_router`: the per-task command router.
    pub mod router {
        tonic::include_proto!("modal.task_command_router");
    }
}

/// Connection provider and leased-resource provider ID.
pub const MODAL_PROVIDER: &str = "modal";
/// Leased-resource type for sandboxes.
pub const MODAL_RESOURCE_TYPE: &str = "sandbox";
/// Session-secret prefix for sandbox records. Reserved from agent-visible
/// session storage by `everruns-core` (`is_internal_session_secret_name`).
pub const MODAL_SANDBOX_SECRET_PREFIX: &str = "modal_sandbox:";
/// Modal app that groups the sandboxes Everruns creates in a workspace.
pub const MODAL_APP_NAME: &str = "everruns-sandboxes";

/// Default runtime: the full VM, which is what the integration exists for.
const MODAL_DEFAULT_RUNTIME: &str = "vm";
const MODAL_DEFAULT_IMAGE: &str = "python:3.13-slim";
/// Applied to the default image only, so a bare create still has git and curl.
/// Modal caches the built image per workspace, so this costs one build.
const MODAL_DEFAULT_SETUP_COMMANDS: &[&str] = &[
    "RUN apt-get update && apt-get install -y --no-install-recommends git curl ca-certificates && rm -rf /var/lib/apt/lists/*",
];
const MODAL_WORKSPACE_PATH: &str = "/workspace";
const MODAL_DEFAULT_TIMEOUT_SECS: u32 = 60 * 60;
/// Modal's own ceiling for sandbox lifetime.
const MODAL_MAX_TIMEOUT_SECS: u32 = 24 * 60 * 60;
const MODAL_DEFAULT_EXEC_TIMEOUT_SECS: u32 = 120;
const MODAL_MAX_EXEC_TIMEOUT_SECS: u32 = 60 * 60;

/// Capability plugins this module contributes to a hosted catalog.
///
/// Experimental (dev grade only) until it has run in production.
pub const CAPABILITY_PLUGINS: &[IntegrationPlugin] = &[IntegrationPlugin {
    experimental_only: true,
    feature_flag: None,
    factory: || Box::new(ModalCapability),
}];

/// Connector plugins this module contributes to a hosted catalog.
pub const CONNECTOR_PLUGINS: &[ConnectorPlugin] = &[ConnectorPlugin {
    experimental_only: true,
    factory: || Box::new(ModalConnector),
}];

// The managed Sandboxes provider (Sandbox Templates `managed` target, provider
// `modal`). The server offers it only at development grade while the
// integration is experimental (`sandbox_templates::resolution`).
inventory::submit! {
    everruns_contracts::session_sandbox::SessionSandboxProviderPlugin {
        factory: || Box::new(ModalSessionSandboxProvider),
    }
}

static SYSTEM_PROMPT: LazyLock<String> = LazyLock::new(|| {
    let mut prompt = String::from(
        "Modal sandboxes are cloud Linux machines. Create one with modal_create_sandbox (runtime \"vm\": full VM, Docker works; \"gvisor\": lighter), then pass its sandbox_id. Commands run via sh -c in `/workspace`. modal_snapshot_sandbox saves state; expose_ports plus modal_tunnel_urls give public URLs. Terminate sandboxes when done.",
    );
    prompt.push_str(everruns_contracts::runtime::tool_output_sanitizer::EXEC_OUTPUT_HINT);
    prompt
});

/// The `modal` capability.
pub struct ModalCapability;

impl Capability for ModalCapability {
    fn id(&self) -> &str {
        "modal"
    }

    fn name(&self) -> &str {
        "Modal"
    }

    fn description(&self) -> &str {
        "Run code in Modal sandboxes: full Linux VMs with their own kernel (Docker, FUSE, \
         databases) or lightweight gVisor containers. Create sandboxes from any registry image, \
         execute commands, read and write files, snapshot filesystems, and expose ports on public \
         URLs. EXPERIMENTAL: This capability may change."
    }

    fn localizations(&self) -> Vec<CapabilityLocalization> {
        vec![CapabilityLocalization::text(
            "uk",
            "Modal",
            "Запускайте код у пісочницях Modal: повноцінних віртуальних машинах Linux із власним \
             ядром (Docker, FUSE, бази даних) або легких контейнерах gVisor. Створюйте пісочниці з \
             будь-якого образу реєстру, виконуйте команди, читайте й записуйте файли, робіть знімки \
             файлової системи та відкривайте порти за публічними адресами.",
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
        Some("Execution")
    }

    fn system_prompt_addition(&self) -> Option<&str> {
        Some(&SYSTEM_PROMPT)
    }

    fn tools(&self) -> Vec<Box<dyn Tool>> {
        vec![
            Box::new(tools::ModalCreateSandboxTool),
            Box::new(tools::ModalExecTool),
            Box::new(tools::ModalReadFileTool),
            Box::new(tools::ModalWriteFileTool),
            Box::new(tools::ModalListSandboxesTool),
            Box::new(tools::ModalManageSandboxTool),
            Box::new(tools::ModalSnapshotSandboxTool),
            Box::new(tools::ModalTunnelUrlsTool),
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

    // Core reserves this prefix from session storage but cannot import the
    // constant (crate layering). Pin them so a rename cannot reopen forgery.
    #[test]
    fn the_sandbox_secret_prefix_is_reserved_from_session_storage() {
        assert!(
            everruns_core::host::session_services::capabilities::is_internal_session_secret_name(
                &format!("{MODAL_SANDBOX_SECRET_PREFIX}sb-example")
            )
        );
    }

    #[test]
    fn capability_metadata() {
        let cap = ModalCapability;
        assert_eq!(cap.id(), "modal");
        assert_eq!(cap.name(), "Modal");
        assert_eq!(cap.status(), CapabilityStatus::Available);
        assert_eq!(cap.risk_level(), RiskLevel::High);
        assert_eq!(cap.category(), Some("Execution"));
        assert_eq!(cap.dependencies(), vec!["session_storage"]);
        assert_eq!(cap.features(), vec![LEASED_RESOURCES_FEATURE]);
    }

    #[test]
    fn capability_has_all_tools_and_they_need_context() {
        let names: Vec<_> = ModalCapability
            .tools()
            .iter()
            .map(|t| {
                assert!(t.requires_context(), "{} should require context", t.name());
                t.name().to_string()
            })
            .collect();
        assert_eq!(
            names,
            [
                "modal_create_sandbox",
                "modal_exec",
                "modal_read_file",
                "modal_write_file",
                "modal_list_sandboxes",
                "modal_manage_sandbox",
                "modal_snapshot_sandbox",
                "modal_tunnel_urls",
            ]
        );
    }

    #[test]
    fn tool_schemas_are_closed_objects() {
        for tool in ModalCapability.tools() {
            let schema = tool.parameters_schema();
            assert_eq!(schema["type"], "object", "{}", tool.name());
            assert_eq!(schema["additionalProperties"], false, "{}", tool.name());
        }
    }

    #[tokio::test]
    async fn system_prompt_within_budget() {
        let ctx =
            everruns_contracts::runtime::capabilities::SystemPromptContext::without_file_store(
                everruns_contracts::typed_id::SessionId::new(),
            );
        let prompt = ModalCapability
            .system_prompt_contribution(&ctx)
            .await
            .unwrap();
        assert!(prompt.contains("modal_create_sandbox"));
        assert!(prompt.len() <= 1400, "prompt is {} bytes", prompt.len());
    }

    #[test]
    fn plugins_are_experimental() {
        assert!(
            CAPABILITY_PLUGINS
                .iter()
                .all(|p| p.experimental_only && p.feature_flag.is_none())
        );
        assert!(CONNECTOR_PLUGINS.iter().all(|p| p.experimental_only));
        assert_eq!(
            (CONNECTOR_PLUGINS[0].factory)().provider_id(),
            MODAL_PROVIDER
        );
    }
}
