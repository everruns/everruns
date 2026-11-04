//! boxd machines for Everruns agents.
//!
//! [boxd](https://boxd.sh) runs persistent KVM microVMs: Ubuntu 24.04 with
//! its own kernel and Docker preinstalled, a 100 GiB copy-on-write disk, and
//! sleep in place (hibernation keeps memory and running processes on disk and
//! bills for disk only). This module contributes the managed Sandboxes
//! provider `boxd` (Sandbox Template `managed` target), the `boxd` capability
//! (agent tools for extra machines), and the `boxd` connection provider.
//!
//! Decisions:
//! - Every machine Everruns creates is `isolated` and unshared: no peers in
//!   the user's boxd org, and the guest is stripped of the boxd CLI, git
//!   credential helper and agent logins. A sandbox for model-written code
//!   should not reach the user's other machines or account.
//! - A machine lives in one region, and calls on it go to that region's
//!   endpoint, so ids Everruns hands out carry the region:
//!   `<region>/<machine id>` ([`MachineRef`]).
//! - No feature flag: the provider is opt-in through a Sandbox Template and a
//!   boxd connection.
//!
//! # Example
//!
//! ```
//! use everruns_contracts::runtime::capabilities::Capability;
//! use everruns_integrations::boxd::{BoxdCapability, MachineRef};
//!
//! assert_eq!(BoxdCapability.id(), "boxd");
//! let machine = MachineRef::parse("nyc/vm_123").unwrap();
//! assert_eq!(machine.region, "nyc");
//! assert_eq!(machine.to_string(), "nyc/vm_123");
//! ```

pub mod client;
pub mod connection;
pub mod egress;
mod owned;
mod session_sandbox;
pub mod state;
mod tools;

use std::sync::LazyLock;

use everruns_contracts::connector::ConnectorPlugin;
use everruns_contracts::runtime::LEASED_RESOURCES_FEATURE;
use everruns_contracts::runtime::capabilities::{
    Capability, CapabilityLocalization, CapabilityStatus, IntegrationPlugin, RiskLevel,
};
use everruns_contracts::runtime::tools::Tool;

pub use client::BoxdClient;
pub use connection::BoxdConnector;
pub use session_sandbox::BoxdSessionSandboxProvider;

/// Check Sandbox Template `target.options` with the provider's own rules, so
/// a template cannot save what machine creation would refuse.
pub fn validate_options(options: &serde_json::Value) -> Result<(), String> {
    session_sandbox::parse_options(options).map(|_| ())
}

/// Generated protobuf types for boxd's public API (`proto/boxd/api.proto`).
#[allow(missing_docs, clippy::all, clippy::pedantic)]
pub mod proto {
    tonic::include_proto!("boxd.api.v1");
}

/// Server stubs for the in-process mock the tests run against: the same
/// messages and paths, with the service cut down to the methods the client
/// calls (see `build.rs`).
#[cfg(feature = "boxd-test-util")]
#[allow(missing_docs, clippy::all, clippy::pedantic)]
pub mod mock_proto {
    include!(concat!(env!("OUT_DIR"), "/boxd_mock/boxd.api.v1.rs"));
}

/// Connection provider and leased-resource provider ID.
pub const BOXD_PROVIDER: &str = "boxd";
/// Leased-resource type for machines.
pub const BOXD_RESOURCE_TYPE: &str = "sandbox";
/// Session-secret prefix for machine records. Reserved from agent-visible
/// session storage by `everruns-core` (`is_internal_session_secret_name`).
pub const BOXD_SANDBOX_SECRET_PREFIX: &str = "boxd_sandbox:";

/// Regions boxd runs today. `target.options.region` is checked against this
/// list when a template is saved, and against the org's live `ListRegions`
/// when a machine is created, so a region boxd adds needs one entry here.
pub const BOXD_REGIONS: &[&str] = &["fra", "nyc"];
/// Machine sizes boxd offers, `<vcpu>x<memory GiB>`.
pub const BOXD_SIZES: &[&str] = &["1x4", "2x8", "4x16", "8x32"];
/// The machine user's home, where the default workspace lives.
pub const BOXD_HOME: &str = "/home/boxd";
const BOXD_WORKSPACE_PATH: &str = "/home/boxd/workspace";
const BOXD_DEFAULT_EXEC_TIMEOUT_SECS: u32 = 120;
const BOXD_MAX_EXEC_TIMEOUT_SECS: u32 = 60 * 60;
/// Largest file the read tools return.
const BOXD_MAX_READ_FILE_BYTES: usize = 5 * 1024 * 1024;

/// Feature flags this module's plugins name: none, the provider is opt-in.
pub const FEATURE_FLAGS: &[everruns_contracts::runtime::FeatureFlagDefinition] = &[];

/// Capability plugins this module contributes to a hosted catalog.
pub const CAPABILITY_PLUGINS: &[IntegrationPlugin] = &[IntegrationPlugin {
    feature_flag: None,
    factory: || Box::new(BoxdCapability),
}];

/// Connector plugins this module contributes to a hosted catalog.
pub const CONNECTOR_PLUGINS: &[ConnectorPlugin] = &[ConnectorPlugin {
    feature_flag: None,
    factory: || Box::new(BoxdConnector),
}];

inventory::submit! {
    everruns_contracts::session_sandbox::SessionSandboxProviderPlugin {
        factory: || Box::new(BoxdSessionSandboxProvider),
    }
}

/// A machine Everruns can address: its region and boxd's machine id.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MachineRef {
    pub region: String,
    pub id: String,
}

impl MachineRef {
    /// Parse `<region>/<machine id>`. The model passes these back, so both
    /// halves are checked before they reach a request or a secret name.
    pub fn parse(value: &str) -> Result<Self, String> {
        let (region, id) = value
            .split_once('/')
            .ok_or_else(|| format!("'{value}' is not a boxd sandbox id (<region>/<machine>)"))?;
        let region_ok = !region.is_empty()
            && region.len() <= 16
            && region
                .bytes()
                .all(|b| b.is_ascii_lowercase() || b.is_ascii_digit());
        let id_ok = !id.is_empty()
            && id.len() <= 96
            && id
                .bytes()
                .all(|b| b.is_ascii_alphanumeric() || b == b'-' || b == b'_');
        if region_ok && id_ok {
            Ok(Self {
                region: region.to_string(),
                id: id.to_string(),
            })
        } else {
            Err(format!(
                "'{value}' is not a boxd sandbox id (<region>/<machine>)"
            ))
        }
    }
}

impl std::fmt::Display for MachineRef {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}/{}", self.region, self.id)
    }
}

/// A fresh machine name. Names are unique per region across all of boxd, so
/// they carry random bits rather than anything about the session.
pub fn machine_name() -> String {
    let id = uuid::Uuid::new_v4().simple().to_string();
    format!("er-{}", &id[..16])
}

static SYSTEM_PROMPT: LazyLock<String> = LazyLock::new(|| {
    let mut prompt = String::from(
        "boxd machines are Linux VMs (Ubuntu 24.04, own kernel, Docker preinstalled). Create one with boxd_create_sandbox (region fra or nyc), then pass its sandbox_id. Commands run in /home/boxd/workspace. boxd_manage_sandbox hibernate keeps running processes at disk-only cost; wake resumes them. Delete machines when done.",
    );
    prompt.push_str(everruns_contracts::runtime::tool_output_sanitizer::EXEC_OUTPUT_HINT);
    prompt
});

/// The `boxd` capability: agent tools for boxd machines.
pub struct BoxdCapability;

impl Capability for BoxdCapability {
    fn id(&self) -> &str {
        BOXD_PROVIDER
    }

    fn name(&self) -> &str {
        "boxd"
    }

    fn description(&self) -> &str {
        "Run code on boxd machines: persistent, hardware-isolated Linux VMs with their own kernel \
         and Docker preinstalled, in Frankfurt or New York. Create machines, execute commands, read \
         and write files, and hibernate them with running processes intact."
    }

    fn localizations(&self) -> Vec<CapabilityLocalization> {
        vec![CapabilityLocalization::text(
            "uk",
            "boxd",
            "Запускайте код на машинах boxd: постійних, апаратно ізольованих віртуальних машинах \
             Linux із власним ядром і встановленим Docker, у Франкфурті або Нью-Йорку. Створюйте \
             машини, виконуйте команди, читайте й записуйте файли та присипляйте їх зі збереженням \
             запущених процесів.",
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
            Box::new(tools::BoxdCreateSandboxTool),
            Box::new(tools::BoxdExecTool),
            Box::new(tools::BoxdReadFileTool),
            Box::new(tools::BoxdWriteFileTool),
            Box::new(tools::BoxdListSandboxesTool),
            Box::new(tools::BoxdManageSandboxTool),
        ]
    }

    fn dependencies(&self) -> Vec<&'static str> {
        vec!["session_storage"]
    }

    fn features(&self) -> Vec<&'static str> {
        vec![LEASED_RESOURCES_FEATURE]
    }
}

/// Exit-code hints shared by the provider and the tools.
pub(crate) fn exit_code_hint(exit_code: i32) -> Option<&'static str> {
    match exit_code {
        0 => None,
        124 => Some("Command timed out."),
        126 => Some("Command found but not executable. Check file permissions."),
        127 => Some("Command not found. Check that the tool is installed and in PATH."),
        137 => {
            Some("Process was killed (SIGKILL). This often means the process ran out of memory.")
        }
        _ if exit_code > 128 && exit_code <= 192 => Some("Process was killed by a signal."),
        _ => None,
    }
}

pub(crate) fn is_normalized_absolute_path(path: &str) -> bool {
    path.starts_with('/')
        && path != "/"
        && !path.ends_with('/')
        && !path.contains("//")
        && !path.split('/').any(|part| matches!(part, "." | ".."))
}

/// `cd` into `cwd`, then run `command` with `sh -c`, both shell-quoted.
pub(crate) fn command_in(cwd: &str, command: &str) -> String {
    format!(
        "cd -- {} && exec sh -c {}",
        client::shell_quote(cwd),
        client::shell_quote(command)
    )
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
                &format!("{BOXD_SANDBOX_SECRET_PREFIX}fra/vm_1")
            )
        );
    }

    #[test]
    fn machine_refs_are_validated() {
        assert_eq!(
            MachineRef::parse("fra/vm_abc-1").unwrap(),
            MachineRef {
                region: "fra".into(),
                id: "vm_abc-1".into()
            }
        );
        for bad in [
            "", "fra", "/vm", "fra/", "FRA/vm", "fra/vm/x", "fra/../x", "fra/a b",
        ] {
            assert!(MachineRef::parse(bad).is_err(), "{bad}");
        }
    }

    #[test]
    fn machine_names_are_short_and_distinct() {
        let (a, b) = (machine_name(), machine_name());
        assert_ne!(a, b);
        assert!(a.starts_with("er-") && a.len() == 19, "{a}");
    }

    #[test]
    fn commands_run_in_their_directory_without_shell_injection() {
        assert_eq!(
            command_in("/home/boxd/it's", "echo $HOME"),
            "cd -- '/home/boxd/it'\\''s' && exec sh -c 'echo $HOME'"
        );
    }

    #[tokio::test]
    async fn system_prompt_within_budget() {
        let ctx =
            everruns_contracts::runtime::capabilities::SystemPromptContext::without_file_store(
                everruns_contracts::typed_id::SessionId::new(),
            );
        let prompt = BoxdCapability
            .system_prompt_contribution(&ctx)
            .await
            .unwrap();
        assert!(prompt.contains("boxd_create_sandbox"));
        assert!(prompt.len() <= 1400, "prompt is {} bytes", prompt.len());
    }
}
