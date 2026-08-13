//! Integration tests for E2B plugin registration and capability.

use everruns_core::capabilities::{CapabilityRegistry, IntegrationPlugin};
use everruns_core::deployment::DeploymentGrade;
use everruns_platform::connector::ConnectorPlugin;

// Force linker to include the integration crate's inventory submissions.
use everruns_integrations_e2b as _;

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
fn test_e2b_plugin_is_submitted() {
    let plugins: Vec<&IntegrationPlugin> = inventory::iter::<IntegrationPlugin>().collect();
    assert!(
        plugins.iter().any(|p| {
            let cap = (p.factory)();
            cap.id() == "e2b"
        }),
        "E2B IntegrationPlugin should be submitted via inventory"
    );
}

#[test]
fn test_e2b_plugin_is_not_experimental() {
    let plugins: Vec<&IntegrationPlugin> = inventory::iter::<IntegrationPlugin>().collect();
    let e2b = plugins
        .iter()
        .find(|p| {
            let cap = (p.factory)();
            cap.id() == "e2b"
        })
        .expect("E2B plugin not found");

    assert!(
        !e2b.experimental_only,
        "E2B should NOT be marked experimental_only"
    );
}

#[test]
fn test_e2b_registered_in_dev_registry() {
    let registry = registry_for_grade(DeploymentGrade::Dev);
    assert!(registry.has("e2b"), "E2B should be in dev registry");
}

#[test]
fn test_e2b_registered_in_prod_registry() {
    let registry = registry_for_grade(DeploymentGrade::Prod);
    assert!(registry.has("e2b"), "E2B should be in prod registry");
}

#[test]
fn test_e2b_capability_metadata() {
    let registry = registry_for_grade(DeploymentGrade::Dev);
    let cap = registry.get("e2b").expect("E2B capability not found");

    assert_eq!(cap.id(), "e2b");
    assert_eq!(cap.name(), "E2B");
    assert_eq!(cap.icon(), Some("cloud"));
    assert_eq!(cap.category(), Some("Sandboxes"));
    assert_eq!(cap.dependencies(), vec!["session_storage"]);
    assert_eq!(cap.tools().len(), 6);
}

#[test]
fn test_e2b_connection_provider_is_submitted() {
    let plugins: Vec<&ConnectorPlugin> = inventory::iter::<ConnectorPlugin>().collect();
    assert!(
        plugins.iter().any(|plugin| {
            let provider = (plugin.factory)();
            provider.provider_id() == "e2b"
        }),
        "E2B ConnectorPlugin should be submitted via inventory"
    );
}
