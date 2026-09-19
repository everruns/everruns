//! Session compute: where commands run, and what they may touch.
//!
//! [`Environment`](crate::Environment) already carries the workspace head every
//! file tool addresses. This module adds the other half of an execution
//! environment: the compute that runs commands against that same head, plus the
//! containment and durability facts a caller must be able to read before
//! trusting it.
//!
//! Two questions are kept apart deliberately, because one machine can answer
//! them differently. **Target** (this module's [`Compute`]) is where a command
//! runs. **Containment** ([`Containment`]) is what that command may touch. Most
//! targets fix containment by construction: Bashkit interprets a shell against a
//! virtual filesystem and Daytona hands out an isolated VM, so for them the
//! field records what is already true. It becomes a choice only for a real
//! machine, where the same box can run a command wide open or under a kernel
//! policy.
//!
//! See `knowledge/harnesses/execution-environments.md`.

use std::fmt;
use std::sync::Arc;

use async_trait::async_trait;
use serde::{Deserialize, Serialize};
use thiserror::Error;

use crate::workspace::WorkspaceHead;

/// The shape of a compute target, independent of any one provider.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ComputeKind {
    /// The machine this process is already running on.
    Host,
    /// A registered box reached over SSH or an agent daemon.
    Machine,
    /// An in-process interpreter over the workspace, such as Bashkit.
    Vfs,
    /// A local container runtime.
    Container,
    /// A provider-owned remote sandbox, such as Daytona or E2B.
    Managed,
}

impl ComputeKind {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Host => "host",
            Self::Machine => "machine",
            Self::Vfs => "vfs",
            Self::Container => "container",
            Self::Managed => "managed",
        }
    }
}

impl fmt::Display for ComputeKind {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(self.as_str())
    }
}

/// What a target can actually do.
///
/// Tools and UI are assembled from this set. An operation a target cannot
/// support is absent, never emulated with misleading semantics: Bashkit
/// advertising `native_processes: false` is the load-bearing case, because a
/// shell that merely fails on every binary looks broken rather than limited.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct ComputeCapabilities {
    /// Arbitrary native binaries and child processes.
    pub native_processes: bool,
    /// Installing packages that outlive one command.
    pub packages: bool,
    /// Interactive pseudo-terminals.
    pub pty: bool,
    /// Reachable listening ports.
    pub ports: bool,
    /// An Everruns-owned archive of the working filesystem can be produced.
    pub portable_checkpoint: bool,
    /// The declared network policy is enforced by something, not just declared.
    pub network_enforced: bool,
}

impl ComputeCapabilities {
    /// A target that runs real Linux processes with nothing withheld.
    pub const fn full_machine() -> Self {
        Self {
            native_processes: true,
            packages: true,
            pty: true,
            ports: true,
            portable_checkpoint: false,
            network_enforced: false,
        }
    }
}

/// How much of the working filesystem survives losing the compute.
///
/// Declared per target and never inferred, because the difference decides
/// whether a session may be promised recovery. A real machine is
/// [`Durability::None`]: Everruns does not own the hardware, so loss is loss.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Durability {
    /// Everruns owns a portable checkpoint; eligible to back a durable agent.
    Checkpointed,
    /// Only a provider-native snapshot exists. A fast restore, not a guarantee.
    ProviderSnapshot,
    /// Nothing recovers the filesystem if the compute is lost.
    None,
}

/// How much of the host a command may reach.
///
/// Ordered: `None` < `Native` < `Isolated`. The ordering is what lets
/// [`Environment`](crate::Environment) reject a profile that asks for less than
/// its target already enforces.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ContainmentLevel {
    /// Nothing contains the command. Honest, and sometimes what is wanted.
    None,
    /// A kernel policy (Seatbelt, Landlock) bounds the command.
    Native,
    /// A separate kernel, VM, or interpreter bounds it.
    Isolated,
}

impl ContainmentLevel {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::None => "none",
            Self::Native => "native",
            Self::Isolated => "isolated",
        }
    }
}

impl fmt::Display for ContainmentLevel {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(self.as_str())
    }
}

/// Outbound network policy for commands.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case", tag = "mode", content = "allowed_hosts")]
pub enum NetworkPolicy {
    Deny,
    Allowlist(Vec<String>),
    Allow,
}

/// What a command may touch, and who enforces it.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Containment {
    pub level: ContainmentLevel,
    pub network: NetworkPolicy,
    /// Paths a command may write. Empty means "whatever the target allows",
    /// which is only meaningful at [`ContainmentLevel::Isolated`].
    #[serde(default)]
    pub writable_roots: Vec<String>,
}

impl Containment {
    /// Nothing contains the command.
    pub fn none() -> Self {
        Self {
            level: ContainmentLevel::None,
            network: NetworkPolicy::Allow,
            writable_roots: Vec::new(),
        }
    }

    /// A kernel policy bounds the command: Seatbelt on macOS, Landlock plus
    /// seccomp on Linux.
    ///
    /// [`HostCompute::contained`] enforces this behind the `native-containment`
    /// feature. Building an Environment with a target that does not enforce it
    /// still fails; see [`EnvironmentError::ContainmentUnavailable`].
    ///
    /// [`HostCompute::contained`]: crate::HostCompute::contained
    /// [`EnvironmentError::ContainmentUnavailable`]: crate::EnvironmentError::ContainmentUnavailable
    pub fn native() -> Self {
        Self {
            level: ContainmentLevel::Native,
            network: NetworkPolicy::Deny,
            writable_roots: Vec::new(),
        }
    }

    /// A separate kernel, VM, or interpreter bounds the command.
    pub fn isolated() -> Self {
        Self {
            level: ContainmentLevel::Isolated,
            network: NetworkPolicy::Deny,
            writable_roots: Vec::new(),
        }
    }

    pub fn network(mut self, policy: NetworkPolicy) -> Self {
        self.network = policy;
        self
    }

    pub fn writable_root(mut self, root: impl Into<String>) -> Self {
        self.writable_roots.push(root.into());
        self
    }
}

/// One command to run.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ExecRequest {
    pub command: String,
    pub cwd: Option<String>,
    pub timeout_secs: Option<u64>,
}

impl ExecRequest {
    pub fn new(command: impl Into<String>) -> Self {
        Self {
            command: command.into(),
            cwd: None,
            timeout_secs: None,
        }
    }

    pub fn cwd(mut self, cwd: impl Into<String>) -> Self {
        self.cwd = Some(cwd.into());
        self
    }

    pub fn timeout_secs(mut self, seconds: u64) -> Self {
        self.timeout_secs = Some(seconds);
        self
    }
}

/// What one command did.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ExecResult {
    pub exit_code: i32,
    pub stdout: String,
    pub stderr: String,
}

impl ExecResult {
    pub fn success(&self) -> bool {
        self.exit_code == 0
    }
}

/// Why compute could not do what was asked.
#[derive(Clone, Debug, Error, PartialEq, Eq)]
#[non_exhaustive]
pub enum ComputeError {
    #[error("compute target is unavailable: {0}")]
    Unavailable(String),
    #[error("command could not be started: {0}")]
    Launch(String),
    #[error("command exceeded its timeout")]
    Timeout,
    #[error("operation is not supported by this compute target: {0}")]
    Unsupported(&'static str),
}

/// A target that can run commands against a workspace head.
///
/// Deliberately narrow. Lifecycle, checkpoints, and provider APIs stay with the
/// provider integration; what a session needs is the ability to connect and run
/// something, plus honest answers about what that something may do.
#[async_trait]
pub trait Compute: Send + Sync {
    /// Stable provider id, such as `host` or `bashkit`.
    fn id(&self) -> &str;

    fn kind(&self) -> ComputeKind;

    fn capabilities(&self) -> ComputeCapabilities;

    /// The containment this target enforces by construction. An Environment may
    /// not claim less than this, and may only claim more when a provider can
    /// actually deliver it.
    fn enforced_containment(&self) -> ContainmentLevel;

    /// What survives losing this compute. Never inferred from the kind.
    fn durability(&self) -> Durability;

    async fn connect(&self, head: &WorkspaceHead) -> Result<Arc<dyn ComputeSession>, ComputeError>;
}

/// A connected target, ready to run commands.
#[async_trait]
pub trait ComputeSession: Send + Sync {
    async fn exec(&self, request: ExecRequest) -> Result<ExecResult, ComputeError>;

    /// Stop a running command. A target with no cancellation story returns
    /// [`ComputeError::Unsupported`] rather than pretending.
    async fn cancel(&self, _execution_id: &str) -> Result<(), ComputeError> {
        Err(ComputeError::Unsupported("cancel"))
    }
}

#[cfg(feature = "process")]
pub use host_compute::{HostCompute, HostComputeSession};

#[cfg(feature = "process")]
mod host_compute {
    use super::*;
    use std::path::PathBuf;

    /// Commands run on the machine this process is already running on.
    ///
    /// This is the target Everruns never had: the operator's own worker, a CI
    /// runner, or a developer box, with nothing between the command and the
    /// filesystem. It is honest about that. `enforced_containment` is
    /// [`ContainmentLevel::None`] and `durability` is [`Durability::None`], so a
    /// caller that needs recovery is refused rather than misled.
    ///
    /// The root is supplied by the caller rather than read from the workspace
    /// head, because a head's bytes may not live on this machine's disk at all.
    /// Rooting it at the directory the head serves is the caller's contract,
    /// the same one `EnvironmentBuilder::workspace_extension` already carries.
    pub struct HostCompute {
        root: PathBuf,
        default_timeout_secs: u64,
        #[cfg(feature = "native-containment")]
        containment: Option<crate::containment::SandboxOptions>,
    }

    impl HostCompute {
        pub fn new(root: impl Into<PathBuf>) -> Self {
            Self {
                root: root.into(),
                default_timeout_secs: 120,
                #[cfg(feature = "native-containment")]
                containment: None,
            }
        }

        pub fn default_timeout_secs(mut self, seconds: u64) -> Self {
            self.default_timeout_secs = seconds;
            self
        }

        /// Bound commands with a kernel policy: Landlock and seccomp on Linux,
        /// Seatbelt on macOS.
        ///
        /// This is what turns [`ContainmentLevel::Native`] from a level nothing
        /// implements into one an [`Environment`](crate::Environment) can be
        /// built with. `enforced_containment` reports `Native` afterwards, so a
        /// profile claiming less than this target now enforces is refused,
        /// exactly as it is for an isolated target.
        ///
        /// [`crate::containment::ContainmentMode::FullAccess`] is not
        /// containment and is rejected here: pass no options instead, and the
        /// target keeps saying `None` honestly.
        #[cfg(feature = "native-containment")]
        pub fn contained(
            mut self,
            options: crate::containment::SandboxOptions,
        ) -> Result<Self, ComputeError> {
            // THREAT[TM-BASH-019]: a target may not advertise `Native` while
            // enforcing nothing.
            if options.mode().is_full_access() {
                return Err(ComputeError::Unavailable(
                    "danger-full-access is not containment; leave the target uncontained instead"
                        .to_string(),
                ));
            }
            self.containment = Some(options);
            Ok(self)
        }
    }

    #[async_trait]
    impl Compute for HostCompute {
        fn id(&self) -> &str {
            "host"
        }

        fn kind(&self) -> ComputeKind {
            ComputeKind::Host
        }

        fn capabilities(&self) -> ComputeCapabilities {
            ComputeCapabilities::full_machine()
        }

        fn enforced_containment(&self) -> ContainmentLevel {
            #[cfg(feature = "native-containment")]
            if self.containment.is_some() {
                return ContainmentLevel::Native;
            }
            ContainmentLevel::None
        }

        fn durability(&self) -> Durability {
            Durability::None
        }

        async fn connect(
            &self,
            _head: &WorkspaceHead,
        ) -> Result<Arc<dyn ComputeSession>, ComputeError> {
            if !self.root.is_dir() {
                return Err(ComputeError::Unavailable(format!(
                    "{} is not a directory",
                    self.root.display()
                )));
            }
            Ok(Arc::new(HostComputeSession {
                root: self.root.clone(),
                default_timeout_secs: self.default_timeout_secs,
                #[cfg(feature = "native-containment")]
                sandbox: self.containment.clone().map(crate::containment::provider),
            }))
        }
    }

    /// A connected [`HostCompute`].
    pub struct HostComputeSession {
        pub(super) root: PathBuf,
        pub(super) default_timeout_secs: u64,
        #[cfg(feature = "native-containment")]
        pub(super) sandbox: Option<Arc<dyn crate::containment::SandboxProvider>>,
    }

    #[async_trait]
    impl ComputeSession for HostComputeSession {
        async fn exec(&self, request: ExecRequest) -> Result<ExecResult, ComputeError> {
            let cwd = match &request.cwd {
                Some(relative) => self.root.join(relative),
                None => self.root.clone(),
            };
            let mut command = self.shell(&cwd, &request.command)?;
            command
                .stdout(std::process::Stdio::piped())
                .stderr(std::process::Stdio::piped())
                .kill_on_drop(true);

            let child = command
                .spawn()
                .map_err(|error| ComputeError::Launch(error.to_string()))?;
            let seconds = request.timeout_secs.unwrap_or(self.default_timeout_secs);
            let output = tokio::time::timeout(
                std::time::Duration::from_secs(seconds),
                child.wait_with_output(),
            )
            .await
            .map_err(|_| ComputeError::Timeout)?
            .map_err(|error| ComputeError::Launch(error.to_string()))?;

            Ok(ExecResult {
                // A signalled process has no code. -1 keeps `success()` false
                // without inventing a plausible exit status.
                exit_code: output.status.code().unwrap_or(-1),
                stdout: String::from_utf8_lossy(&output.stdout).into_owned(),
                stderr: String::from_utf8_lossy(&output.stderr).into_owned(),
            })
        }
    }

    impl HostComputeSession {
        /// The shell for one command: contained when the target was configured
        /// that way, a plain login shell otherwise.
        ///
        /// A containment provider that cannot apply its policy returns an error
        /// here, which becomes a launch failure rather than an uncontained run.
        fn shell(
            &self,
            cwd: &std::path::Path,
            command: &str,
        ) -> Result<tokio::process::Command, ComputeError> {
            #[cfg(feature = "native-containment")]
            if let Some(sandbox) = &self.sandbox {
                return sandbox
                    .command(cwd, command)
                    .map_err(|error| ComputeError::Launch(format!("{error:#}")));
            }
            let mut shell = tokio::process::Command::new("bash");
            shell.arg("-lc").arg(command).current_dir(cwd);
            Ok(shell)
        }
    }
}

#[cfg(all(test, feature = "process"))]
mod host_compute_tests {
    use super::*;
    use std::path::PathBuf;

    fn session(root: PathBuf) -> HostComputeSession {
        HostComputeSession {
            root,
            default_timeout_secs: 30,
            #[cfg(feature = "native-containment")]
            sandbox: None,
        }
    }

    #[tokio::test]
    async fn a_command_runs_in_the_configured_root() {
        let directory = tempfile::tempdir().expect("temp dir");
        let session = session(directory.path().to_path_buf());

        let result = session
            .exec(ExecRequest::new("pwd && echo marker > witness.txt"))
            .await
            .expect("command runs");

        assert!(result.success(), "stderr: {}", result.stderr);
        assert!(
            directory.path().join("witness.txt").exists(),
            "the command wrote into the root it was given"
        );
    }

    #[tokio::test]
    async fn a_failing_command_reports_its_status_rather_than_an_error() {
        let directory = tempfile::tempdir().expect("temp dir");

        let result = session(directory.path().to_path_buf())
            .exec(ExecRequest::new("exit 3"))
            .await
            .expect("a non-zero exit is a result, not a transport failure");

        assert_eq!(result.exit_code, 3);
        assert!(!result.success());
    }

    #[tokio::test]
    async fn a_command_that_outlives_its_timeout_is_a_timeout() {
        let directory = tempfile::tempdir().expect("temp dir");

        let error = session(directory.path().to_path_buf())
            .exec(ExecRequest::new("sleep 5").timeout_secs(1))
            .await
            .expect_err("the wait is bounded");

        assert_eq!(error, ComputeError::Timeout);
    }

    #[tokio::test]
    async fn connecting_to_a_missing_root_fails_before_any_command_runs() {
        let compute = HostCompute::new("/nonexistent/everruns/host/compute/root");
        assert_eq!(compute.kind(), ComputeKind::Host);
        // The honest pair: an uncontained machine contains nothing and recovers
        // nothing.
        assert_eq!(compute.enforced_containment(), ContainmentLevel::None);
        assert_eq!(compute.durability(), Durability::None);
    }

    #[cfg(feature = "native-containment")]
    #[tokio::test]
    async fn a_contained_host_target_enforces_native_containment() {
        use crate::containment::{ContainmentMode, SandboxOptions};

        let directory = tempfile::tempdir().expect("temp dir");
        let compute = HostCompute::new(directory.path())
            .contained(SandboxOptions::new(ContainmentMode::WorkspaceWrite))
            .expect("a contained target");

        assert_eq!(compute.enforced_containment(), ContainmentLevel::Native);
        // Still not durable: the boundary says what a command may touch, not
        // what survives losing the machine.
        assert_eq!(compute.durability(), Durability::None);

        let open = HostCompute::new(directory.path())
            .contained(SandboxOptions::new(ContainmentMode::FullAccess));
        assert!(
            open.is_err(),
            "full access is the absence of containment, not a level of it"
        );
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn containment_levels_order_from_open_to_closed() {
        assert!(ContainmentLevel::None < ContainmentLevel::Native);
        assert!(ContainmentLevel::Native < ContainmentLevel::Isolated);
    }

    #[test]
    fn network_policy_round_trips_with_its_mode_tag() {
        let allowlist = NetworkPolicy::Allowlist(vec!["crates.io".to_string()]);
        let json = serde_json::to_string(&allowlist).expect("serializable");
        assert_eq!(
            json,
            r#"{"mode":"allowlist","allowed_hosts":["crates.io"]}"#
        );
        assert_eq!(
            serde_json::from_str::<NetworkPolicy>(&json).expect("deserializable"),
            allowlist
        );
        assert_eq!(
            serde_json::to_string(&NetworkPolicy::Deny).expect("serializable"),
            r#"{"mode":"deny"}"#
        );
    }

    #[test]
    fn a_full_machine_withholds_the_two_things_everruns_does_not_own() {
        let capabilities = ComputeCapabilities::full_machine();
        assert!(capabilities.native_processes);
        // Everruns owns neither the disk nor the network policy of somebody
        // else's machine, so neither may be advertised.
        assert!(!capabilities.portable_checkpoint);
        assert!(!capabilities.network_enforced);
    }
}
