//! The helper process that applies Linux kernel restrictions, then becomes the shell.
//!
//! Landlock and seccomp restrict *the calling process*, and they have to be
//! installed before the shell is exec'd. Doing that from `pre_exec` would mean
//! allocating and taking locks after forking a multi-threaded Tokio runtime,
//! where another thread may have held the allocator lock at fork time. So the
//! parent spawns a helper instead: the helper restricts itself in a
//! single-threaded process, then `exec`s bash in place. The restrictions are
//! inherited by every descendant.
//!
//! The helper is [`everruns-sandbox-exec`], shipped by this package. An
//! embedder that would rather not ship a second file points
//! [`SandboxLauncher::ReexecSelf`](super::SandboxLauncher::ReexecSelf) at its
//! own binary and routes the leading argument here.
//!
//! ```no_run
//! // In an embedder's `main`, before anything else runs:
//! let mut arguments = std::env::args().skip(1);
//! if arguments.next().as_deref() == Some("__sandbox-exec") {
//!     everruns_host::containment::worker::run_from_args(arguments)?;
//! }
//! # Ok::<(), anyhow::Error>(())
//! ```
//!
//! [`everruns-sandbox-exec`]: https://docs.rs/everruns-host

use std::path::PathBuf;

use anyhow::{Context, Result};

use super::ContainmentMode;

/// The request a launcher encodes on the helper's command line.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct WorkerRequest {
    /// Trusted writable root established by the embedder.
    pub workspace: PathBuf,
    /// Working directory for the shell. Must be a non-symlink descendant of the workspace.
    pub cwd: PathBuf,
    /// The private temp directory, writable at every mode.
    pub temp: PathBuf,
    /// The containment to apply. [`ContainmentMode::FullAccess`] is rejected:
    /// an uncontained command never reaches the worker.
    pub mode: ContainmentMode,
    /// Extra writable roots, already created and canonicalized by the launcher.
    pub writable_roots: Vec<PathBuf>,
    /// The script to run.
    pub script: String,
}

impl WorkerRequest {
    /// Parse the argument list a launcher produced.
    ///
    /// Every field is required and unknown flags are an error, so a launcher
    /// and worker that drift apart fail loudly instead of running with a
    /// silently narrower or wider policy.
    pub fn parse<I, S>(arguments: I) -> Result<Self>
    where
        I: IntoIterator<Item = S>,
        S: Into<std::ffi::OsString>,
    {
        let mut workspace = None;
        let mut cwd = None;
        let mut temp = None;
        let mut mode = None;
        let mut writable_roots = Vec::new();
        let mut script = None;

        let mut arguments = arguments.into_iter().map(Into::into);
        while let Some(flag) = arguments.next() {
            let flag = flag.to_string_lossy().into_owned();
            let mut value = || {
                arguments
                    .next()
                    .with_context(|| format!("{flag} expects a value"))
            };
            match flag.as_str() {
                "--workspace" => workspace = Some(PathBuf::from(value()?)),
                "--cwd" => cwd = Some(PathBuf::from(value()?)),
                "--temp" => temp = Some(PathBuf::from(value()?)),
                "--writable" => writable_roots.push(PathBuf::from(value()?)),
                "--mode" => {
                    let raw = value()?.to_string_lossy().into_owned();
                    mode = Some(
                        ContainmentMode::parse(&raw)
                            .with_context(|| format!("unknown containment mode: {raw}"))?,
                    );
                }
                "--script" => script = Some(value()?.to_string_lossy().into_owned()),
                other => anyhow::bail!("unknown sandbox worker argument: {other}"),
            }
        }

        let request = Self {
            workspace: workspace.context("--workspace is required")?,
            cwd: cwd.context("--cwd is required")?,
            temp: temp.context("--temp is required")?,
            mode: mode.context("--mode is required")?,
            writable_roots,
            script: script.context("--script is required")?,
        };
        if request.mode.is_full_access() {
            anyhow::bail!("the sandbox worker cannot run danger-full-access");
        }
        Ok(request)
    }
}

/// Parse `arguments` and [`run`] the request. Never returns on success.
pub fn run_from_args<I, S>(arguments: I) -> Result<std::convert::Infallible>
where
    I: IntoIterator<Item = S>,
    S: Into<std::ffi::OsString>,
{
    run(&WorkerRequest::parse(arguments)?)
}

/// Apply the request's kernel restrictions, then replace this process with bash.
///
/// Fails closed: a kernel that cannot fully enforce the policy is an error, not
/// a quiet downgrade to an uncontained shell.
#[cfg(target_os = "linux")]
pub fn run(request: &WorkerRequest) -> Result<std::convert::Infallible> {
    use landlock::{
        ABI, Access, AccessFs, PathBeneath, PathFd, Ruleset, RulesetAttr, RulesetCreatedAttr,
        RulesetStatus,
    };
    use seccompiler::{
        BpfProgram, SeccompAction, SeccompCmpArgLen, SeccompCmpOp, SeccompCondition, SeccompFilter,
        SeccompRule,
    };
    use std::convert::TryInto;
    use std::os::unix::process::CommandExt;

    let workspace = open_directory_no_symlinks(&request.workspace, None)?;
    let relative_cwd = request
        .cwd
        .strip_prefix(&request.workspace)
        .with_context(|| {
            format!(
                "working directory is outside workspace: {}",
                request.cwd.display()
            )
        })?;
    let cwd = open_directory_no_symlinks(relative_cwd, Some(&workspace))?;
    // Keep both descriptors alive until after Landlock is installed. This makes
    // validation and use the same filesystem objects, closing symlink-swap races.
    unsafe {
        if libc::fchdir(std::os::fd::AsRawFd::as_raw_fd(&cwd)) != 0 {
            return Err(std::io::Error::last_os_error()).context("enter sandbox working directory");
        }
    }

    // ABI V3 is the oldest policy that also mediates truncate(2); requiring full
    // enforcement avoids silently weakening the write boundary.
    let abi = ABI::V3;
    let mut ruleset = Ruleset::default()
        .handle_access(AccessFs::from_all(abi))?
        .create()?
        .add_rule(PathBeneath::new(
            PathFd::new("/")?,
            AccessFs::from_read(abi),
        ))?
        .add_rule(PathBeneath::new(
            PathFd::new(&request.temp)?,
            AccessFs::from_all(abi),
        ))?
        // The one writable device. Without it every `2>/dev/null` in a login
        // shell's profile scripts fails, which turns a contained shell into a
        // wall of permission errors before the command even starts. The macOS
        // profile has always allowed exactly this.
        .add_rule(PathBeneath::new(
            PathFd::new("/dev/null")?,
            AccessFs::WriteFile | AccessFs::ReadFile,
        ))?;
    if request.mode == ContainmentMode::WorkspaceWrite {
        // Landlock path rules are additive, so a writable workspace grant
        // cannot subtract a read-only `.git` the way the Seatbelt profile does.
        // Git metadata inside the workspace is therefore writable here; linked
        // worktree metadata outside it stays read-only.
        ruleset = ruleset
            .add_rule(PathBeneath::new(
                PathFd::new(format!(
                    "/proc/self/fd/{}",
                    std::os::fd::AsRawFd::as_raw_fd(&workspace)
                ))?,
                AccessFs::from_all(abi),
            ))?
            .add_rule(PathBeneath::new(
                PathFd::new("/tmp")?,
                AccessFs::from_all(abi),
            ))?;
        for root in &request.writable_roots {
            ruleset = ruleset.add_rule(PathBeneath::new(
                PathFd::new(root)?,
                AccessFs::from_all(abi),
            ))?;
        }
    }
    let status = ruleset.restrict_self()?;
    if status.ruleset != RulesetStatus::FullyEnforced || !status.no_new_privs {
        anyhow::bail!(
            "native containment unavailable: Landlock ABI v3 is not fully enforced by this Linux \
             kernel; refusing to run uncontained. Select `danger-full-access` only inside an \
             already isolated environment"
        );
    }

    // Internet and packet sockets are denied. Unix sockets stay available for
    // local toolchains; the sanitized environment already removed the
    // credential-bearing agent socket paths before this worker started.
    let socket_rules = [libc::AF_INET, libc::AF_INET6, libc::AF_PACKET]
        .into_iter()
        .map(|family| {
            SeccompRule::new(vec![SeccompCondition::new(
                0,
                SeccompCmpArgLen::Dword,
                SeccompCmpOp::Eq,
                family as u64,
            )?])
        })
        .collect::<std::result::Result<Vec<_>, _>>()?;
    let filter: BpfProgram = SeccompFilter::new(
        [(libc::SYS_socket, socket_rules)].into_iter().collect(),
        SeccompAction::Allow,
        SeccompAction::Errno(libc::EACCES as u32),
        std::env::consts::ARCH.try_into()?,
    )?
    .try_into()?;
    seccompiler::apply_filter(&filter)?;

    Err(std::process::Command::new("/bin/bash")
        .arg("-lc")
        .arg(&request.script)
        .exec()
        .into())
}

#[cfg(target_os = "linux")]
fn open_directory_no_symlinks(
    path: &std::path::Path,
    relative_to: Option<&std::fs::File>,
) -> Result<std::fs::File> {
    use std::ffi::CString;
    use std::os::fd::{AsRawFd, FromRawFd};
    use std::os::unix::ffi::OsStrExt;

    let mut current = relative_to.map(std::fs::File::try_clone).transpose()?;
    for component in path.components() {
        use std::path::Component;
        let segment = match component {
            Component::RootDir if current.is_none() => {
                let root = CString::new("/").expect("static path");
                let fd = unsafe {
                    libc::open(
                        root.as_ptr(),
                        libc::O_PATH | libc::O_DIRECTORY | libc::O_CLOEXEC,
                    )
                };
                if fd < 0 {
                    return Err(std::io::Error::last_os_error()).context("open filesystem root");
                }
                current = Some(unsafe { std::fs::File::from_raw_fd(fd) });
                continue;
            }
            Component::CurDir => continue,
            Component::Normal(segment) => segment,
            _ => anyhow::bail!("working directory contains an invalid path component"),
        };
        let segment = CString::new(segment.as_bytes()).context("path contains NUL")?;
        let parent = current.as_ref().map_or(libc::AT_FDCWD, AsRawFd::as_raw_fd);
        let fd = unsafe {
            libc::openat(
                parent,
                segment.as_ptr(),
                libc::O_PATH | libc::O_DIRECTORY | libc::O_NOFOLLOW | libc::O_CLOEXEC,
            )
        };
        if fd < 0 {
            return Err(std::io::Error::last_os_error()).with_context(|| {
                format!(
                    "open directory without following symlinks: {}",
                    path.display()
                )
            });
        }
        current = Some(unsafe { std::fs::File::from_raw_fd(fd) });
    }
    current.context("directory path is empty")
}

/// Landlock and seccomp are Linux primitives; every other platform contains
/// commands another way or not at all.
#[cfg(not(target_os = "linux"))]
pub fn run(_request: &WorkerRequest) -> Result<std::convert::Infallible> {
    anyhow::bail!("the sandbox worker runs only on Linux")
}

#[cfg(test)]
mod tests {
    use super::*;

    fn arguments() -> Vec<String> {
        [
            "--workspace",
            "/work",
            "--cwd",
            "/work",
            "--temp",
            "/tmp/everruns-sandbox-1",
            "--mode",
            "workspace-write",
            "--writable",
            "/cache",
            "--script",
            "cargo test",
        ]
        .iter()
        .map(|value| value.to_string())
        .collect()
    }

    #[test]
    fn a_launcher_argument_list_round_trips_into_a_request() {
        let request = WorkerRequest::parse(arguments()).expect("parses");
        assert_eq!(request.cwd, PathBuf::from("/work"));
        assert_eq!(request.workspace, PathBuf::from("/work"));
        assert_eq!(request.mode, ContainmentMode::WorkspaceWrite);
        assert_eq!(request.writable_roots, vec![PathBuf::from("/cache")]);
        assert_eq!(request.script, "cargo test");
    }

    #[test]
    fn a_script_that_looks_like_a_flag_stays_a_script() {
        let mut arguments = arguments();
        let last = arguments.len() - 1;
        arguments[last] = "--mode read-only".to_string();

        let request = WorkerRequest::parse(arguments).expect("parses");
        assert_eq!(request.script, "--mode read-only");
        assert_eq!(request.mode, ContainmentMode::WorkspaceWrite);
    }

    #[test]
    fn a_missing_field_is_an_error_rather_than_a_default() {
        for dropped in ["--workspace", "--cwd", "--temp", "--mode", "--script"] {
            let mut kept = Vec::new();
            let mut arguments = arguments().into_iter();
            while let Some(flag) = arguments.next() {
                let value = arguments.next().expect("paired");
                if flag != dropped {
                    kept.push(flag);
                    kept.push(value);
                }
            }
            assert!(
                WorkerRequest::parse(kept).is_err(),
                "{dropped} must be required"
            );
        }
    }

    #[test]
    fn the_worker_refuses_to_run_an_uncontained_command() {
        let mut arguments = arguments();
        let mode = arguments
            .iter()
            .position(|value| value == "--mode")
            .unwrap()
            + 1;
        arguments[mode] = "danger-full-access".to_string();
        let error = WorkerRequest::parse(arguments).expect_err("refused");
        assert!(error.to_string().contains("danger-full-access"));
    }

    #[test]
    fn an_unknown_flag_fails_instead_of_being_ignored() {
        let mut arguments = arguments();
        arguments.push("--allow-everything".to_string());
        assert!(WorkerRequest::parse(arguments).is_err());
    }

    #[cfg(target_os = "linux")]
    #[test]
    fn working_directory_resolution_rejects_a_symlink_escape() {
        use std::os::unix::fs::symlink;

        let workspace = tempfile::tempdir().expect("workspace");
        let outside = tempfile::tempdir().expect("outside");
        symlink(outside.path(), workspace.path().join("escape")).expect("create escape symlink");
        let root = open_directory_no_symlinks(workspace.path(), None).expect("open workspace");

        let error = open_directory_no_symlinks(std::path::Path::new("escape"), Some(&root))
            .expect_err("symlink is not followed");
        assert!(error.to_string().contains("without following symlinks"));
    }
}
