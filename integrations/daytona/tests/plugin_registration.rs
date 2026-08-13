//! Integration tests for Daytona plugin registration and capability.

use everruns_core::capabilities::{CapabilityRegistry, IntegrationPlugin};
use everruns_core::deployment::DeploymentGrade;

// Force linker to include the integration crate's inventory submissions.
use everruns_integrations_daytona as _;

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
fn test_daytona_plugin_is_submitted() {
    let plugins: Vec<&IntegrationPlugin> = inventory::iter::<IntegrationPlugin>().collect();
    assert!(
        plugins.iter().any(|p| {
            let cap = (p.factory)();
            cap.id() == "daytona"
        }),
        "Daytona IntegrationPlugin should be submitted via inventory"
    );
}

#[test]
fn test_daytona_plugin_is_not_experimental() {
    let plugins: Vec<&IntegrationPlugin> = inventory::iter::<IntegrationPlugin>().collect();
    let daytona = plugins
        .iter()
        .find(|p| {
            let cap = (p.factory)();
            cap.id() == "daytona"
        })
        .expect("Daytona plugin not found");

    assert!(
        !daytona.experimental_only,
        "Daytona should NOT be marked experimental_only"
    );
}

#[test]
fn test_daytona_registered_in_dev_registry() {
    let registry = registry_for_grade(DeploymentGrade::Dev);
    assert!(registry.has("daytona"), "Daytona should be in dev registry");
}

#[test]
fn test_daytona_registered_in_prod_registry() {
    let registry = registry_for_grade(DeploymentGrade::Prod);
    assert!(
        registry.has("daytona"),
        "Daytona should be in prod registry"
    );
}

#[test]
fn test_daytona_capability_metadata() {
    let registry = registry_for_grade(DeploymentGrade::Dev);
    let cap = registry
        .get("daytona")
        .expect("Daytona capability not found");

    assert_eq!(cap.id(), "daytona");
    assert_eq!(cap.name(), "Daytona");
    assert_eq!(cap.icon(), Some("daytona"));
    assert_eq!(cap.category(), Some("Sandboxes"));
    assert_eq!(cap.dependencies(), vec!["session_storage"]);
    assert_eq!(cap.tools().len(), 10);
}
