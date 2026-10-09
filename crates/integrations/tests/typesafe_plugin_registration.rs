//! The capability and its connector are published as plugin consts and named
//! by `everruns-capabilities::integrations_catalog`.

#![cfg(feature = "typesafe-hosted")]
#![cfg_attr(test, allow(clippy::unwrap_used, clippy::expect_used))]

use everruns_contracts::runtime::capabilities::{CapabilityRegistry, IntegrationPlugin};
use everruns_contracts::runtime::deployment::DeploymentGrade;

use everruns_integrations::typesafe::{CAPABILITY_PLUGINS, CONNECTOR_PLUGINS};

fn registry_for_grade(grade: DeploymentGrade) -> CapabilityRegistry {
    let mut registry = CapabilityRegistry::new();
    registry.register_plugins(CAPABILITY_PLUGINS.iter(), |plugin| {
        plugin.feature_flag.is_none_or(|flag| {
            everruns_contracts::runtime::feature_flag_available(
                flag,
                everruns_integrations::typesafe::FEATURE_FLAGS,
                grade,
            )
        })
    });
    registry
}

fn jev_plugin() -> &'static IntegrationPlugin {
    CAPABILITY_PLUGINS
        .iter()
        .find(|plugin| (plugin.factory)().id() == "jev")
        .expect("Jev IntegrationPlugin should be published in CAPABILITY_PLUGINS")
}

#[test]
fn plugin_is_behind_its_feature_flag() {
    assert_eq!(jev_plugin().feature_flag, Some("typesafe"));
}

#[test]
fn capability_is_available_in_dev_and_prod() {
    assert!(registry_for_grade(DeploymentGrade::Dev).has("jev"));
    assert!(registry_for_grade(DeploymentGrade::Prod).has("jev"));
}

#[test]
fn capability_metadata_is_stable() {
    let registry = registry_for_grade(DeploymentGrade::Dev);
    let capability = registry.get("jev").expect("capability");
    assert_eq!(capability.name(), "[Experimental] Jev Decisions");
    assert_eq!(capability.icon(), Some("scale"));
    assert_eq!(capability.category(), Some("Reasoning"));
    assert!(capability.dependencies().is_empty());
    assert_eq!(capability.tools().len(), 1);
}

#[test]
fn connector_is_published_with_an_api_key_form() {
    let plugin = CONNECTOR_PLUGINS
        .iter()
        .find(|plugin| (plugin.factory)().provider_id() == "typesafe")
        .expect("TypeSafe ConnectorPlugin should be published in CONNECTOR_PLUGINS");
    assert_eq!(plugin.feature_flag, Some("typesafe"));
    let connector = (plugin.factory)();
    let schema = connector.form_schema().expect("form schema");
    assert_eq!(schema.fields.len(), 1);
    assert_eq!(schema.fields[0].name, "api_key");
}
