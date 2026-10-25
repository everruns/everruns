#![cfg_attr(test, allow(clippy::unwrap_used, clippy::expect_used))]
//! Integration test: verify Cursor plugin and connection provider registration.

use everruns_contracts::runtime::tool_narration::ToolNarrationPhase;
use everruns_contracts::tool_types::ToolCall;
use serde_json::json;

#[test]
fn cursor_narration_names_agent_work() {
    use everruns_contracts::runtime::capabilities::Capability;
    let cap = everruns_integrations::cursor::CursorCapability;
    let call = ToolCall {
        id: "call_1".into(),
        name: "cursor_launch_agent".into(),
        arguments: json!({
            "name": "Fix checkout bug",
            "repository": "https://github.com/acme/app",
            "prompt": "Fix it"
        }),
    };

    let narration = cap
        .narrate(
            None,
            &call,
            ToolNarrationPhase::Started,
            None,
            everruns_contracts::runtime::tool_narration::ToolNarrationContext::default(),
        )
        .expect("narration");
    assert_eq!(narration, "Starting Cursor agent: Fix checkout bug");
}
