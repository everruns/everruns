use everruns_core::capabilities::{CapabilityRegistry, IntegrationPlugin};

// Force linker to include the integration crate's inventory submissions.
use everruns_integrations_github as _;

#[test]
fn plugin_is_submitted() {
    let plugins: Vec<&IntegrationPlugin> = inventory::iter::<IntegrationPlugin>().collect();
    assert!(
        plugins.iter().any(|plugin| {
            let cap = (plugin.factory)();
            cap.id() == "github_scout"
        }),
        "GitHub IntegrationPlugin should be submitted via inventory"
    );
}

#[test]
fn registry_includes_github_capability_and_blueprint() {
    let mut registry = CapabilityRegistry::new();
    registry.register_inventory_plugins(|_| true);

    let cap = registry
        .get("github_scout")
        .expect("github_scout capability should be registered");
    assert!(cap.tools().is_empty());
    assert_eq!(cap.dependencies(), vec!["subagents"]);

    let blueprint = registry
        .blueprint("github_scout")
        .expect("github_scout blueprint should be registered");
    assert_eq!(blueprint.name, "GitHub Scout");
}
