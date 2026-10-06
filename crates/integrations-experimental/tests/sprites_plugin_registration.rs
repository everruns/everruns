#![cfg_attr(test, allow(clippy::unwrap_used, clippy::expect_used))]
//! Integration tests for Sprites plugin registration and capability.

use everruns_contracts::runtime::capabilities::{CapabilityRegistry, IntegrationPlugin};
use everruns_contracts::runtime::deployment::DeploymentGrade;

use everruns_integrations_experimental::sprites::CAPABILITY_PLUGINS;

fn registry_for_grade(grade: DeploymentGrade) -> CapabilityRegistry {
    let mut registry = CapabilityRegistry::new();
    registry.register_plugins(CAPABILITY_PLUGINS.iter(), |plugin| {
        plugin.feature_flag.is_none_or(|flag| {
            everruns_contracts::runtime::feature_flag_available(
                flag,
                everruns_integrations_experimental::sprites::FEATURE_FLAGS,
                grade,
            )
        })
    });
    registry
}

#[test]
fn test_sprites_plugin_is_published() {
    let plugins: Vec<&IntegrationPlugin> = CAPABILITY_PLUGINS.iter().collect();
    assert!(
        plugins.iter().any(|p| {
            let cap = (p.factory)();
            cap.id() == "sprites"
        }),
        "Sprites IntegrationPlugin should be published in CAPABILITY_PLUGINS"
    );
}

#[test]
fn test_sprites_plugin_is_behind_its_feature_flag() {
    let plugins: Vec<&IntegrationPlugin> = CAPABILITY_PLUGINS.iter().collect();
    let sprites = plugins
        .iter()
        .find(|p| {
            let cap = (p.factory)();
            cap.id() == "sprites"
        })
        .expect("Sprites plugin not found");

    assert_eq!(
        sprites.feature_flag,
        Some("sprites"),
        "sprites should be behind its feature flag"
    );
}

#[test]
fn test_sprites_registered_in_dev_registry() {
    let registry = registry_for_grade(DeploymentGrade::Dev);
    assert!(registry.has("sprites"), "Sprites should be in dev registry");
}

#[test]
fn test_sprites_not_registered_in_prod_registry() {
    let registry = registry_for_grade(DeploymentGrade::Prod);
    assert!(
        !registry.has("sprites"),
        "Experimental Sprites should stay out of the prod registry"
    );
}

#[test]
fn test_sprites_capability_metadata() {
    let registry = registry_for_grade(DeploymentGrade::Dev);
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
