//! Integration tests for Deno plugin registration and capability.

use everruns_core::capabilities::{CapabilityRegistry, IntegrationPlugin};
use everruns_core::deployment::DeploymentGrade;
use everruns_integrations_deno as _;
use everruns_platform::connector::ConnectorPlugin;

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
fn test_deno_plugin_is_submitted() {
    let plugins: Vec<&IntegrationPlugin> = inventory::iter::<IntegrationPlugin>().collect();
    assert!(
        plugins.iter().any(|plugin| {
            let capability = (plugin.factory)();
            capability.id() == "deno"
        }),
        "Deno IntegrationPlugin should be submitted via inventory"
    );
}

#[test]
fn test_deno_registered_in_prod_registry() {
    let registry = registry_for_grade(DeploymentGrade::Prod);
    assert!(registry.has("deno"), "Deno should be in prod registry");
}

#[test]
fn test_deno_connection_provider_is_submitted() {
    let plugins: Vec<&ConnectorPlugin> = inventory::iter::<ConnectorPlugin>().collect();
    assert!(
        plugins.iter().any(|plugin| {
            let provider = (plugin.factory)();
            provider.provider_id() == "deno"
        }),
        "Deno ConnectorPlugin should be submitted via inventory"
    );
}
