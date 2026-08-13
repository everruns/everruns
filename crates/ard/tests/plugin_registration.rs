//! Integration test: verify the ARD plugin and connector register via inventory.

use everruns_core::capabilities::{CapabilityRegistry, IntegrationPlugin};
use everruns_core::deployment::DeploymentGrade;
use everruns_platform::connector::ConnectorPlugin;

// Force linker to include the integration crate's inventory submissions.
use everruns_ard as _;

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

#[test]
fn capability_plugin_is_submitted() {
    let plugins: Vec<&IntegrationPlugin> = inventory::iter::<IntegrationPlugin>().collect();
    assert!(
        plugins
            .iter()
            .any(|p| (p.factory)().id() == "resource_discovery"),
        "resource_discovery IntegrationPlugin should be submitted via inventory"
    );
}

#[test]
fn capability_is_experimental_dev_only() {
    let plugins: Vec<&IntegrationPlugin> = inventory::iter::<IntegrationPlugin>().collect();
    let plugin = plugins
        .iter()
        .find(|p| (p.factory)().id() == "resource_discovery")
        .expect("plugin not found");
    assert!(plugin.experimental_only);

    let dev = registry_for_grade(DeploymentGrade::Dev);
    assert!(dev.has("resource_discovery"), "should be in dev registry");
    let prod = registry_for_grade(DeploymentGrade::Prod);
    assert!(
        !prod.has("resource_discovery"),
        "experimental capability should not be in prod registry"
    );
}

#[test]
fn capability_metadata_and_tools() {
    let registry = registry_for_grade(DeploymentGrade::Dev);
    let cap = registry
        .get("resource_discovery")
        .expect("capability not found");
    assert_eq!(cap.icon(), Some("compass"));
    assert_eq!(cap.category(), Some("Discovery"));
    let tools = cap.tools();
    assert_eq!(tools.len(), 3);
    let names: Vec<&str> = tools.iter().map(|t| t.name()).collect();
    assert!(names.contains(&"discover_resources"));
    assert!(names.contains(&"attach_resource"));
    assert!(names.contains(&"list_attached_resources"));
}

#[test]
fn connector_is_submitted_with_form_schema() {
    let plugins: Vec<&ConnectorPlugin> = inventory::iter::<ConnectorPlugin>().collect();
    let plugin = plugins
        .iter()
        .find(|p| (p.factory)().provider_id() == "ard")
        .expect("ard ConnectorPlugin should be submitted via inventory");
    assert!(plugin.experimental_only);
    let provider = (plugin.factory)();
    let schema = provider.form_schema().expect("should have form schema");
    assert_eq!(schema.fields[0].name, "api_key");
}
