//! The capability and its connector are published as plugin consts and named
//! by `everruns-capabilities::integrations_catalog`.

#![cfg(feature = "typesafe-hosted")]
#![cfg_attr(test, allow(clippy::unwrap_used, clippy::expect_used))]

use everruns_contracts::runtime::capabilities::CapabilityRegistry;
use everruns_contracts::runtime::deployment::DeploymentGrade;

use everruns_integrations::typesafe::CAPABILITY_PLUGINS;

fn registry_for_grade(grade: DeploymentGrade) -> CapabilityRegistry {
    let mut registry = CapabilityRegistry::new();
    registry.register_plugins(CAPABILITY_PLUGINS.iter(), |plugin| {
        plugin.feature_flag.is_none_or(|flag| {
            everruns_contracts::runtime::feature_flag_available(flag, std::iter::empty(), grade)
        })
    });
    registry
}

#[test]
fn capability_is_available_in_dev_and_prod() {
    assert!(registry_for_grade(DeploymentGrade::Dev).has("jev"));
    assert!(registry_for_grade(DeploymentGrade::Prod).has("jev"));
}
