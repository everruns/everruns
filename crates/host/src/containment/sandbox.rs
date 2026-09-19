//! Pluggable containment for arbitrary child-process execution.
//!
//! Structured file tools keep going through the trusted host broker. Every
//! shell entry point receives one of these providers instead, so foreground,
//! background, and interactive commands share the same kernel boundary.

use std::path::{Path, PathBuf};
use std::process::Stdio;

use anyhow::{Context, Result};
use tokio::process::Command;

/// How much of the host a command may reach.
///
/// Ordered from most contained to least, matching
/// `everruns_host::ContainmentLevel`'s direction of travel. The string forms
/// are the wire names Yolop already uses, so a config written for one runs on
/// the other.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, PartialOrd, Ord)]
pub enum ContainmentMode {
    /// Reads anywhere, writes only to the private temp directory.
    ReadOnly,
    /// Adds the workspace, `/tmp`, and any configured extra roots to the
    /// writable set. The default: it is what a coding agent needs and nothing
    /// more.
    #[default]
    WorkspaceWrite,
    /// No containment. Named for what it is so nobody picks it by accident.
    FullAccess,
}

impl ContainmentMode {
    /// The wire name, shared with Yolop's `sandbox_mode` config.
    pub fn as_str(self) -> &'static str {
        match self {
            Self::ReadOnly => "read-only",
            Self::WorkspaceWrite => "workspace-write",
            Self::FullAccess => "danger-full-access",
        }
    }

    /// Parse a wire name. Unknown values are rejected rather than defaulted:
    /// a typo in a containment setting must not quietly widen or narrow it.
    pub fn parse(value: &str) -> Option<Self> {
        match value {
            "read-only" => Some(Self::ReadOnly),
            "workspace-write" => Some(Self::WorkspaceWrite),
            "danger-full-access" => Some(Self::FullAccess),
            _ => None,
        }
    }

    /// Whether this mode leaves the command uncontained.
    pub fn is_full_access(self) -> bool {
        self == Self::FullAccess
    }
}

impl std::fmt::Display for ContainmentMode {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter.write_str(self.as_str())
    }
}

/// How the Linux restrictions get applied.
///
/// Landlock and seccomp must be installed in a process that has not yet execed
/// the shell, and installing them from `pre_exec` would allocate and lock after
/// a fork of a multi-threaded runtime. So a helper process does it: it
/// restricts itself, then execs bash.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub enum SandboxLauncher {
    /// Look for `everruns-sandbox-exec` next to the current executable, then on
    /// `PATH`. This package ships that binary.
    #[default]
    Discover,
    /// Run a helper binary at a known path.
    Helper(PathBuf),
    /// Re-exec the current executable with these leading arguments, which must
    /// route into [`worker::run`](crate::containment::worker::run). This is how a single-binary
    /// embedder (Yolop's `__sandbox-exec`) avoids shipping a second file.
    ReexecSelf(Vec<String>),
}

/// Everything a provider needs that model input may not choose.
#[derive(Clone, Debug)]
pub struct SandboxOptions {
    mode: ContainmentMode,
    writable_roots: Vec<PathBuf>,
    launcher: SandboxLauncher,
    temp_tag: String,
}

impl SandboxOptions {
    /// Options for `mode`, with no extra writable roots and the default launcher.
    pub fn new(mode: ContainmentMode) -> Self {
        Self {
            mode,
            writable_roots: Vec::new(),
            launcher: SandboxLauncher::default(),
            temp_tag: "everruns".to_string(),
        }
    }

    /// Add a directory to the writable set, beyond the workspace and temp.
    ///
    /// For caches and tool state a build needs to write: a Cargo registry, a
    /// skills directory. Ignored at [`ContainmentMode::ReadOnly`], where only
    /// the private temp is writable.
    pub fn writable_root(mut self, root: impl Into<PathBuf>) -> Self {
        self.writable_roots.push(root.into());
        self
    }

    /// Select how Linux restrictions are applied. See [`SandboxLauncher`].
    pub fn launcher(mut self, launcher: SandboxLauncher) -> Self {
        self.launcher = launcher;
        self
    }

    /// Name the private temp directory after the embedder rather than `everruns`.
    pub fn temp_tag(mut self, tag: impl Into<String>) -> Self {
        self.temp_tag = tag.into();
        self
    }

    /// The configured mode.
    pub fn mode(&self) -> ContainmentMode {
        self.mode
    }

    /// The configured extra writable roots.
    pub fn writable_roots(&self) -> &[PathBuf] {
        &self.writable_roots
    }
}

/// A source of contained shell processes.
pub trait SandboxProvider: Send + Sync {
    /// The containment this provider applies.
    fn mode(&self) -> ContainmentMode;

    /// A command that runs `script` in `cwd` under this containment.
    ///
    /// Fails rather than returning an uncontained command when the OS primitive
    /// the mode needs is unavailable.
    fn command(&self, cwd: &Path, script: &str) -> Result<Command>;
}

/// The provider for `options`.
pub fn provider(options: SandboxOptions) -> std::sync::Arc<dyn SandboxProvider> {
    match options.mode {
        ContainmentMode::ReadOnly | ContainmentMode::WorkspaceWrite => {
            std::sync::Arc::new(NativeSandbox { options })
        }
        ContainmentMode::FullAccess => std::sync::Arc::new(UnsafeHost),
    }
}

/// The warning a host must show for `mode`, if any.
///
/// Windows has no implementation, so every mode there runs uncontained and
/// warns regardless of what was configured. Saying so is the fail-closed rule
/// honored in the only way a missing primitive allows.
pub fn danger_warning(mode: ContainmentMode) -> Option<&'static str> {
    #[cfg(windows)]
    {
        let _ = mode;
        Some(
            "WARNING: kernel containment is not available on Windows — shell commands run \
             uncontained with full access to your files, processes, and the network",
        )
    }
    #[cfg(not(windows))]
    {
        mode.is_full_access().then_some(
            "DANGER: danger-full-access — shell commands can access and modify files, \
             processes, and the network outside the workspace",
        )
    }
}

/// Network availability of the shell process this mode launches.
pub fn network_access(mode: ContainmentMode) -> &'static str {
    #[cfg(windows)]
    {
        let _ = mode;
        "enabled"
    }
    #[cfg(any(target_os = "macos", target_os = "linux"))]
    {
        if mode.is_full_access() {
            "enabled"
        } else {
            "disabled"
        }
    }
    #[cfg(not(any(target_os = "macos", target_os = "linux", windows)))]
    {
        if mode.is_full_access() {
            "enabled"
        } else {
            "unavailable"
        }
    }
}

/// Pipe both output streams and reap the child when its future is dropped.
///
/// A timed-out or canceled command leaves no orphan holding the workspace open.
pub fn configure_stdio(command: &mut Command) {
    command
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .kill_on_drop(true);
}

/// The base shell invocation for this platform, the single place the host shell
/// is chosen. Unix uses a bash login shell; Windows uses PowerShell.
fn shell_command(cwd: &Path, script: &str) -> Command {
    #[cfg(windows)]
    {
        // Windows PowerShell 5.1 ships in-box, so it needs no install step.
        // `-Command` runs inline text and is exempt from the script-file
        // execution policy. If argument quoting ever bites, `-EncodedCommand`
        // (base64 UTF-16LE) is the hardening path.
        let mut command = Command::new("powershell.exe");
        command
            .arg("-NoProfile")
            .arg("-NonInteractive")
            .arg("-Command")
            .arg(script)
            .current_dir(cwd);
        command
    }
    #[cfg(not(windows))]
    {
        let mut command = Command::new("bash");
        command.arg("-lc").arg(script).current_dir(cwd);
        command
    }
}

struct UnsafeHost;

impl SandboxProvider for UnsafeHost {
    fn mode(&self) -> ContainmentMode {
        ContainmentMode::FullAccess
    }

    fn command(&self, cwd: &Path, script: &str) -> Result<Command> {
        Ok(shell_command(cwd, script))
    }
}

struct NativeSandbox {
    options: SandboxOptions,
}

impl SandboxProvider for NativeSandbox {
    fn mode(&self) -> ContainmentMode {
        self.options.mode
    }

    fn command(&self, cwd: &Path, script: &str) -> Result<Command> {
        native_command(cwd, script, &self.options)
    }
}

#[cfg(target_os = "macos")]
fn native_command(cwd: &Path, script: &str, options: &SandboxOptions) -> Result<Command> {
    let executable = Path::new("/usr/bin/sandbox-exec");
    // THREAT[TM-BASH-019]: a missing OS primitive is an error, never a silent
    // fall back to an uncontained host process.
    if !executable.is_file() {
        anyhow::bail!(
            "native containment unavailable: /usr/bin/sandbox-exec is missing; refusing to run \
             uncontained. Select `danger-full-access` only inside an already isolated environment"
        )
    }

    // Seatbelt denies network and all writes by default. Reads stay available
    // so compilers, SDKs and package caches keep working; the workspace, the
    // private temp, and the conventional shared /tmp are writable for tool
    // compatibility. /dev/null is the sole writable device. The explicit
    // /private/tmp spelling covers macOS canonicalization through the /tmp
    // symlink.
    let temp = sandbox_temp_dir(&options.temp_tag)?;
    let home = sandbox_home_dir(&temp)?;
    let profile = macos_profile(cwd, &temp, options)?;
    let mut command = Command::new(executable);
    command
        .arg("-p")
        .arg(profile)
        .arg("/bin/bash")
        .arg("-lc")
        .arg(script)
        .current_dir(cwd);
    apply_native_environment(&mut command, &home, &temp);
    Ok(command)
}

#[cfg(target_os = "linux")]
fn native_command(cwd: &Path, script: &str, options: &SandboxOptions) -> Result<Command> {
    let temp = sandbox_temp_dir(&options.temp_tag)?;
    let home = sandbox_home_dir(&temp)?;
    let (program, leading) = resolve_launcher(&options.launcher)?;
    let mut command = Command::new(program);
    command.args(leading);
    command
        .arg("--cwd")
        .arg(cwd)
        .arg("--temp")
        .arg(&temp)
        .arg("--mode")
        .arg(options.mode.as_str());
    for root in prepared_writable_roots(options)? {
        command.arg("--writable").arg(root);
    }
    // Last, so an embedder reading a process listing sees the fixed arguments
    // before the free-form script.
    command.arg("--script").arg(script).current_dir(cwd);
    apply_native_environment(&mut command, &home, &temp);
    Ok(command)
}

/// Resolve a launcher to the program to run and the arguments that route into
/// the worker.
#[cfg(target_os = "linux")]
fn resolve_launcher(launcher: &SandboxLauncher) -> Result<(PathBuf, Vec<String>)> {
    const HELPER: &str = "everruns-sandbox-exec";
    match launcher {
        SandboxLauncher::Helper(path) => Ok((path.clone(), Vec::new())),
        SandboxLauncher::ReexecSelf(arguments) => Ok((
            std::env::current_exe().context("resolve the current executable to re-exec")?,
            arguments.clone(),
        )),
        SandboxLauncher::Discover => {
            let sibling = std::env::current_exe()
                .ok()
                .and_then(|exe| exe.parent().map(|dir| dir.join(HELPER)))
                .filter(|path| path.is_file());
            if let Some(path) = sibling {
                return Ok((path, Vec::new()));
            }
            // THREAT[TM-BASH-024]: a bare name lets the OS search PATH, which
            // is the agent process's own PATH rather than model input. Checking
            // is_file() here would race the exec anyway, and the launch error
            // names the binary clearly enough.
            Ok((PathBuf::from(HELPER), Vec::new()))
        }
    }
}

/// Create the configured extra writable roots and canonicalize them.
///
/// Landlock needs an existing directory for every rule, and canonicalizing here
/// means a symlinked root cannot widen the boundary past what was configured.
///
/// THREAT[TM-BASH-023]
#[cfg(any(target_os = "linux", target_os = "macos"))]
fn prepared_writable_roots(options: &SandboxOptions) -> Result<Vec<PathBuf>> {
    if options.mode != ContainmentMode::WorkspaceWrite {
        return Ok(Vec::new());
    }
    options
        .writable_roots
        .iter()
        .map(|root| {
            std::fs::create_dir_all(root)
                .with_context(|| format!("create writable root: {}", root.display()))?;
            std::fs::canonicalize(root)
                .with_context(|| format!("canonicalize writable root: {}", root.display()))
        })
        .collect()
}

#[cfg(target_os = "windows")]
fn native_command(cwd: &Path, script: &str, _options: &SandboxOptions) -> Result<Command> {
    // THREAT[TM-BASH-025]: Windows has no containment implementation yet. Rather than refuse to run,
    // execute uncontained: the caller is warned at every mode via
    // `danger_warning`, which is the honest form of fail-closed here.
    Ok(shell_command(cwd, script))
}

#[cfg(not(any(target_os = "macos", target_os = "linux", target_os = "windows")))]
fn native_command(_cwd: &Path, _script: &str, _options: &SandboxOptions) -> Result<Command> {
    // THREAT[TM-BASH-019]: fail closed rather than run an unbounded command.
    anyhow::bail!(
        "native containment is supported only on macOS and Linux; refusing to run uncontained. \
         Select `danger-full-access` only inside an already isolated environment"
    )
}

/// A per-process private temp directory, mode 0700.
#[cfg(not(target_os = "windows"))]
fn sandbox_temp_dir(tag: &str) -> Result<PathBuf> {
    let path = std::env::temp_dir().join(format!("{tag}-sandbox-{}", std::process::id()));
    prepare_sandbox_temp(&path)?;
    std::fs::canonicalize(&path)
        .with_context(|| format!("canonicalize sandbox temp directory: {}", path.display()))
}

#[cfg(not(target_os = "windows"))]
fn prepare_sandbox_temp(path: &Path) -> Result<()> {
    use std::io::ErrorKind;
    use std::os::unix::fs::PermissionsExt;

    match std::fs::create_dir(path) {
        Ok(()) => {}
        Err(error) if error.kind() == ErrorKind::AlreadyExists => {
            let metadata = std::fs::symlink_metadata(path)
                .with_context(|| format!("inspect sandbox temp directory: {}", path.display()))?;
            if !metadata.file_type().is_dir() {
                anyhow::bail!(
                    "sandbox temp path is not a directory (possible path-alias attack): {}",
                    path.display()
                );
            }
        }
        Err(error) => {
            return Err(error)
                .with_context(|| format!("create sandbox temp directory: {}", path.display()));
        }
    }
    std::fs::set_permissions(path, std::fs::Permissions::from_mode(0o700))
        .with_context(|| format!("secure sandbox temp directory: {}", path.display()))?;
    Ok(())
}

#[cfg(not(target_os = "windows"))]
fn sandbox_home_dir(temp: &Path) -> Result<PathBuf> {
    let path = temp.join("home");
    std::fs::create_dir_all(&path)
        .with_context(|| format!("create sandbox home directory: {}", path.display()))?;
    Ok(path)
}

/// Hand the child a sanitized environment: toolchain and locale variables
/// survive, everything else is dropped.
///
/// The allowlist is the point. Credential-bearing variables and agent socket
/// paths are what a contained command must not inherit, and an allowlist cannot
/// be outgrown by a new secret-shaped variable name the way a denylist can.
#[cfg(not(target_os = "windows"))]
fn apply_native_environment(command: &mut Command, home: &Path, temp: &Path) {
    command.env_clear();
    for (key, value) in std::env::vars_os() {
        if safe_environment_key(&key.to_string_lossy()) {
            command.env(key, value);
        }
    }
    for (key, value) in toolchain_homes() {
        command.env(key, value);
    }
    command.env("HOME", home).env("TMPDIR", temp);
}

/// Toolchain directories that default to a path under `HOME`, pinned before
/// `HOME` is replaced.
///
/// A rustup install leaves `CARGO_HOME` and `RUSTUP_HOME` unset and resolves
/// them from `HOME`. Rewriting `HOME` to the private temp would therefore point
/// `cargo` at an empty directory and break every Rust command in a contained
/// shell, for no security gain: the paths are readable either way, and neither
/// is writable unless a caller names it a writable root. Naming them
/// explicitly keeps the allowlist's promise (no credential-bearing variable is
/// inherited) while leaving the toolchain usable.
#[cfg(not(target_os = "windows"))]
fn toolchain_homes() -> Vec<(&'static str, PathBuf)> {
    let Some(home) = std::env::var_os("HOME").map(PathBuf::from) else {
        return Vec::new();
    };
    [("CARGO_HOME", ".cargo"), ("RUSTUP_HOME", ".rustup")]
        .into_iter()
        .filter(|(key, _)| std::env::var_os(key).is_none())
        .map(|(key, directory)| (key, home.join(directory)))
        .filter(|(_, path)| path.is_dir())
        .collect()
}

#[cfg(not(target_os = "windows"))]
fn safe_environment_key(key: &str) -> bool {
    matches!(
        key,
        "PATH"
            | "LANG"
            | "LC_ALL"
            | "LC_CTYPE"
            | "TERM"
            | "COLORTERM"
            | "NO_COLOR"
            | "FORCE_COLOR"
            | "CARGO_HOME"
            | "RUSTUP_HOME"
            | "SDKROOT"
            | "DEVELOPER_DIR"
            | "PKG_CONFIG_PATH"
            | "CPATH"
            | "LIBRARY_PATH"
            | "C_INCLUDE_PATH"
            | "CPLUS_INCLUDE_PATH"
            | "JAVA_HOME"
            | "GOPATH"
            | "GOROOT"
    ) || key.starts_with("LC_")
}

/// Yolop's Seatbelt profile, carried over intact.
///
/// Host *reads* are allowed for toolchain compatibility, so this policy stops
/// writes and network exfiltration rather than reading of unrelated files; that
/// limit is part of the stated threat model, not an implied guarantee. `.git`
/// below the workspace stays read-only, which Landlock cannot express (its path
/// rules are additive), so the two platforms differ here by construction.
#[cfg(target_os = "macos")]
fn macos_profile(cwd: &Path, temp: &Path, options: &SandboxOptions) -> Result<String> {
    let mut writable = String::new();
    if options.mode == ContainmentMode::WorkspaceWrite {
        writable.push_str(&format!(" (subpath \"{}\")", seatbelt_escape(cwd)));
        writable.push_str(" (subpath \"/tmp\") (subpath \"/private/tmp\")");
        for root in prepared_writable_roots(options)? {
            writable.push_str(&format!(" (subpath \"{}\")", seatbelt_escape(&root)));
        }
    }
    let git = seatbelt_escape(&cwd.join(".git"));
    Ok(format!(
        "(version 1)\n(deny default)\n(allow process*)\n(allow file-read*)\n\
         (allow sysctl-read)\n(allow mach-lookup)\n\
         (allow file-write-data (require-all (path \"/dev/null\") (vnode-type CHARACTER-DEVICE)))\n\
         (allow file-write* (subpath \"{temp}\"){writable})\n(deny network*)\n\
         (deny file-write* (literal \"{git}\") (subpath \"{git}\"))",
        temp = seatbelt_escape(temp),
    ))
}

/// Escape a path for a Seatbelt string literal.
#[cfg(target_os = "macos")]
fn seatbelt_escape(path: &Path) -> String {
    path.to_string_lossy()
        .replace('\\', "\\\\")
        .replace('"', "\\\"")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn modes_round_trip_through_their_wire_names() {
        for mode in [
            ContainmentMode::ReadOnly,
            ContainmentMode::WorkspaceWrite,
            ContainmentMode::FullAccess,
        ] {
            assert_eq!(ContainmentMode::parse(mode.as_str()), Some(mode));
        }
        // A typo must not silently resolve to a mode.
        assert_eq!(ContainmentMode::parse("workspace_write"), None);
        assert_eq!(ContainmentMode::parse(""), None);
    }

    #[test]
    fn modes_order_from_contained_to_open() {
        assert!(ContainmentMode::ReadOnly < ContainmentMode::WorkspaceWrite);
        assert!(ContainmentMode::WorkspaceWrite < ContainmentMode::FullAccess);
        assert_eq!(ContainmentMode::default(), ContainmentMode::WorkspaceWrite);
    }

    #[test]
    fn only_full_access_runs_with_the_network_on_a_supported_platform() {
        #[cfg(any(target_os = "macos", target_os = "linux"))]
        {
            assert_eq!(network_access(ContainmentMode::WorkspaceWrite), "disabled");
            assert_eq!(network_access(ContainmentMode::FullAccess), "enabled");
            assert!(danger_warning(ContainmentMode::WorkspaceWrite).is_none());
            assert!(danger_warning(ContainmentMode::FullAccess).is_some());
        }
    }

    #[cfg(not(target_os = "windows"))]
    #[test]
    fn the_environment_allowlist_drops_credentials_and_keeps_toolchains() {
        assert!(safe_environment_key("PATH"));
        assert!(safe_environment_key("CARGO_HOME"));
        assert!(safe_environment_key("LC_TIME"));
        for key in [
            "OPENAI_API_KEY",
            "ANTHROPIC_API_KEY",
            "AWS_SECRET_ACCESS_KEY",
            "SSH_AUTH_SOCK",
            "GITHUB_TOKEN",
        ] {
            assert!(!safe_environment_key(key), "{key} must not be inherited");
        }
    }

    #[cfg(not(target_os = "windows"))]
    #[test]
    fn a_rustup_toolchain_survives_the_rewritten_home() {
        // Only filled in when the variable is unset and the directory exists,
        // so this asserts the rule rather than the machine it runs on.
        for (key, path) in toolchain_homes() {
            assert!(std::env::var_os(key).is_none());
            assert!(path.is_dir(), "{key} must point at a real directory");
        }
    }

    #[cfg(not(target_os = "windows"))]
    #[test]
    fn the_private_temp_directory_is_owner_only() {
        use std::os::unix::fs::PermissionsExt;

        let temp = sandbox_temp_dir("everruns-test").expect("temp directory");
        let mode = std::fs::metadata(&temp)
            .expect("temp metadata")
            .permissions()
            .mode();
        assert_eq!(mode & 0o777, 0o700);
        assert!(temp.join("home").parent().is_some());
        let _ = std::fs::remove_dir_all(&temp);
    }

    #[test]
    fn full_access_is_the_only_mode_that_hands_back_a_bare_shell() {
        let temp = tempfile::tempdir().expect("workspace");
        let open = provider(SandboxOptions::new(ContainmentMode::FullAccess));
        assert!(open.command(temp.path(), "true").is_ok());
        assert_eq!(open.mode(), ContainmentMode::FullAccess);
    }

    #[cfg(target_os = "linux")]
    #[test]
    fn a_re_exec_launcher_runs_the_current_binary_with_its_routing_arguments() {
        let (program, leading) = resolve_launcher(&SandboxLauncher::ReexecSelf(vec![
            "__sandbox-exec".to_string(),
        ]))
        .expect("launcher resolves");
        assert_eq!(program, std::env::current_exe().expect("current exe"));
        assert_eq!(leading, vec!["__sandbox-exec".to_string()]);
    }

    #[cfg(target_os = "linux")]
    #[test]
    fn extra_writable_roots_apply_only_where_writes_are_allowed() {
        let directory = tempfile::tempdir().expect("root");
        let cache = directory.path().join("cache");
        let options =
            SandboxOptions::new(ContainmentMode::WorkspaceWrite).writable_root(cache.clone());
        let roots = prepared_writable_roots(&options).expect("roots");
        assert_eq!(roots.len(), 1);
        assert!(cache.is_dir(), "a configured root is created, not assumed");

        let read_only = SandboxOptions::new(ContainmentMode::ReadOnly).writable_root(cache);
        assert!(
            prepared_writable_roots(&read_only)
                .expect("roots")
                .is_empty()
        );
    }
}
