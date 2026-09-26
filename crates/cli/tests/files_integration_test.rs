#![cfg_attr(test, allow(clippy::unwrap_used, clippy::expect_used))]
//! Integration tests for CLI file sync operations.
//!
//! Tests the full sync workflow with filesystem operations:
//! scan, state persistence, and reconciliation logic.

// Since the CLI is a binary crate, CLI-invocation behavior is tested via
// subprocess here. Logic that lives in library-like modules (e.g. scan_local)
// is unit-tested in place under crates/cli/src/commands/files/.

#[test]
fn test_cli_binary_exists() {
    // Verify the binary can be found and shows help
    let output = std::process::Command::new("cargo")
        .args(["run", "-p", "everruns-cli", "--", "--help"])
        .output()
        .expect("Failed to run cargo");
    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(
        stdout.contains("Everruns CLI") || stdout.contains("everruns"),
        "Help output should contain CLI name"
    );
}

#[test]
fn test_cli_files_help() {
    let output = std::process::Command::new("cargo")
        .args(["run", "-p", "everruns-cli", "--", "files", "--help"])
        .output()
        .expect("Failed to run cargo");
    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(stdout.contains("sync"), "Should list sync subcommand");
    assert!(stdout.contains("push"), "Should list push subcommand");
    assert!(stdout.contains("pull"), "Should list pull subcommand");
    assert!(stdout.contains("ls"), "Should list ls subcommand");
}

#[test]
fn test_cli_files_sync_help() {
    let output = std::process::Command::new("cargo")
        .args(["run", "-p", "everruns-cli", "--", "files", "sync", "--help"])
        .output()
        .expect("Failed to run cargo");
    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(stdout.contains("--session"), "Should have --session flag");
    assert!(stdout.contains("--interval"), "Should have --interval flag");
    assert!(stdout.contains("--conflict"), "Should have --conflict flag");
    assert!(stdout.contains("--dry-run"), "Should have --dry-run flag");
    assert!(stdout.contains("--delete"), "Should have --delete flag");
    assert!(stdout.contains("--verbose"), "Should have --verbose flag");
}

#[test]
fn test_cli_files_push_requires_session() {
    let output = std::process::Command::new("cargo")
        .args(["run", "-p", "everruns-cli", "--", "files", "push"])
        .output()
        .expect("Failed to run cargo");
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(
        stderr.contains("--session") || stderr.contains("required"),
        "Should fail with missing session"
    );
    assert!(!output.status.success());
}

#[test]
fn test_cli_files_pull_requires_session() {
    let output = std::process::Command::new("cargo")
        .args(["run", "-p", "everruns-cli", "--", "files", "pull"])
        .output()
        .expect("Failed to run cargo");
    assert!(!output.status.success());
}

#[test]
fn test_cli_files_ls_requires_session() {
    let output = std::process::Command::new("cargo")
        .args(["run", "-p", "everruns-cli", "--", "files", "ls"])
        .output()
        .expect("Failed to run cargo");
    assert!(!output.status.success());
}

// test_gitignore_respected moved to crates/cli/src/commands/files/sync_engine.rs
// as test_scan_local_gitignore_respected, which calls the real scan_local
// instead of reimplementing its WalkBuilder setup.
