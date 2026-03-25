//! Integration tests for CLI chat command.
//!
//! Tests the chat subcommand's argument handling and basic invocation.
//! Full end-to-end tests with a running server are in the server crate's
//! workflow tests; these verify the CLI binary's interface.

#[test]
fn test_cli_chat_help() {
    let output = std::process::Command::new("cargo")
        .args(["run", "-p", "everruns-cli", "--", "chat", "--help"])
        .output()
        .expect("Failed to run cargo");
    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(stdout.contains("--session"), "Should have --session flag");
    assert!(stdout.contains("--timeout"), "Should have --timeout flag");
    assert!(
        stdout.contains("--no-stream"),
        "Should have --no-stream flag"
    );
}

#[test]
fn test_cli_chat_requires_session_and_message() {
    // Chat without --session should fail
    let output = std::process::Command::new("cargo")
        .args(["run", "-p", "everruns-cli", "--", "chat"])
        .output()
        .expect("Failed to run cargo");
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(
        !output.status.success(),
        "Should fail without required args"
    );
    assert!(
        stderr.contains("--session") || stderr.contains("required"),
        "Should mention missing --session"
    );
}
