#![cfg_attr(test, allow(clippy::unwrap_used, clippy::expect_used))]
//! Integration test: verify the ARD plugin and connector register via inventory.

use everruns_contracts::connector::ConnectorPlugin;
use everruns_core::capabilities::{CapabilityRegistry, IntegrationPlugin};
use everruns_core::deployment::DeploymentGrade;

use everruns_ard::{CAPABILITY_PLUGINS, CONNECTOR_PLUGINS};

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
fn capability_is_unflagged_and_registered_everywhere() {
    let plugins: Vec<&IntegrationPlugin> = CAPABILITY_PLUGINS.iter().collect();
    let plugin = plugins
        .iter()
        .find(|p| (p.factory)().id() == "resource_discovery")
        .expect("plugin not found");
    assert_eq!(plugin.feature_flag, None);

    let dev = registry_for_grade(DeploymentGrade::Dev);
    assert!(dev.has("resource_discovery"), "should be in dev registry");
    let prod = registry_for_grade(DeploymentGrade::Prod);
    assert!(
        prod.has("resource_discovery"),
        "ungated capability should be in prod registry"
    );
}

#[test]
fn connector_is_submitted_with_form_schema() {
    let plugins: Vec<&ConnectorPlugin> = CONNECTOR_PLUGINS.iter().collect();
    let plugin = plugins
        .iter()
        .find(|p| (p.factory)().provider_id() == "ard")
        .expect("ard ConnectorPlugin should be published in CONNECTOR_PLUGINS");
    assert_eq!(plugin.feature_flag, None);
    let provider = (plugin.factory)();
    let schema = provider.form_schema().expect("should have form schema");
    assert_eq!(schema.fields[0].name, "api_key");
}
