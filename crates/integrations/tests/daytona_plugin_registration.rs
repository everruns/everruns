#![cfg_attr(test, allow(clippy::unwrap_used, clippy::expect_used))]
//! Integration tests for Daytona plugin registration and capability.

use everruns_contracts::runtime::capabilities::{CapabilityRegistry, IntegrationPlugin};
use everruns_contracts::runtime::deployment::DeploymentGrade;

use everruns_integrations::daytona::{CAPABILITY_PLUGINS, CONNECTOR_PLUGINS};

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
fn test_daytona_plugin_is_published() {
    let plugins: Vec<&IntegrationPlugin> = CAPABILITY_PLUGINS.iter().collect();
    assert!(
        plugins.iter().any(|p| {
            let cap = (p.factory)();
            cap.id() == "daytona"
        }),
        "Daytona IntegrationPlugin should be published in CAPABILITY_PLUGINS"
    );
}

#[test]
fn test_daytona_plugin_is_not_behind_a_feature_flag() {
    let plugins: Vec<&IntegrationPlugin> = CAPABILITY_PLUGINS.iter().collect();
    let daytona = plugins
        .iter()
        .find(|p| {
            let cap = (p.factory)();
            cap.id() == "daytona"
        })
        .expect("Daytona plugin not found");

    assert!(
        daytona.feature_flag.is_none(),
        "daytona should not be behind a feature flag"
    );
}

#[test]
fn test_daytona_connector_is_not_behind_a_feature_flag() {
    let daytona = CONNECTOR_PLUGINS
        .iter()
        .find(|plugin| (plugin.factory)().provider_id() == "daytona")
        .expect("Daytona connector not found");

    assert!(
        daytona.feature_flag.is_none(),
        "daytona connection setup must be available wherever the ungated sandbox capability is available"
    );
}

#[test]
fn test_daytona_registered_in_dev_registry() {
    let registry = registry_for_grade(DeploymentGrade::Dev);
    assert!(registry.has("daytona"), "Daytona should be in dev registry");
}

#[test]
fn test_daytona_registered_in_prod_registry() {
    let registry = registry_for_grade(DeploymentGrade::Prod);
    assert!(
        registry.has("daytona"),
        "Daytona should be in prod registry"
    );
}

#[test]
fn test_daytona_capability_metadata() {
    let registry = registry_for_grade(DeploymentGrade::Dev);
    let cap = registry
        .get("daytona")
        .expect("Daytona capability not found");

    assert_eq!(cap.id(), "daytona");
    assert_eq!(cap.name(), "Daytona");
    assert_eq!(cap.icon(), Some("daytona"));
    assert_eq!(cap.category(), Some("Sandboxes"));
    assert_eq!(cap.dependencies(), vec!["session_storage"]);
    assert_eq!(cap.tools().len(), 10);
}

#[test]
fn test_desktop_computer_use_is_not_feature_flagged() {
    let id = everruns_integrations::daytona::computer::DAYTONA_COMPUTER_USE_CAPABILITY_ID;
    let plugin = CAPABILITY_PLUGINS
        .iter()
        .find(|p| (p.factory)().id() == id)
        .expect("Daytona desktop computer use plugin not found");
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
