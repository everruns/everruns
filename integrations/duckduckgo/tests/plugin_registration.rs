//! Integration test: verify DuckDuckGo plugin registers via inventory.

use everruns_core::capabilities::{CapabilityRegistry, IntegrationPlugin};
use everruns_core::deployment::DeploymentGrade;

// Force linker to include the integration crate's inventory submissions.
use everruns_integrations_duckduckgo as _;

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
fn test_duckduckgo_plugin_is_submitted() {
    let plugins: Vec<&IntegrationPlugin> = inventory::iter::<IntegrationPlugin>().collect();
    assert!(
        plugins.iter().any(|p| {
            let cap = (p.factory)();
            cap.id() == "duckduckgo"
        }),
        "DuckDuckGo IntegrationPlugin should be submitted via inventory"
    );
}

#[test]
fn test_duckduckgo_plugin_is_experimental() {
    let plugins: Vec<&IntegrationPlugin> = inventory::iter::<IntegrationPlugin>().collect();
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
