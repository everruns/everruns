//! Integration tests for Sprites plugin registration and capability.

use everruns_core::capabilities::{CapabilityRegistry, IntegrationPlugin};
use everruns_core::deployment::DeploymentGrade;

// Force linker to include the integration crate's inventory submissions.
use everruns_integrations_sprites as _;

#[test]
fn test_sprites_plugin_is_submitted() {
    let plugins: Vec<&IntegrationPlugin> = inventory::iter::<IntegrationPlugin>().collect();
    assert!(
        plugins.iter().any(|p| {
            let cap = (p.factory)();
            cap.id() == "sprites"
        }),
        "Sprites IntegrationPlugin should be submitted via inventory"
    );
}

#[test]
fn test_sprites_plugin_is_not_experimental() {
    let plugins: Vec<&IntegrationPlugin> = inventory::iter::<IntegrationPlugin>().collect();
    let sprites = plugins
        .iter()
        .find(|p| {
            let cap = (p.factory)();
            cap.id() == "sprites"
        })
        .expect("Sprites plugin not found");

    assert!(
        !sprites.experimental_only,
        "Sprites should NOT be marked experimental_only"
    );
}

#[test]
fn test_sprites_registered_in_dev_registry() {
    let registry = CapabilityRegistry::with_builtins_for_grade(DeploymentGrade::Dev);
    assert!(registry.has("sprites"), "Sprites should be in dev registry");
}

#[test]
fn test_sprites_registered_in_prod_registry() {
    let registry = CapabilityRegistry::with_builtins_for_grade(DeploymentGrade::Prod);
    assert!(
        registry.has("sprites"),
        "Sprites should be in prod registry"
    );
}

#[test]
fn test_sprites_capability_metadata() {
    let registry = CapabilityRegistry::with_builtins_for_grade(DeploymentGrade::Dev);
    let cap = registry
        .get("sprites")
        .expect("Sprites capability not found");

    assert_eq!(cap.id(), "sprites");
    assert_eq!(cap.name(), "Sprites");
    assert_eq!(cap.icon(), Some("sprites"));
    assert_eq!(cap.category(), Some("Execution"));
    assert_eq!(cap.dependencies(), vec!["session_storage"]);
    assert_eq!(cap.tools().len(), 9);
}
