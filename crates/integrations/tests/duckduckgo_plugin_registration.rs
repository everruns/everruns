#![cfg_attr(test, allow(clippy::unwrap_used, clippy::expect_used))]
//! Integration test: verify DuckDuckGo plugin is published by the crate catalog.

use everruns_contracts::runtime::capabilities::{CapabilityRegistry, IntegrationPlugin};
use everruns_contracts::runtime::deployment::DeploymentGrade;

use everruns_integrations::duckduckgo::CAPABILITY_PLUGINS;

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
fn test_duckduckgo_plugin_is_published() {
    let plugins: Vec<&IntegrationPlugin> = CAPABILITY_PLUGINS.iter().collect();
    assert!(
        plugins.iter().any(|p| {
            let cap = (p.factory)();
            cap.id() == "duckduckgo"
        }),
        "DuckDuckGo IntegrationPlugin should be published in CAPABILITY_PLUGINS"
    );
}

#[test]
fn test_duckduckgo_plugin_is_not_feature_flagged() {
    let plugins: Vec<&IntegrationPlugin> = CAPABILITY_PLUGINS.iter().collect();
    let duckduckgo = plugins
        .iter()
        .find(|p| {
            let cap = (p.factory)();
            cap.id() == "duckduckgo"
        })
        .expect("DuckDuckGo plugin not found");

    assert_eq!(
        duckduckgo.feature_flag, None,
        "duckduckgo should not be behind a feature flag"
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
fn test_duckduckgo_registered_in_prod_registry() {
    let registry = registry_for_grade(DeploymentGrade::Prod);
    assert!(
        registry.has("duckduckgo"),
        "DuckDuckGo is ungated, so it should be in prod registry"
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
