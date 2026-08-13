//! Integration test: verify Cursor plugin and connection provider registration.

use everruns_core::capabilities::{CapabilityRegistry, IntegrationPlugin};
use everruns_core::deployment::DeploymentGrade;
use everruns_core::tool_narration::ToolNarrationPhase;
use everruns_core::tool_types::ToolCall;
use everruns_platform::connector::ConnectorPlugin;
use serde_json::json;

// Force linker to include the integration crate's inventory submissions.
use everruns_integrations_cursor as _;

fn registry_for_grade(grade: DeploymentGrade) -> CapabilityRegistry {
    let decisions = everruns_core::ExecutionFeatureDecisions::from_env(grade);
    let mut registry = CapabilityRegistry::new();
    registry.register_inventory_plugins(|plugin| {
        (!plugin.experimental_only || grade.experimental_features_enabled())
            && plugin
                .feature_flag
                .is_none_or(|flag| decisions.is_enabled(flag))
    });
    registry
}

#[test]
fn cursor_plugin_is_submitted() {
    let plugins: Vec<&IntegrationPlugin> = inventory::iter::<IntegrationPlugin>().collect();
    assert!(
        plugins.iter().any(|p| {
            let cap = (p.factory)();
            cap.id() == "cursor"
        }),
        "Cursor IntegrationPlugin should be submitted via inventory"
    );
}

#[test]
fn cursor_plugin_is_registered_in_prod_and_dev() {
    let dev = registry_for_grade(DeploymentGrade::Dev);
    let prod = registry_for_grade(DeploymentGrade::Prod);
    assert!(dev.has("cursor"), "Cursor should be in dev registry");
    assert!(prod.has("cursor"), "Cursor should be in prod registry");
}

#[test]
fn cursor_capability_metadata() {
    let registry = registry_for_grade(DeploymentGrade::Dev);
    let cap = registry.get("cursor").expect("Cursor capability not found");
    assert_eq!(cap.id(), "cursor");
    assert_eq!(cap.name(), "Cursor");
    assert_eq!(cap.icon(), Some("code"));
    assert_eq!(cap.category(), Some("Execution"));
    assert_eq!(cap.tools().len(), 9);
}

#[test]
fn cursor_connection_provider_is_submitted() {
    let plugins: Vec<&ConnectorPlugin> = inventory::iter::<ConnectorPlugin>().collect();
    let plugin = plugins
        .iter()
        .find(|p| {
            let provider = (p.factory)();
            provider.provider_id() == "cursor"
        })
        .expect("Cursor connection plugin not found");
    assert!(!plugin.experimental_only);

    let provider = (plugin.factory)();
    let schema = provider.form_schema().expect("should have form schema");
    assert_eq!(schema.fields.len(), 1);
    assert_eq!(schema.fields[0].name, "api_key");
    assert!(schema.instructions_markdown.contains("Cloud Agents"));
}

#[test]
fn cursor_narration_names_agent_work() {
    use everruns_core::capabilities::Capability;
    let cap = everruns_integrations_cursor::CursorCapability;
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
            everruns_core::tool_narration::ToolNarrationContext::default(),
        )
        .expect("narration");
    assert_eq!(narration, "Starting Cursor agent: Fix checkout bug");
}
