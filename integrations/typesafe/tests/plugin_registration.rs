//! The capability and its connector register through inventory.

#![cfg(feature = "hosted")]

use everruns_core::capabilities::{CapabilityRegistry, IntegrationPlugin};
use everruns_core::deployment::DeploymentGrade;
use everruns_platform::connector::ConnectorPlugin;

// Force the linker to include the integration crate's inventory submissions.
use everruns_integrations_typesafe as _;

fn registry_for_grade(grade: DeploymentGrade) -> CapabilityRegistry {
    let decisions = everruns_core::ExecutionFeatureDecisions::from_env(grade);
    let mut registry = CapabilityRegistry::new();
    registry.register_inventory_plugins(|plugin| {
        (!plugin.experimental_only || grade.experimental_features_enabled())
            && plugin
                .feature_flag
                .is_none_or(|flag| decisions.is_enabled(flag))
    });
    registry
}

fn typesafe_plugin() -> &'static IntegrationPlugin {
    inventory::iter::<IntegrationPlugin>()
        .find(|plugin| (plugin.factory)().id() == "typesafe")
        .expect("TypeSafe IntegrationPlugin should be submitted via inventory")
}

#[test]
fn plugin_is_experimental_only() {
    assert!(typesafe_plugin().experimental_only);
    assert!(typesafe_plugin().feature_flag.is_none());
}

#[test]
fn capability_is_available_in_dev_and_withheld_in_prod() {
    assert!(registry_for_grade(DeploymentGrade::Dev).has("typesafe"));
    assert!(!registry_for_grade(DeploymentGrade::Prod).has("typesafe"));
}

#[test]
fn capability_metadata_is_stable() {
    let registry = registry_for_grade(DeploymentGrade::Dev);
    let capability = registry.get("typesafe").expect("capability");
    assert_eq!(capability.name(), "[Experimental] TypeSafe Judgments");
    assert_eq!(capability.icon(), Some("scale"));
    assert_eq!(capability.category(), Some("Reasoning"));
    assert!(capability.dependencies().is_empty());
    assert_eq!(capability.tools().len(), 1);
}

#[test]
fn connector_is_submitted_with_an_api_key_form() {
    let plugin = inventory::iter::<ConnectorPlugin>()
        .find(|plugin| (plugin.factory)().provider_id() == "typesafe")
        .expect("TypeSafe ConnectorPlugin should be submitted via inventory");
    assert!(plugin.experimental_only);
    let connector = (plugin.factory)();
    let schema = connector.form_schema().expect("form schema");
    assert_eq!(schema.fields.len(), 1);
    assert_eq!(schema.fields[0].name, "api_key");
}
