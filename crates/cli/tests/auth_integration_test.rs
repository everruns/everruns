#![cfg_attr(test, allow(clippy::unwrap_used, clippy::expect_used))]
//! Integration tests for CLI authentication module.
//!
//! Tests credential storage, profile management, and resolution logic.

#[test]
fn test_cli_login_help() {
    let output = std::process::Command::new("cargo")
        .args(["run", "-p", "everruns-cli", "--", "login", "--help"])
        .output()
        .expect("Failed to run cargo");
    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(
        stdout.contains("Interactive login"),
        "Help should describe login command"
    );
    assert!(stdout.contains("--token"), "Help should show --token flag");
}

#[test]
fn test_cli_logout_help() {
    let output = std::process::Command::new("cargo")
        .args(["run", "-p", "everruns-cli", "--", "logout", "--help"])
        .output()
        .expect("Failed to run cargo");
    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(
        stdout.contains("credentials") || stdout.contains("Remove"),
        "Help should describe logout"
    );
}

#[test]
fn test_cli_status_help() {
    let output = std::process::Command::new("cargo")
        .args(["run", "-p", "everruns-cli", "--", "status", "--help"])
        .output()
        .expect("Failed to run cargo");
    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(
        stdout.contains("user") || stdout.contains("org") || stdout.contains("Show"),
        "Help should describe status"
    );
}

#[test]
fn test_cli_orgs_help() {
    let output = std::process::Command::new("cargo")
        .args(["run", "-p", "everruns-cli", "--", "orgs", "--help"])
        .output()
        .expect("Failed to run cargo");
    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(
        stdout.contains("select") || stdout.contains("organization"),
        "Help should show orgs subcommands"
    );
}

#[test]
fn test_cli_profile_flag() {
    let output = std::process::Command::new("cargo")
        .args([
            "run",
            "-p",
            "everruns-cli",
            "--",
            "--profile",
            "staging",
            "status",
        ])
        .output()
        .expect("Failed to run cargo");
    // Should fail gracefully (not logged in), but not crash
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(
        stderr.contains("Not logged in") || !output.status.success(),
        "Should handle missing profile gracefully"
    );
}

#[test]
fn test_cli_status_without_login_exits_nonzero() {
    let output = std::process::Command::new("cargo")
        .args([
            "run",
            "-p",
            "everruns-cli",
            "--",
            "--profile",
            "nonexistent_test_profile_12345",
            "status",
        ])
        .env_remove("EVERRUNS_API_KEY")
        .output()
        .expect("Failed to run cargo");
    // Should exit with non-zero since no credentials exist for this profile
    assert!(
        !output.status.success(),
        "Status should fail when not logged in"
    );
}

#[test]
fn package_validation_and_local_diff_print_results_without_credentials() {
    let path = format!(
        "{}/../../examples/agents/triage",
        env!("CARGO_MANIFEST_DIR")
    );
    for args in [
        vec!["agents", "validate", path.as_str()],
        vec!["agents", "diff", path.as_str(), "--against", path.as_str()],
    ] {
        let result = std::process::Command::new(env!("CARGO_BIN_EXE_everruns"))
            .args(args)
            .env_remove("EVERRUNS_API_KEY")
            .env_remove("EVERRUNS_API_URL")
            .output()
            .unwrap();
        assert!(
            result.status.success(),
            "{}",
            String::from_utf8_lossy(&result.stderr)
        );
        let value: serde_json::Value = serde_json::from_slice(&result.stdout).unwrap();
        assert!(
            value["valid"] == true || value["changed"] == false,
            "{value}"
        );
    }
}
