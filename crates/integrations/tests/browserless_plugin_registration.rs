#![allow(clippy::unwrap_used, clippy::expect_used)]
//! Integration tests for Browserless plugin registration and capability.

use everruns_contracts::runtime::capabilities::CapabilityRegistry;
use everruns_integrations::browserless::{CAPABILITY_PLUGINS, CONNECTOR_PLUGINS};

#[test]
fn browserless_surfaces_register_without_rollout_flags() {
    let mut registry = CapabilityRegistry::new();
    registry.register_plugins(CAPABILITY_PLUGINS.iter(), |plugin| {
        plugin.feature_flag.is_none()
    });
    for id in ["browserless", "browser_use", "computer_use"] {
        assert!(registry.has(id), "{id} is built in");
    }
    assert!(
        CONNECTOR_PLUGINS
            .iter()
            .all(|plugin| plugin.feature_flag.is_none())
    );
}
