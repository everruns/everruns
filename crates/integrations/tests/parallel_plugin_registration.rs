#![cfg_attr(test, allow(clippy::unwrap_used, clippy::expect_used))]
//! Integration test: verify Parallel plugin is published by the crate catalog.

use everruns_contracts::runtime::capabilities::CapabilityRegistry;

use everruns_integrations::parallel::{CAPABILITY_PLUGINS, PARALLEL_CAPABILITY_ID};

#[test]
fn only_the_free_search_capability_is_published() {
    let mut registry = CapabilityRegistry::new();
    registry.register_plugins(CAPABILITY_PLUGINS.iter(), |plugin| {
        plugin.feature_flag.is_none()
    });
    assert!(registry.has(PARALLEL_CAPABILITY_ID));
    // The paid `parallel` capability (machine payments) was removed.
    assert!(!registry.has("parallel"));
    assert_eq!(CAPABILITY_PLUGINS.len(), 1);
}
