//! The `bash` tool over real host processes, contained by the kernel.
//!
//! Everruns already had two shells that could not be the third. `bashkit_shell`
//! interprets a script against the session filesystem with no host underneath
//! it, which is exactly right until an agent needs `cargo`, `git`, or anything
//! else that is a binary. The remote sandboxes (Daytona, E2B, Deno) run real
//! binaries, on a machine somewhere else. This capability is the case in
//! between, the one Yolop has shipped all along: real processes, on the machine
//! the agent is already running on, bounded by a kernel policy rather than by a
//! promise.
//!
//! The boundary itself is [`crate::containment`]. This module is the
//! agent-facing half: the capability, its configuration, and the tool.
//!
//! It lives in `everruns-host` rather than an integration crate because it is
//! an *embedder* capability, not a hosted-product one. Running arbitrary host
//! processes is something a CLI host, a CI runner, or an operator's own box
//! opts into; it is not something a shared multi-tenant worker should offer.
//! `everruns-host` is where an embedder composes its runtime, so that is where
//! this belongs, next to [`HostCompute`](crate::HostCompute), which contains
//! commands under the same policy.
//!
//! # Choosing between this and `bashkit_shell`
//!
//! Both contribute a tool named `bash` over the session workspace, so enable
//! one or the other, not both. Pick `bashkit_shell` when the workspace is
//! virtual and the scripts are shell-shaped work over files. Pick `host_shell`
//! when the agent needs the machine: a toolchain, a package manager, a test
//! suite. It requires a real-disk session filesystem, and says so rather than
//! pretending when it gets a virtual one.
//!
//! # Example
//!
//! ```
//! use everruns_core::capabilities::Capability;
//! use everruns_host::host_shell::{HostShell, HostShellCapability};
//!
//! assert_eq!(HostShellCapability.id(), "host_shell");
//! assert_eq!(HostShellCapability.tools().len(), 1);
//!
//! // The default is contained, not convenient.
//! let shell = HostShell::new();
//! assert_eq!(
//!     shell.containment_mode(),
//!     everruns_host::containment::ContainmentMode::WorkspaceWrite
//! );
//! ```

mod approval;
mod config;
mod tool;

use async_trait::async_trait;
use everruns_core::capabilities::{
    Capability, CapabilityLocalization, CapabilityStatus, RiskLevel, SystemPromptContext,
};
use everruns_core::tools::Tool;
use serde_json::Value;

use crate::containment::{ContainmentMode, SandboxLauncher, danger_warning, network_access};

pub use approval::{HostShellApproval, ShellApprovalGate, ShellApprovalRequest};
pub use config::{ApprovalPolicy, HostShellConfig};
pub use tool::BashTool;

/// The capability reference id.
pub const HOST_SHELL_CAPABILITY_ID: &str = "host_shell";

/// Agent-facing host shell configuration.
///
/// ```
/// use everruns_host::containment::ContainmentMode;
/// use everruns_host::host_shell::{ApprovalPolicy, HostShell};
///
/// let shell = HostShell::new()
///     .containment(ContainmentMode::ReadOnly)
///     .approval(ApprovalPolicy::OnRequest)
///     .writable_root("/var/cache/agent");
///
/// assert_eq!(shell.containment_mode(), ContainmentMode::ReadOnly);
/// ```
#[derive(Clone, Debug, Default)]
pub struct HostShell {
    containment: Option<ContainmentMode>,
    approval: Option<ApprovalPolicy>,
    writable_roots: Vec<String>,
    launcher: Option<SandboxLauncher>,
}

impl HostShell {
    /// The default configuration: workspace-write containment, no approvals.
    pub fn new() -> Self {
        Self::default()
    }

    /// Select what commands may touch.
    pub fn containment(mut self, containment: ContainmentMode) -> Self {
        self.containment = Some(containment);
        self
    }

    /// Select when a human is asked.
    pub fn approval(mut self, approval: ApprovalPolicy) -> Self {
        self.approval = Some(approval);
        self
    }

    /// Add a directory commands may write beyond the workspace.
    pub fn writable_root(mut self, root: impl Into<String>) -> Self {
        self.writable_roots.push(root.into());
        self
    }

    /// Select how the Linux helper process is launched.
    ///
    /// The default looks for `everruns-sandbox-exec` beside the current
    /// executable, then on `PATH`. A single-binary embedder has neither, because
    /// cargo does not build a dependency's binaries: it routes the arguments
    /// back into [`containment::worker::run_from_args`] from its own `main` and
    /// names that here.
    ///
    /// ```no_run
    /// # use everruns_host::containment::SandboxLauncher;
    /// # use everruns_host::host_shell::HostShell;
    /// // In `main`, before anything else runs:
    /// let mut arguments = std::env::args().skip(1);
    /// if arguments.next().as_deref() == Some("__sandbox-exec") {
    ///     everruns_host::containment::worker::run_from_args(arguments)?;
    /// }
    ///
    /// let shell = HostShell::new()
    ///     .launcher(SandboxLauncher::ReexecSelf(vec!["__sandbox-exec".into()]));
    /// # let _ = shell;
    /// # Ok::<(), anyhow::Error>(())
    /// ```
    ///
    /// [`containment::worker::run_from_args`]: crate::containment::worker::run_from_args
    pub fn launcher(mut self, launcher: SandboxLauncher) -> Self {
        self.launcher = Some(launcher);
        self
    }
}

/// Accessors, named without the builder's `self`-consuming shape so a caller
/// can read back what a value says.
impl HostShell {
    /// The configured containment, or the default.
    pub fn containment_mode(&self) -> ContainmentMode {
        self.containment.unwrap_or_default()
    }

    /// The configured approval policy, or the default.
    pub fn approval_policy(&self) -> ApprovalPolicy {
        self.approval.unwrap_or_default()
    }
}

impl everruns_capability::IntoCapability for HostShell {
    fn into_capability(self) -> everruns_capability::CapabilitySpec {
        let mut config = serde_json::Map::new();
        if let Some(containment) = self.containment {
            config.insert("containment".into(), containment.as_str().into());
        }
        if let Some(approval) = self.approval {
            config.insert("approval".into(), approval.as_str().into());
        }
        if !self.writable_roots.is_empty() {
            config.insert("writable_roots".into(), self.writable_roots.into());
        }
        if let Some(launcher) = self.launcher {
            config.insert(
                "launcher".into(),
                match launcher {
                    SandboxLauncher::Discover => Value::from("discover"),
                    SandboxLauncher::Helper(path) => {
                        serde_json::json!({ "helper": path.to_string_lossy() })
                    }
                    SandboxLauncher::ReexecSelf(arguments) => {
                        serde_json::json!({ "reexec_self": arguments })
                    }
                },
            );
        }
        everruns_capability::CapabilityRef::new(HOST_SHELL_CAPABILITY_ID)
            .config(Value::Object(config))
            .into()
    }
}

/// Run commands on the machine hosting the agent, bounded by a kernel policy.
pub struct HostShellCapability;

#[async_trait]
impl Capability for HostShellCapability {
    fn id(&self) -> &str {
        HOST_SHELL_CAPABILITY_ID
    }

    fn name(&self) -> &str {
        "Host Shell"
    }

    fn description(&self) -> &str {
        r#"Run bash commands on the machine hosting this agent.

> [!NOTE]
> Unlike the sandboxed Bashkit shell, these are real processes with a real
> toolchain: compilers, package managers and test runners work. A kernel policy
> (Landlock and seccomp on Linux, Seatbelt on macOS) bounds what they may write
> and whether they may reach the network.

> [!WARNING]
> The workspace must be a real directory on that machine, and whoever runs the
> agent owns the consequences of what it executes there. Setting containment to
> `danger-full-access` removes the boundary entirely."#
    }

    fn localizations(&self) -> Vec<CapabilityLocalization> {
        vec![CapabilityLocalization::text(
            "uk",
            "Оболонка хоста",
            r#"Виконуйте bash-команди на машині, де працює цей агент.

> [!NOTE]
> На відміну від пісочниці Bashkit, це справжні процеси зі справжнім
> інструментарієм: компілятори, менеджери пакетів і запуск тестів працюють.
> Політика ядра (Landlock і seccomp на Linux, Seatbelt на macOS) обмежує, куди
> вони можуть писати і чи мають доступ до мережі.

> [!WARNING]
> Робоча тека має бути справжньою текою на цій машині, і відповідальність за
> виконане несе той, хто запускає агента. Режим `danger-full-access` знімає
> обмеження повністю."#,
        )]
    }

    fn status(&self) -> CapabilityStatus {
        CapabilityStatus::Available
    }

    fn risk_level(&self) -> RiskLevel {
        RiskLevel::High
    }

    fn icon(&self) -> Option<&str> {
        Some("terminal")
    }

    fn category(&self) -> Option<&str> {
        Some("Execution")
    }

    fn tools(&self) -> Vec<Box<dyn Tool>> {
        vec![Box::new(BashTool::default())]
    }

    fn tools_with_config(&self, config: &Value) -> Vec<Box<dyn Tool>> {
        // A config that failed validation cannot widen the boundary: fall back
        // to the contained default rather than to what the bad value asked for.
        let parsed = HostShellConfig::from_json(config).unwrap_or_default();
        vec![Box::new(BashTool::new(parsed))]
    }

    fn system_prompt_addition(&self) -> Option<&str> {
        Some(
            "The `bash` tool runs real processes on the machine hosting you, rooted at the \
             workspace. Each call is a fresh shell, so nothing persists between calls. The \
             result states the containment that was in force; a write outside the workspace or \
             an outbound connection may be refused by the kernel rather than by the tool.",
        )
    }

    async fn system_prompt_contribution_with_config(
        &self,
        _context: &SystemPromptContext,
        config: &Value,
    ) -> Option<String> {
        // Derived from the configuration rather than written by hand, so the
        // prompt cannot disagree with the boundary the commands actually run
        // under.
        let parsed = HostShellConfig::from_json(config).ok()?;
        let mut lines = vec![format!(
            "Shell containment: {} (network {}).",
            parsed.containment.as_str(),
            network_access(parsed.containment)
        )];
        if parsed.containment == ContainmentMode::ReadOnly {
            lines.push(
                "Writes to the workspace will fail. Report what a change should be rather than \
                 attempting it."
                    .to_string(),
            );
        }
        if let Some(warning) = danger_warning(parsed.containment) {
            lines.push(warning.to_string());
        }
        Some(lines.join(" "))
    }

    fn config_schema(&self) -> Option<Value> {
        Some(serde_json::json!({
            "type": "object",
            "properties": {
                "containment": {
                    "type": "string",
                    "enum": ["read-only", "workspace-write", "danger-full-access"],
                    "title": "What commands may touch",
                    "description": "read-only reads the host and writes nothing; \
                                    workspace-write adds the workspace, /tmp and any configured \
                                    roots; danger-full-access removes the boundary.",
                    "default": "workspace-write"
                },
                "approval": {
                    "type": "string",
                    "enum": ["never", "on-failure", "on-request", "untrusted"],
                    "title": "When a human is asked",
                    "description": "Requires the host to supply an approval gate. Without one, \
                                    any policy that would ask refuses instead.",
                    "default": "never"
                },
                "writable_roots": {
                    "type": "array",
                    "items": {"type": "string"},
                    "title": "Extra writable directories",
                    "description": "Caches and tool state a build needs to write, beyond the \
                                    workspace."
                },
                "launcher": {
                    "title": "How the Linux helper process is launched",
                    "description": "\"discover\" finds everruns-sandbox-exec beside the binary \
                                    or on PATH. {\"helper\": path} names it. \
                                    {\"reexec_self\": [args]} re-execs this binary with those \
                                    leading arguments, which it must route into the containment \
                                    worker.",
                    "default": "discover"
                },
                "foreground_timeout_secs": {"type": "integer", "minimum": 1, "default": 120},
                "background_timeout_secs": {"type": "integer", "minimum": 1, "default": 86400},
                "max_output_bytes": {"type": "integer", "minimum": 1, "default": 1048576}
            },
            "additionalProperties": false
        }))
    }

    fn validate_config(&self, config: &Value) -> Result<(), String> {
        HostShellConfig::from_json(config).map(|_| ())
    }

    fn dependencies(&self) -> Vec<&'static str> {
        vec!["session_file_system"]
    }

    fn features(&self) -> Vec<&'static str> {
        vec!["file_system"]
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use everruns_capability::IntoCapability;
    use everruns_provider::typed_id::SessionId;
    use serde_json::json;

    #[test]
    fn the_capability_declares_the_workspace_it_needs() {
        assert_eq!(
            HostShellCapability.dependencies(),
            vec!["session_file_system"]
        );
        assert_eq!(HostShellCapability.risk_level(), RiskLevel::High);
        assert_eq!(HostShellCapability.tools()[0].name(), "bash");
    }

    #[test]
    fn a_typed_value_becomes_the_config_the_capability_reads() {
        let spec = HostShell::new()
            .containment(ContainmentMode::ReadOnly)
            .approval(ApprovalPolicy::Untrusted)
            .writable_root("/cache")
            .into_capability();
        let config = spec.capability_ref().config_value().clone();

        assert_eq!(config["containment"], json!("read-only"));
        assert_eq!(config["approval"], json!("untrusted"));
        assert_eq!(config["writable_roots"], json!(["/cache"]));
        HostShellCapability
            .validate_config(&config)
            .expect("what the builder writes must be what the capability accepts");
    }

    #[test]
    fn a_single_binary_embedder_can_name_itself_as_the_launcher() {
        let spec = HostShell::new()
            .launcher(SandboxLauncher::ReexecSelf(vec!["__sandbox-exec".into()]))
            .into_capability();
        let config = spec.capability_ref().config_value().clone();

        assert_eq!(
            config["launcher"],
            json!({"reexec_self": ["__sandbox-exec"]})
        );
        HostShellCapability
            .validate_config(&config)
            .expect("what the builder writes must be what the capability accepts");
        assert_eq!(
            HostShellConfig::from_json(&config)
                .expect("parses")
                .launcher,
            SandboxLauncher::ReexecSelf(vec!["__sandbox-exec".to_string()])
        );
    }

    #[test]
    fn an_unconfigured_value_writes_nothing_and_still_validates() {
        let spec = HostShell::new().into_capability();
        let config = spec.capability_ref().config_value().clone();
        assert_eq!(config, json!({}));
        HostShellCapability.validate_config(&config).expect("valid");
    }

    #[test]
    fn a_rejected_config_falls_back_to_the_contained_default() {
        let bad = json!({"containment": "none"});
        assert!(HostShellCapability.validate_config(&bad).is_err());
        // Still exactly one tool, built from defaults rather than from the
        // value that failed.
        assert_eq!(HostShellCapability.tools_with_config(&bad).len(), 1);
    }

    #[tokio::test]
    async fn the_prompt_states_the_boundary_it_runs_under() {
        let context = SystemPromptContext::without_file_store(SessionId::new());
        let read_only = HostShellCapability
            .system_prompt_contribution_with_config(&context, &json!({"containment": "read-only"}))
            .await
            .expect("a contribution");
        assert!(read_only.contains("read-only"));
        assert!(read_only.contains("Writes to the workspace will fail"));

        let open = HostShellCapability
            .system_prompt_contribution_with_config(
                &context,
                &json!({"containment": "danger-full-access"}),
            )
            .await
            .expect("a contribution");
        assert!(open.contains("DANGER") || open.contains("WARNING"));
    }
}
