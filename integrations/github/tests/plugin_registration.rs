use everruns_core::capabilities::{CapabilityRegistry, IntegrationPlugin};

use everruns_integrations_github::CAPABILITY_PLUGINS;

#[test]
fn plugin_is_published() {
    let plugins: Vec<&IntegrationPlugin> = CAPABILITY_PLUGINS.iter().collect();
    assert!(
        plugins.iter().any(|plugin| {
            let cap = (plugin.factory)();
            cap.id() == "github_scout"
        }),
        "GitHub IntegrationPlugin should be published in CAPABILITY_PLUGINS"
    );
}

#[test]
fn registry_includes_github_capability_and_blueprint() {
    let mut registry = CapabilityRegistry::new();
    registry.register_plugins(CAPABILITY_PLUGINS.iter(), |_| true);

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
