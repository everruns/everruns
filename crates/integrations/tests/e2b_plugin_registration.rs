#![cfg_attr(test, allow(clippy::unwrap_used, clippy::expect_used))]
//! Integration tests for E2B plugin registration.

use everruns_integrations::e2b::{CAPABILITY_PLUGINS, FEATURE_FLAGS};

/// The desktop is a Sandbox Template option (`sandbox_desktop`), so no
/// E2B capability hides behind a feature flag any more.
#[test]
fn test_no_e2b_capability_is_feature_flagged() {
    assert!(FEATURE_FLAGS.is_empty());
    assert!(
        CAPABILITY_PLUGINS
            .iter()
            .all(|plugin| plugin.feature_flag.is_none())
    );
}
