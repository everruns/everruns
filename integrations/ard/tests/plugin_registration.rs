//! Plugin registration tests: the ARD capability auto-registers via inventory
//! and is gated to Dev (experimental).

use everruns_core::capabilities::{CapabilityRegistry, IntegrationPlugin};
use everruns_core::deployment::DeploymentGrade;
use everruns_integrations_ard as _; // force-link the crate so inventory submits run

#[test]
fn ard_plugin_is_submitted() {
    let plugins: Vec<&IntegrationPlugin> = inventory::iter::<IntegrationPlugin>().collect();
    assert!(
        plugins
            .iter()
            .any(|p| (p.factory)().id() == "resource_discovery"),
        "resource_discovery IntegrationPlugin should be submitted via inventory"
    );
}

#[test]
fn ard_plugin_is_experimental() {
    let plugins: Vec<&IntegrationPlugin> = inventory::iter::<IntegrationPlugin>().collect();
    let plugin = plugins
        .iter()
        .find(|p| (p.factory)().id() == "resource_discovery")
        .expect("plugin not found");
    assert!(plugin.experimental_only);
}

#[test]
fn ard_registered_in_dev_registry() {
    let registry = CapabilityRegistry::with_builtins_for_grade(DeploymentGrade::Dev);
    assert!(registry.has("resource_discovery"));
}

#[test]
fn ard_not_registered_in_prod_registry() {
    let registry = CapabilityRegistry::with_builtins_for_grade(DeploymentGrade::Prod);
    assert!(!registry.has("resource_discovery"));
}
