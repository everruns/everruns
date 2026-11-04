#![cfg_attr(test, allow(clippy::unwrap_used, clippy::expect_used))]
//! Integration test: verify Brave Search plugin is published by the crate catalog.

#![cfg(feature = "brave-search-hosted")]

use everruns_contracts::runtime::capabilities::CapabilityRegistry;
use everruns_contracts::runtime::deployment::DeploymentGrade;

use everruns_integrations::brave_search::CAPABILITY_PLUGINS;

fn registry_for_grade(grade: DeploymentGrade) -> CapabilityRegistry {
    let mut registry = CapabilityRegistry::new();
    registry.register_plugins(CAPABILITY_PLUGINS.iter(), |plugin| {
        plugin.feature_flag.is_none_or(|flag| {
            everruns_contracts::runtime::feature_flag_available(flag, &[], grade)
        })
    });
    registry
}

#[test]
fn test_brave_search_registered_in_prod_registry() {
    let registry = registry_for_grade(DeploymentGrade::Prod);
    assert!(
        registry.has("brave_search"),
        "Brave Search has no feature flag, so it is in the prod registry"
    );
}
