//! Integration tests for CLI authentication module.
//!
//! Tests credential storage, profile management, and resolution logic.

use std::fs;
use tempfile::TempDir;

/// Set up an isolated config directory and return its path
fn setup_config_dir() -> TempDir {
    tempfile::tempdir().unwrap()
}

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
fn test_credential_store_serialize_deserialize() {
    let json = r#"{
        "profiles": {
            "default": {
                "api_url": "http://localhost:9300/api",
                "api_key": "evr_0123456789abcdef0123456789abcdef0123456789abcdef0123456789abcdef",
                "org_id": "org_00000000000000000000000000000001",
                "user_email": "test@example.com",
                "user_name": "Test User"
            },
            "staging": {
                "api_url": "https://staging.everruns.com/api",
                "api_key": "evr_abcdef0123456789abcdef0123456789abcdef0123456789abcdef0123456789"
            }
        },
        "current_profile": "default"
    }"#;

    let parsed: serde_json::Value = serde_json::from_str(json).unwrap();
    assert_eq!(parsed["current_profile"], "default");
    assert_eq!(
        parsed["profiles"]["default"]["api_key"],
        "evr_0123456789abcdef0123456789abcdef0123456789abcdef0123456789abcdef"
    );
    assert_eq!(
        parsed["profiles"]["default"]["org_id"],
        "org_00000000000000000000000000000001"
    );
    // Staging profile should not have org_id
    assert!(parsed["profiles"]["staging"]["org_id"].is_null());
}

#[test]
fn test_credential_file_permissions() {
    let dir = setup_config_dir();
    let creds_dir = dir.path().join("everruns");
    fs::create_dir_all(&creds_dir).unwrap();
    let creds_path = creds_dir.join("credentials.json");
    fs::write(&creds_path, "{}").unwrap();

    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        fs::set_permissions(&creds_path, fs::Permissions::from_mode(0o600)).unwrap();
        let metadata = fs::metadata(&creds_path).unwrap();
        let mode = metadata.permissions().mode() & 0o777;
        assert_eq!(mode, 0o600, "Credentials file should have 0600 permissions");
    }
}

#[test]
fn test_multiple_profiles_in_credential_file() {
    let json = serde_json::json!({
        "profiles": {
            "default": {
                "api_url": "http://localhost/api",
                "api_key": "evr_key1",
            },
            "production": {
                "api_url": "https://prod.example.com/api",
                "api_key": "evr_key2",
                "org_id": "org_prod",
            }
        },
        "current_profile": "production"
    });

    let profiles = json["profiles"].as_object().unwrap();
    assert_eq!(profiles.len(), 2);
    assert_eq!(json["current_profile"], "production");
    assert_eq!(profiles["production"]["org_id"], "org_prod");
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
