#![cfg_attr(test, allow(clippy::unwrap_used, clippy::expect_used))]
//! Integration tests for E2B plugin registration and capability.

use everruns_contracts::connector::ConnectorPlugin;
use everruns_contracts::runtime::capabilities::{CapabilityRegistry, IntegrationPlugin};
use everruns_contracts::runtime::deployment::DeploymentGrade;

use everruns_integrations_e2b::{CAPABILITY_PLUGINS, CONNECTOR_PLUGINS};

fn registry_for_grade(grade: DeploymentGrade) -> CapabilityRegistry {
    let decisions = everruns_contracts::runtime::ExecutionFeatureDecisions::from_env(grade);
    let mut registry = CapabilityRegistry::new();
    registry.register_plugins(CAPABILITY_PLUGINS.iter(), |plugin| {
        (!plugin.experimental_only || grade.experimental_features_enabled())
            && plugin
                .feature_flag
                .is_none_or(|flag| decisions.is_enabled(flag))
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
fn test_e2b_plugin_is_not_experimental() {
    let plugins: Vec<&IntegrationPlugin> = CAPABILITY_PLUGINS.iter().collect();
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
fn test_desktop_computer_use_is_experimental_only() {
    let id = everruns_integrations_e2b::computer::DESKTOP_COMPUTER_USE_CAPABILITY_ID;
    let plugin = CAPABILITY_PLUGINS
        .iter()
        .find(|p| (p.factory)().id() == id)
        .expect("desktop computer use plugin not found");
    assert!(plugin.experimental_only);
    assert!(plugin.feature_flag.is_none());

    let dev = registry_for_grade(DeploymentGrade::Dev);
    let cap = dev
        .get(id)
        .expect("desktop computer use in the dev registry");
    assert_eq!(cap.dependencies(), vec!["session_storage"]);
    let tools = cap.tools();
    assert_eq!(tools.len(), 1);
    assert_eq!(tools[0].name(), "computer");
    assert!(
        !registry_for_grade(DeploymentGrade::Prod).has(id),
        "desktop computer use must stay out of prod while experimental"
    );
}
