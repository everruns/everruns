//! The capability and its connector are published as plugin consts and named
//! by `everruns-integrations-catalog`.

#![cfg(feature = "hosted")]
#![cfg_attr(test, allow(clippy::unwrap_used, clippy::expect_used))]

use everruns_core::capabilities::{CapabilityRegistry, IntegrationPlugin};
use everruns_core::deployment::DeploymentGrade;

use everruns_integrations_typesafe::{CAPABILITY_PLUGINS, CONNECTOR_PLUGINS};

fn registry_for_grade(grade: DeploymentGrade) -> CapabilityRegistry {
    let decisions = everruns_core::ExecutionFeatureDecisions::from_env(grade);
    let mut registry = CapabilityRegistry::new();
    registry.register_plugins(CAPABILITY_PLUGINS.iter(), |plugin| {
        (!plugin.experimental_only || grade.experimental_features_enabled())
            && plugin
                .feature_flag
                .is_none_or(|flag| decisions.is_enabled(flag))
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
fn plugin_is_experimental_only() {
    assert!(jev_plugin().experimental_only);
    assert!(jev_plugin().feature_flag.is_none());
}

#[test]
fn capability_is_available_in_dev_and_withheld_in_prod() {
    assert!(registry_for_grade(DeploymentGrade::Dev).has("jev"));
    assert!(!registry_for_grade(DeploymentGrade::Prod).has("jev"));
}

#[test]
fn capability_metadata_is_stable() {
    let registry = registry_for_grade(DeploymentGrade::Dev);
    let capability = registry.get("jev").expect("capability");
    assert_eq!(capability.name(), "[Experimental] Jev Classifications");
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
    assert!(plugin.experimental_only);
    let connector = (plugin.factory)();
    let schema = connector.form_schema().expect("form schema");
    assert_eq!(schema.fields.len(), 1);
    assert_eq!(schema.fields[0].name, "api_key");
}
