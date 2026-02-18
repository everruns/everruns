//! Integration test: verify Brave Search plugin registers via inventory.

use everruns_core::capabilities::{CapabilityRegistry, IntegrationPlugin};
use everruns_core::deployment::DeploymentGrade;

// Force linker to include the integration crate's inventory submissions.
use everruns_integrations_brave_search as _;

#[test]
fn test_brave_search_plugin_is_submitted() {
    let plugins: Vec<&IntegrationPlugin> = inventory::iter::<IntegrationPlugin>().collect();
    assert!(
        plugins.iter().any(|p| {
            let cap = (p.factory)();
            cap.id() == "brave_search"
        }),
        "Brave Search IntegrationPlugin should be submitted via inventory"
    );
}

#[test]
fn test_brave_search_plugin_is_experimental() {
    let plugins: Vec<&IntegrationPlugin> = inventory::iter::<IntegrationPlugin>().collect();
    let brave_search = plugins
        .iter()
        .find(|p| {
            let cap = (p.factory)();
            cap.id() == "brave_search"
        })
        .expect("Brave Search plugin not found");

    assert!(
        brave_search.experimental_only,
        "Brave Search should be marked experimental_only"
    );
}

#[test]
fn test_brave_search_registered_in_dev_registry() {
    let registry = CapabilityRegistry::with_builtins_for_grade(DeploymentGrade::Dev);
    assert!(
        registry.has("brave_search"),
        "Brave Search should be in dev registry"
    );
}

#[test]
fn test_brave_search_not_registered_in_prod_registry() {
    let registry = CapabilityRegistry::with_builtins_for_grade(DeploymentGrade::Prod);
    assert!(
        !registry.has("brave_search"),
        "Brave Search should NOT be in prod registry"
    );
}

#[test]
fn test_brave_search_capability_metadata() {
    let registry = CapabilityRegistry::with_builtins_for_grade(DeploymentGrade::Dev);
    let cap = registry
        .get("brave_search")
        .expect("Brave Search capability not found");

    assert_eq!(cap.id(), "brave_search");
    assert_eq!(cap.name(), "[Experimental] Brave Search");
    assert_eq!(cap.icon(), Some("search"));
    assert_eq!(cap.category(), Some("Network"));
    assert!(cap.dependencies().is_empty());
    assert_eq!(cap.tools().len(), 1);
}
