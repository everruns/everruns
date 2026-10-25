#![cfg_attr(test, allow(clippy::unwrap_used, clippy::expect_used))]
//! Integration tests for Daytona plugin registration and capability.

use everruns_contracts::runtime::capabilities::CapabilityRegistry;
use everruns_contracts::runtime::deployment::DeploymentGrade;

use everruns_integrations::daytona::CAPABILITY_PLUGINS;

fn registry_for_grade(grade: DeploymentGrade) -> CapabilityRegistry {
    let mut registry = CapabilityRegistry::new();
    registry.register_plugins(CAPABILITY_PLUGINS.iter(), |plugin| {
        plugin.feature_flag.is_none_or(|flag| {
            everruns_contracts::runtime::feature_flag_available(
                flag,
                everruns_integrations::daytona::FEATURE_FLAGS,
                grade,
            )
        })
    });
    registry
}

#[test]
fn test_desktop_computer_use_is_behind_its_feature_flag() {
    let id = everruns_integrations::daytona::computer::DAYTONA_COMPUTER_USE_CAPABILITY_ID;
    let plugin = CAPABILITY_PLUGINS
        .iter()
        .find(|p| (p.factory)().id() == id)
        .expect("Daytona desktop computer use plugin not found");
    assert_eq!(plugin.feature_flag, Some("daytona_computer_use"));

    let dev = registry_for_grade(DeploymentGrade::Dev);
    let cap = dev
        .get(id)
        .expect("desktop computer use in the dev registry");
    assert_eq!(cap.dependencies(), vec!["session_storage"]);
    let tools = cap.tools();
    assert_eq!(tools.len(), 1);
    assert_eq!(tools[0].name(), "computer");
    assert!(
        registry_for_grade(DeploymentGrade::Prod).has(id),
        "desktop computer use is adoption grade, so prod registers it for opted-in organisations"
    );
}
