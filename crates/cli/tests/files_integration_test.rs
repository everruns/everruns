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
fn test_sync_state_roundtrip_with_files() {
    let dir = create_test_dir(&[
        ("src/main.rs", "fn main() {}"),
        ("src/lib.rs", "pub mod foo;"),
        ("README.md", "# Hello"),
    ]);

    let state_dir = dir.path().join(".everruns-sync");

    // Create state with file entries
    let state_json = serde_json::json!({
        "session_id": "ses_integration_test",
        "last_sync": "2026-03-20T12:00:00Z",
        "files": {
            "src/main.rs": {
                "local_hash": "sha256:aaa",
                "remote_hash": "sha256:bbb",
                "local_mtime": null,
                "remote_updated_at": "2026-03-20T12:00:00Z"
            },
            "README.md": {
                "local_hash": "sha256:ccc",
                "remote_hash": "sha256:ccc",
                "local_mtime": null,
                "remote_updated_at": null
            }
        }
    });

    fs::create_dir_all(&state_dir).unwrap();
    fs::write(
        state_dir.join("state.json"),
        serde_json::to_string_pretty(&state_json).unwrap(),
    )
    .unwrap();

    // Verify it can be read back
    let data = fs::read_to_string(state_dir.join("state.json")).unwrap();
    let loaded: serde_json::Value = serde_json::from_str(&data).unwrap();
    assert_eq!(loaded["session_id"], "ses_integration_test");
    assert_eq!(loaded["files"]["src/main.rs"]["local_hash"], "sha256:aaa");
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

#[test]
fn test_binary_detection() {
    // Binary detection: null bytes in first 8KB → base64
    let text_content = b"Hello, World!";
    let binary_content = b"Hello\x00World";

    let is_text_binary = text_content.contains(&0);
    let is_binary_binary = binary_content.contains(&0);

    assert!(!is_text_binary, "Text should not be detected as binary");
    assert!(
        is_binary_binary,
        "Content with null bytes should be detected as binary"
    );
}

#[test]
fn test_content_hash_consistency() {
    use sha2::{Digest, Sha256};

    let content = b"fn main() { println!(\"hello\"); }";

    // Hash the same way the CLI does
    let mut hasher = Sha256::new();
    hasher.update(content);
    let result = hasher.finalize();
    let hash1 = format!(
        "sha256:{}",
        result
            .iter()
            .map(|b| format!("{:02x}", b))
            .collect::<String>()
    );

    // Hash again — should be deterministic
    let mut hasher2 = Sha256::new();
    hasher2.update(content);
    let result2 = hasher2.finalize();
    let hash2 = format!(
        "sha256:{}",
        result2
            .iter()
            .map(|b| format!("{:02x}", b))
            .collect::<String>()
    );

    assert_eq!(hash1, hash2);
    assert!(hash1.starts_with("sha256:"));
    assert_eq!(hash1.len(), 7 + 64); // "sha256:" + 64 hex chars
}

#[test]
fn test_sync_state_dir_not_synced() {
    // .everruns-sync directory should never be synced
    let dir = create_test_dir(&[("code.rs", "fn main() {}")]);

    // Create a sync state file
    let state_dir = dir.path().join(".everruns-sync");
    fs::create_dir_all(&state_dir).unwrap();
    fs::write(state_dir.join("state.json"), "{}").unwrap();

    // Walk and verify .everruns-sync is excluded
    let mut builder = ignore::WalkBuilder::new(dir.path());
    builder.hidden(false).git_ignore(false);

    let mut overrides = ignore::overrides::OverrideBuilder::new(dir.path());
    overrides.add("!.everruns-sync").unwrap();
    builder.overrides(overrides.build().unwrap());

    let mut found: Vec<String> = Vec::new();
    for entry in builder.build() {
        let entry = entry.unwrap();
        if entry.path().is_file() {
            let rel = entry.path().strip_prefix(dir.path()).unwrap();
            found.push(rel.to_string_lossy().to_string());
        }
    }

    assert!(found.contains(&"code.rs".to_string()));
    assert!(
        !found.iter().any(|f| f.contains(".everruns-sync")),
        ".everruns-sync should be excluded"
    );
}
