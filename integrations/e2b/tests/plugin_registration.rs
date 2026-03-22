use everruns_core::capabilities::{Capability, IntegrationPlugin};
use everruns_integrations_e2b::E2BCapability;

#[test]
fn capability_metadata() {
    let capability = E2BCapability;
    assert_eq!(capability.id(), "e2b");
    assert_eq!(capability.name(), "E2B");
    assert_eq!(capability.icon(), Some("cloud"));
    assert_eq!(capability.category(), Some("Execution"));
}

#[test]
fn plugin_registered() {
    let plugins: Vec<&IntegrationPlugin> =
        inventory::iter::<IntegrationPlugin>.into_iter().collect();
    assert!(
        plugins
            .iter()
            .any(|plugin| (plugin.factory)().id() == "e2b")
    );
}
