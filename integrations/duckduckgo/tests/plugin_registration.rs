//! Integration test: verify DuckDuckGo plugin is published by the crate catalog.

use everruns_core::capabilities::{CapabilityRegistry, IntegrationPlugin};
use everruns_core::deployment::DeploymentGrade;

use everruns_integrations_duckduckgo::CAPABILITY_PLUGINS;

fn registry_for_grade(grade: DeploymentGrade) -> CapabilityRegistry {
    let decisions = everruns_core::ExecutionFeatureDecisions::from_env(grade);
    let mut registry = CapabilityRegistry::new();
    registry.register_plugins(CAPABILITY_PLUGINS.iter(), |plugin| {
        (!plugin.experimental_only || grade.experimental_features_enabled())
            && plugin
                .feature_flag
                .is_none_or(|flag| decisions.is_enabled(flag))
    });
    registry
}

#[test]
fn test_duckduckgo_plugin_is_published() {
    let plugins: Vec<&IntegrationPlugin> = CAPABILITY_PLUGINS.iter().collect();
    assert!(
        plugins.iter().any(|p| {
            let cap = (p.factory)();
            cap.id() == "duckduckgo"
        }),
        "DuckDuckGo IntegrationPlugin should be published in CAPABILITY_PLUGINS"
    );
}

#[test]
fn test_duckduckgo_plugin_is_experimental() {
    let plugins: Vec<&IntegrationPlugin> = CAPABILITY_PLUGINS.iter().collect();
    let duckduckgo = plugins
        .iter()
        .find(|p| {
            let cap = (p.factory)();
            cap.id() == "duckduckgo"
        })
        .expect("DuckDuckGo plugin not found");

    assert!(
        duckduckgo.experimental_only,
        "DuckDuckGo should be marked experimental_only"
    );
}

#[test]
fn test_duckduckgo_registered_in_dev_registry() {
    let registry = registry_for_grade(DeploymentGrade::Dev);
    assert!(
        registry.has("duckduckgo"),
        "DuckDuckGo should be in dev registry"
    );
}

#[test]
fn test_duckduckgo_not_registered_in_prod_registry() {
    let registry = registry_for_grade(DeploymentGrade::Prod);
    assert!(
        !registry.has("duckduckgo"),
        "DuckDuckGo should NOT be in prod registry"
    );
}

#[test]
fn test_duckduckgo_capability_metadata() {
    let registry = registry_for_grade(DeploymentGrade::Dev);
    let cap = registry
        .get("duckduckgo")
        .expect("DuckDuckGo capability not found");

    assert_eq!(cap.id(), "duckduckgo");
    assert_eq!(cap.name(), "[Experimental] DuckDuckGo");
    assert_eq!(cap.icon(), Some("search"));
    assert_eq!(cap.category(), Some("Network"));
    assert!(cap.dependencies().is_empty());
    assert_eq!(cap.tools().len(), 1);
}
