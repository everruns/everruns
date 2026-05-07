//! Integration test: verify Cursor plugin and connection provider registration.

use everruns_core::capabilities::{CapabilityRegistry, IntegrationPlugin, ToolCallHook};
use everruns_core::connection_provider::ConnectionProviderPlugin;
use everruns_core::deployment::DeploymentGrade;
use everruns_core::tool_narration::ToolNarrationPhase;
use everruns_core::tool_types::ToolCall;
use serde_json::json;

// Force linker to include the integration crate's inventory submissions.
use everruns_integrations_cursor as _;

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
    let dev = CapabilityRegistry::with_builtins_for_grade(DeploymentGrade::Dev);
    let prod = CapabilityRegistry::with_builtins_for_grade(DeploymentGrade::Prod);
    assert!(dev.has("cursor"), "Cursor should be in dev registry");
    assert!(prod.has("cursor"), "Cursor should be in prod registry");
}

#[test]
fn cursor_capability_metadata() {
    let registry = CapabilityRegistry::with_builtins_for_grade(DeploymentGrade::Dev);
    let cap = registry.get("cursor").expect("Cursor capability not found");
    assert_eq!(cap.id(), "cursor");
    assert_eq!(cap.name(), "Cursor");
    assert_eq!(cap.icon(), Some("code"));
    assert_eq!(cap.category(), Some("Execution"));
    assert_eq!(cap.tools().len(), 9);
}

#[test]
fn cursor_connection_provider_is_submitted() {
    let plugins: Vec<&ConnectionProviderPlugin> =
        inventory::iter::<ConnectionProviderPlugin>().collect();
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
fn cursor_narration_hook_names_agent_work() {
    let cap = everruns_integrations_cursor::CursorCapability;
    use everruns_core::capabilities::Capability;
    let hooks = cap.tool_call_hooks();
    let hook: &dyn ToolCallHook = hooks[0].as_ref();
    let call = ToolCall {
        id: "call_1".into(),
        name: "cursor_launch_agent".into(),
        arguments: json!({
            "name": "Fix checkout bug",
            "repository": "https://github.com/acme/app",
            "prompt": "Fix it"
        }),
    };

    let narration = hook
        .narration(None, &call, ToolNarrationPhase::Started, None)
        .expect("narration");
    assert_eq!(narration, "Starting Cursor agent: Fix checkout bug");
}
