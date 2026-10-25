#![cfg_attr(test, allow(clippy::unwrap_used, clippy::expect_used))]
//! Integration test: verify Cursor plugin and connection provider registration.

use everruns_contracts::runtime::capabilities::CapabilityRegistry;
use everruns_contracts::runtime::deployment::DeploymentGrade;
use everruns_contracts::runtime::tool_narration::ToolNarrationPhase;
use everruns_contracts::tool_types::ToolCall;
use serde_json::json;

use everruns_integrations::cursor::CAPABILITY_PLUGINS;

fn registry_for_grade(grade: DeploymentGrade) -> CapabilityRegistry {
    let mut registry = CapabilityRegistry::new();
    registry.register_plugins(CAPABILITY_PLUGINS.iter(), |plugin| {
        plugin.feature_flag.is_none_or(|flag| {
            everruns_contracts::runtime::feature_flag_available(flag, std::iter::empty(), grade)
        })
    });
    registry
}

#[test]
fn cursor_plugin_is_registered_in_prod_and_dev() {
    let dev = registry_for_grade(DeploymentGrade::Dev);
    let prod = registry_for_grade(DeploymentGrade::Prod);
    assert!(dev.has("cursor"), "Cursor should be in dev registry");
    assert!(prod.has("cursor"), "Cursor should be in prod registry");
}

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
