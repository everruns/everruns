#![cfg_attr(test, allow(clippy::unwrap_used, clippy::expect_used))]
//! Integration tests for Daytona plugin registration.

use everruns_integrations::daytona::{CAPABILITY_PLUGINS, FEATURE_FLAGS};

/// The desktop is a Sandbox Template option (`sandbox_desktop`), so no
/// Daytona capability hides behind a feature flag any more.
#[test]
fn test_no_daytona_capability_is_feature_flagged() {
    assert!(FEATURE_FLAGS.is_empty());
    assert!(
        CAPABILITY_PLUGINS
            .iter()
            .all(|plugin| plugin.feature_flag.is_none())
    );
}
