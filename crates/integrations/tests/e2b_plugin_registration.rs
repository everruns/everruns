#![cfg_attr(test, allow(clippy::unwrap_used, clippy::expect_used))]
//! Integration tests for E2B plugin registration and capability.

use everruns_contracts::connector::ConnectorPlugin;
use everruns_contracts::runtime::capabilities::{CapabilityRegistry, IntegrationPlugin};
use everruns_contracts::runtime::deployment::DeploymentGrade;

use everruns_integrations::e2b::{CAPABILITY_PLUGINS, CONNECTOR_PLUGINS};

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
fn test_e2b_plugin_is_published() {
    let plugins: Vec<&IntegrationPlugin> = CAPABILITY_PLUGINS.iter().collect();
    assert!(
        plugins.iter().any(|p| {
            let cap = (p.factory)();
            cap.id() == "e2b"
        }),
        "E2B IntegrationPlugin should be published in CAPABILITY_PLUGINS"
    );
}

#[test]
fn test_e2b_plugin_is_not_behind_a_feature_flag() {
    let plugins: Vec<&IntegrationPlugin> = CAPABILITY_PLUGINS.iter().collect();
    let e2b = plugins
        .iter()
        .find(|p| {
            let cap = (p.factory)();
            cap.id() == "e2b"
        })
        .expect("E2B plugin not found");

    assert!(
        e2b.feature_flag.is_none(),
        "e2b should not be behind a feature flag"
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
fn test_e2b_connection_provider_is_published() {
    let plugins: Vec<&ConnectorPlugin> = CONNECTOR_PLUGINS.iter().collect();
    assert!(
        plugins.iter().any(|plugin| {
            let provider = (plugin.factory)();
            provider.provider_id() == "e2b"
        }),
        "E2B ConnectorPlugin should be published in CONNECTOR_PLUGINS"
    );
}

#[test]
fn test_desktop_computer_use_is_not_feature_flagged() {
    let id = everruns_integrations::e2b::computer::DESKTOP_COMPUTER_USE_CAPABILITY_ID;
    let plugin = CAPABILITY_PLUGINS
        .iter()
        .find(|p| (p.factory)().id() == id)
        .expect("desktop computer use plugin not found");
    assert_eq!(plugin.feature_flag, None);

    let dev = registry_for_grade(DeploymentGrade::Dev);
    let cap = dev
        .get(id)
        .expect("desktop computer use in the dev registry");
    assert_eq!(cap.dependencies(), vec!["session_storage"]);
    let tools = cap.tools();
    assert_eq!(tools.len(), 1);
    assert_eq!(tools[0].name(), "computer");
    assert!(
        registry_for_grade(DeploymentGrade::Prod).has(id),
        "desktop computer use is ungated, so prod registers it"
    );
}
