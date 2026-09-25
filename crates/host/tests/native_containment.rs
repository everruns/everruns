#![cfg(target_os = "linux")]
#![allow(clippy::unwrap_used, clippy::expect_used)]
//! The Linux boundary, exercised through the real helper binary.
//!
//! These are the tests a unit test cannot stand in for: the worker restricts a
//! process it then replaces, so nothing short of spawning it proves the rules
//! hold. A kernel without full Landlock ABI v3 cannot answer the question, so
//! the suite asserts the fail-closed error and returns rather than passing
//! vacuously.

use std::path::PathBuf;
use std::sync::Arc;

use everruns_host::containment::{
    ContainmentMode, SandboxLauncher, SandboxOptions, SandboxProvider, configure_stdio, provider,
};
use tokio::process::Command;

/// A scratch directory outside `/tmp`.
///
/// `/tmp` is deliberately writable at [`ContainmentMode::WorkspaceWrite`] for
/// development-tool compatibility, so a bare `tempfile::tempdir()` is *inside*
/// the boundary and proves nothing about escaping it. Cargo's per-test
/// directory lives under `target/`, which is not.
fn scratch() -> tempfile::TempDir {
    tempfile::tempdir_in(env!("CARGO_TARGET_TMPDIR")).expect("scratch directory")
}

/// The helper this package ships, as cargo built it for this test run.
fn helper() -> PathBuf {
    let path = PathBuf::from(env!("CARGO_BIN_EXE_everruns-sandbox-exec"));
    assert!(
        path.is_file(),
        "cargo builds the helper for integration tests"
    );
    path
}

fn contained(mode: ContainmentMode) -> Arc<dyn SandboxProvider> {
    provider(SandboxOptions::new(mode).launcher(SandboxLauncher::Helper(helper())))
}

async fn run(mut command: Command) -> (i32, String) {
    configure_stdio(&mut command);
    let output = command.output().await.expect("the helper spawns");
    let mut text = String::from_utf8_lossy(&output.stdout).into_owned();
    text.push_str(&String::from_utf8_lossy(&output.stderr));
    (output.status.code().unwrap_or(-1), text)
}

/// Whether this kernel can enforce the policy at all. On one that cannot, the
/// provider fails closed, which is itself the contract worth asserting.
async fn kernel_enforces_landlock(workspace: &std::path::Path) -> bool {
    let command = contained(ContainmentMode::WorkspaceWrite)
        .command(workspace, workspace, "true")
        .expect("command builds");
    let (code, output) = run(command).await;
    if code == 0 {
        return true;
    }
    assert!(
        output.contains("Landlock") || output.contains("native containment unavailable"),
        "an unenforceable kernel must say so, not run uncontained: {output}"
    );
    eprintln!("skipping: this kernel does not fully enforce Landlock ABI v3");
    false
}

#[tokio::test]
async fn a_contained_command_writes_inside_the_workspace_and_nowhere_else() {
    let workspace = scratch();
    let outside = scratch();
    if !kernel_enforces_landlock(workspace.path()).await {
        return;
    }

    let inside = contained(ContainmentMode::WorkspaceWrite)
        .command(
            workspace.path(),
            workspace.path(),
            "echo contained > witness.txt",
        )
        .expect("command builds");
    let (code, output) = run(inside).await;
    assert_eq!(code, 0, "writing into the workspace: {output}");
    assert!(workspace.path().join("witness.txt").is_file());

    let escape = contained(ContainmentMode::WorkspaceWrite)
        .command(
            workspace.path(),
            workspace.path(),
            &format!("echo escaped > {}/witness.txt", outside.path().display()),
        )
        .expect("command builds");
    let (code, _) = run(escape).await;
    assert_ne!(code, 0, "a write outside the workspace must fail");
    assert!(!outside.path().join("witness.txt").exists());
}

#[tokio::test]
async fn a_symlinked_working_directory_cannot_escape_the_workspace() {
    use std::os::unix::fs::symlink;

    let workspace = scratch();
    let outside = scratch();
    if !kernel_enforces_landlock(workspace.path()).await {
        return;
    }

    // This models two tool calls: first create an allowed workspace symlink,
    // then request it as the next command's working directory.
    symlink(outside.path(), workspace.path().join("escape")).expect("create escape symlink");
    let command = contained(ContainmentMode::WorkspaceWrite)
        .command(
            workspace.path(),
            &workspace.path().join("escape"),
            "echo escaped > witness.txt",
        )
        .expect("command builds");
    let (code, output) = run(command).await;

    assert_ne!(code, 0, "symlinked cwd must be rejected: {output}");
    assert!(!outside.path().join("witness.txt").exists());
}

#[tokio::test]
async fn read_only_containment_denies_the_workspace_too() {
    let workspace = scratch();
    if !kernel_enforces_landlock(workspace.path()).await {
        return;
    }

    let command = contained(ContainmentMode::ReadOnly)
        .command(
            workspace.path(),
            workspace.path(),
            "echo denied > witness.txt",
        )
        .expect("command builds");
    let (code, _) = run(command).await;
    assert_ne!(code, 0, "read-only means the workspace is read-only");
    assert!(!workspace.path().join("witness.txt").exists());

    let read = contained(ContainmentMode::ReadOnly)
        .command(workspace.path(), workspace.path(), "ls /")
        .expect("command builds");
    assert_eq!(run(read).await.0, 0, "host reads stay available");
}

#[tokio::test]
async fn a_configured_writable_root_is_writable_and_its_parent_is_not() {
    let workspace = scratch();
    let shared = scratch();
    let cache = shared.path().join("cache");
    if !kernel_enforces_landlock(workspace.path()).await {
        return;
    }

    let sandbox = provider(
        SandboxOptions::new(ContainmentMode::WorkspaceWrite)
            .launcher(SandboxLauncher::Helper(helper()))
            .writable_root(&cache),
    );

    let command = sandbox
        .command(
            workspace.path(),
            workspace.path(),
            &format!("echo cached > {}/witness.txt", cache.display()),
        )
        .expect("command builds");
    assert_eq!(run(command).await.0, 0, "the configured root is writable");

    let parent = sandbox
        .command(
            workspace.path(),
            workspace.path(),
            &format!("echo nope > {}/witness.txt", shared.path().display()),
        )
        .expect("command builds");
    assert_ne!(
        run(parent).await.0,
        0,
        "granting a root must not grant its parent"
    );
}

#[tokio::test]
async fn outbound_network_sockets_are_denied() {
    let workspace = scratch();
    if !kernel_enforces_landlock(workspace.path()).await {
        return;
    }

    // `/dev/tcp` is bash's own socket path, so this tests the seccomp filter
    // without depending on curl or nc being installed.
    let command = contained(ContainmentMode::WorkspaceWrite)
        .command(
            workspace.path(),
            workspace.path(),
            "exec 3<>/dev/tcp/1.1.1.1/80",
        )
        .expect("command builds");
    let (code, _) = run(command).await;
    assert_ne!(code, 0, "an internet socket must not be creatable");
}

#[tokio::test]
async fn credentials_in_the_parent_environment_do_not_reach_the_command() {
    let workspace = scratch();
    if !kernel_enforces_landlock(workspace.path()).await {
        return;
    }

    // SAFETY: the value is a fixture, not a credential, and the test sets it
    // before building the command that reads it.
    unsafe { std::env::set_var("EVERRUNS_TEST_SECRET", "leaked") };
    let command = contained(ContainmentMode::WorkspaceWrite)
        .command(
            workspace.path(),
            workspace.path(),
            "echo \"[${EVERRUNS_TEST_SECRET:-absent}]\"",
        )
        .expect("command builds");
    let (_, output) = run(command).await;
    unsafe { std::env::remove_var("EVERRUNS_TEST_SECRET") };

    assert!(output.contains("[absent]"), "unexpected output: {output}");
}

#[tokio::test]
async fn shared_tmp_stays_writable_by_design() {
    let workspace = scratch();
    if !kernel_enforces_landlock(workspace.path()).await {
        return;
    }

    // A documented limitation, pinned so it cannot change by accident: build
    // tools expect to write to /tmp, and the cost is that a contained command
    // can leave files where other host processes see them.
    let command = contained(ContainmentMode::WorkspaceWrite)
        .command(
            workspace.path(),
            workspace.path(),
            "touch /tmp/everruns-containment-witness",
        )
        .expect("command builds");
    assert_eq!(run(command).await.0, 0);
    let _ = std::fs::remove_file("/tmp/everruns-containment-witness");
}

#[tokio::test]
async fn dev_null_stays_writable() {
    let workspace = scratch();
    if !kernel_enforces_landlock(workspace.path()).await {
        return;
    }

    // Login shells redirect to /dev/null all over their profile scripts, so a
    // policy without this rule buries every command in permission errors before
    // it starts.
    let command = contained(ContainmentMode::ReadOnly)
        .command(workspace.path(), workspace.path(), "echo quiet > /dev/null")
        .expect("command builds");
    let (code, output) = run(command).await;
    assert_eq!(code, 0, "unexpected output: {output}");
}
