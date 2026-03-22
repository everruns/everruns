//! Integration tests for Deno plugin registration and capability.

use everruns_core::capabilities::{CapabilityRegistry, IntegrationPlugin};
use everruns_core::connection_provider::ConnectionProviderPlugin;
use everruns_core::deployment::DeploymentGrade;
use everruns_integrations_deno as _;

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
    let registry = CapabilityRegistry::with_builtins_for_grade(DeploymentGrade::Prod);
    assert!(registry.has("deno"), "Deno should be in prod registry");
}

#[test]
fn test_deno_connection_provider_is_submitted() {
    let plugins: Vec<&ConnectionProviderPlugin> =
        inventory::iter::<ConnectionProviderPlugin>().collect();
    assert!(
        plugins.iter().any(|plugin| {
            let provider = (plugin.factory)();
            provider.provider_id() == "deno"
        }),
        "Deno ConnectionProviderPlugin should be submitted via inventory"
    );
}
