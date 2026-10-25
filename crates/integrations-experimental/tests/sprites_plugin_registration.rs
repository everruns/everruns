#![cfg_attr(test, allow(clippy::unwrap_used, clippy::expect_used))]
//! Integration tests for Sprites plugin registration and capability.

use everruns_contracts::runtime::capabilities::CapabilityRegistry;
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
fn test_sprites_is_off_in_every_grade() {
    let registry = registry_for_grade(DeploymentGrade::Dev);
    assert!(
        !registry.has("sprites"),
        "Sprites is off by default, even in dev"
    );
}

#[test]
fn test_sprites_not_registered_in_prod_registry() {
    let registry = registry_for_grade(DeploymentGrade::Prod);
    assert!(
        !registry.has("sprites"),
        "Experimental Sprites should stay out of the prod registry"
    );
}
