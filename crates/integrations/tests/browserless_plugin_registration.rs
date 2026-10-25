#![allow(clippy::unwrap_used, clippy::expect_used)]
//! Integration tests for Browserless plugin registration and capability.

use everruns_contracts::runtime::capabilities::CapabilityRegistry;
use everruns_contracts::runtime::deployment::DeploymentGrade;

use everruns_integrations::browserless::CAPABILITY_PLUGINS;

fn registry_for_grade(grade: DeploymentGrade) -> CapabilityRegistry {
    let mut registry = CapabilityRegistry::new();
    registry.register_plugins(CAPABILITY_PLUGINS.iter(), |plugin| {
        plugin.feature_flag.is_none_or(|flag| {
            everruns_contracts::runtime::feature_flag_available(
                flag,
                everruns_integrations::browserless::FEATURE_FLAGS,
                grade,
            )
        })
    });
    registry
}

#[test]
fn test_browserless_registered_in_prod_registry() {
    let registry = registry_for_grade(DeploymentGrade::Prod);
    assert!(
        registry.has("browserless"),
        "Browserless should be in prod registry"
    );
}
