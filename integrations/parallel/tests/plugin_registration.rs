//! Integration test: verify Parallel plugin registers via inventory.

use everruns_core::capabilities::{CapabilityRegistry, IntegrationPlugin};
use everruns_core::connection_provider::ConnectionProviderPlugin;
use everruns_core::deployment::DeploymentGrade;

use everruns_integrations_parallel as _;

#[test]
fn parallel_plugin_is_submitted() {
    let plugins: Vec<&IntegrationPlugin> = inventory::iter::<IntegrationPlugin>().collect();
    assert!(
        plugins.iter().any(|p| {
            let cap = (p.factory)();
            cap.id() == "parallel_search"
        }),
        "Parallel IntegrationPlugin should be submitted via inventory"
    );
}

#[test]
fn parallel_plugin_is_experimental() {
    let plugins: Vec<&IntegrationPlugin> = inventory::iter::<IntegrationPlugin>().collect();
    let plugin = plugins
        .iter()
        .find(|p| {
            let cap = (p.factory)();
            cap.id() == "parallel_search"
        })
        .expect("Parallel plugin not found");

    assert!(plugin.experimental_only);
}

#[test]
fn parallel_registered_in_dev_registry() {
    let registry = CapabilityRegistry::with_builtins_for_grade(DeploymentGrade::Dev);
    assert!(registry.has("parallel_search"));
}

#[test]
fn parallel_not_registered_in_prod_registry() {
    let registry = CapabilityRegistry::with_builtins_for_grade(DeploymentGrade::Prod);
    assert!(!registry.has("parallel_search"));
}

#[test]
fn connection_provider_is_submitted() {
    let plugins: Vec<&ConnectionProviderPlugin> =
        inventory::iter::<ConnectionProviderPlugin>().collect();
    assert!(
        plugins.iter().any(|p| {
            let provider = (p.factory)();
            provider.provider_id() == "parallel"
        }),
        "Parallel ConnectionProviderPlugin should be submitted via inventory"
    );
}

#[test]
fn connection_provider_has_api_key_form() {
    let plugins: Vec<&ConnectionProviderPlugin> =
        inventory::iter::<ConnectionProviderPlugin>().collect();
    let plugin = plugins
        .iter()
        .find(|p| {
            let provider = (p.factory)();
            provider.provider_id() == "parallel"
        })
        .expect("Parallel connection plugin not found");

    let provider = (plugin.factory)();
    let schema = provider.form_schema().expect("should have form schema");
    assert_eq!(schema.fields.len(), 1);
    assert_eq!(schema.fields[0].name, "api_key");
}
