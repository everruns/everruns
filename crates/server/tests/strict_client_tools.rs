//! Strict-mode coverage for the `EVERRUNS_REJECT_NON_CLIENT_SIDE_TOOLS` knob.
//!
//! These tests live in their own integration-test binary so the env var is
//! set per-process and cannot race with the lenient unit tests in the
//! library binary. See `crates/server/src/api/sessions.rs` for the
//! deserializer implementation and `specs/client-side-tools.md` for the
//! deprecation timeline.

use everruns_server::api::sessions::CreateSessionRequest;
use everruns_server::domains::agents::types::{CreateAgentRequest, UpdateAgentRequest};

const TEST_HARNESS_ID: &str = "harness_550e8400e29b41d4a716446655440000";

fn enable_strict_mode() {
    // SAFETY: integration-test binary owns its own process; setting env
    // applies for every test in this binary.
    unsafe { std::env::set_var("EVERRUNS_REJECT_NON_CLIENT_SIDE_TOOLS", "1") };
}

#[test]
fn create_session_request_strict_mode_rejects_builtin_tools() {
    enable_strict_mode();
    let json = format!(
        r#"{{
            "harness_id": "{}",
            "tools": [
                {{
                    "type": "builtin",
                    "name": "read_file",
                    "description": "Read file",
                    "parameters": {{"type": "object"}}
                }}
            ]
        }}"#,
        TEST_HARNESS_ID
    );
    let err = serde_json::from_str::<CreateSessionRequest>(&json).unwrap_err();
    assert!(
        err.to_string()
            .contains("tools must contain only client_side definitions"),
        "expected hard-reject error, got: {err}"
    );
}

#[test]
fn create_agent_request_strict_mode_rejects_builtin_tools() {
    enable_strict_mode();
    let json = r#"{
        "name": "test-agent",
        "system_prompt": "You are helpful",
        "tools": [
            {
                "type": "builtin",
                "name": "read_file",
                "description": "Read file",
                "parameters": {"type": "object"}
            }
        ]
    }"#;
    let err = serde_json::from_str::<CreateAgentRequest>(json).unwrap_err();
    assert!(
        err.to_string()
            .contains("tools must contain only client_side definitions"),
        "expected hard-reject error, got: {err}"
    );
}

#[test]
fn update_agent_request_strict_mode_rejects_builtin_tools() {
    enable_strict_mode();
    let json = r#"{
        "tools": [
            {
                "type": "builtin",
                "name": "web_fetch",
                "description": "Fetch",
                "parameters": {"type": "object"}
            }
        ]
    }"#;
    let err = serde_json::from_str::<UpdateAgentRequest>(json).unwrap_err();
    assert!(
        err.to_string()
            .contains("tools must contain only client_side definitions"),
        "expected hard-reject error, got: {err}"
    );
}
