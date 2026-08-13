//! Integration tests for Browserless plugin registration and capability.

use everruns_core::capabilities::{CapabilityRegistry, IntegrationPlugin};
use everruns_core::deployment::DeploymentGrade;

// Force linker to include the integration crate's inventory submissions.
use everruns_integrations_browserless as _;

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
fn test_browserless_plugin_is_submitted() {
    let plugins: Vec<&IntegrationPlugin> = inventory::iter::<IntegrationPlugin>().collect();
    assert!(
        plugins.iter().any(|p| {
            let cap = (p.factory)();
            cap.id() == "browserless"
        }),
        "Browserless IntegrationPlugin should be submitted via inventory"
    );
}

#[test]
fn test_browserless_plugin_is_not_experimental() {
    let plugins: Vec<&IntegrationPlugin> = inventory::iter::<IntegrationPlugin>().collect();
    let browserless = plugins
        .iter()
        .find(|p| {
            let cap = (p.factory)();
            cap.id() == "browserless"
        })
        .expect("Browserless plugin not found");

    assert!(
        !browserless.experimental_only,
        "Browserless should NOT be marked experimental_only"
    );
}

#[test]
fn test_browserless_registered_in_dev_registry() {
    let registry = registry_for_grade(DeploymentGrade::Dev);
    assert!(
        registry.has("browserless"),
        "Browserless should be in dev registry"
    );
}

#[test]
fn test_browserless_registered_in_prod_registry() {
    let registry = registry_for_grade(DeploymentGrade::Prod);
    assert!(
        registry.has("browserless"),
        "Browserless should be in prod registry"
    );
}

#[test]
fn test_browserless_capability_metadata() {
    let registry = registry_for_grade(DeploymentGrade::Dev);
    let cap = registry
        .get("browserless")
        .expect("Browserless capability not found");

    assert_eq!(cap.id(), "browserless");
    assert_eq!(cap.name(), "Browserless");
    assert_eq!(cap.icon(), Some("browserless"));
    assert_eq!(cap.category(), Some("Browser"));
    assert_eq!(cap.dependencies(), vec!["session_storage"]);
    assert_eq!(cap.tools().len(), 7);
}
