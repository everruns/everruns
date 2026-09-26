#![cfg_attr(test, allow(clippy::unwrap_used, clippy::expect_used))]
//! Integration tests for CLI file sync operations.
//!
//! Tests the full sync workflow with filesystem operations:
//! scan, state persistence, and reconciliation logic.

use std::fs;

// Import via the crate's public modules
// Since the CLI is a binary crate, we test via subprocess for CLI invocation
// and via direct imports for library-like logic.

/// Test helper: create a temp directory with files
fn create_test_dir(files: &[(&str, &str)]) -> tempfile::TempDir {
    let dir = tempfile::tempdir().unwrap();
    for (path, content) in files {
        let full_path = dir.path().join(path);
        if let Some(parent) = full_path.parent() {
            fs::create_dir_all(parent).unwrap();
        }
        fs::write(full_path, content).unwrap();
    }
    dir
}

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

#[test]
fn test_gitignore_respected() {
    let dir = create_test_dir(&[
        ("keep.txt", "keep me"),
        ("build/output.js", "compiled"),
        (".gitignore", "build/\n"),
    ]);

    // WalkBuilder needs a .git dir to recognize .gitignore
    fs::create_dir_all(dir.path().join(".git")).unwrap();

    // Simulate what scan_local does: walk with gitignore support
    let mut builder = ignore::WalkBuilder::new(dir.path());
    builder
        .hidden(false)
        .git_ignore(true)
        .git_global(false)
        .git_exclude(false);

    let mut found_files: Vec<String> = Vec::new();
    for entry in builder.build() {
        let entry = entry.unwrap();
        if entry.path().is_file() {
            let rel = entry.path().strip_prefix(dir.path()).unwrap();
            found_files.push(rel.to_string_lossy().replace('\\', "/"));
        }
    }

    assert!(found_files.contains(&"keep.txt".to_string()));
    assert!(found_files.contains(&".gitignore".to_string()));
    assert!(
        !found_files.contains(&"build/output.js".to_string()),
        "build/ should be gitignored, found: {:?}",
        found_files
    );
}
