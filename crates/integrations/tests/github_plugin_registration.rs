#![cfg_attr(test, allow(clippy::unwrap_used, clippy::expect_used))]
use everruns_contracts::runtime::capabilities::CapabilityRegistry;

use everruns_integrations::github::CAPABILITY_PLUGINS;

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
