//! Integration test: verify CodeSandbox plugin registers via inventory.

use everruns_core::capabilities::{CapabilityRegistry, IntegrationPlugin};
use everruns_core::deployment::DeploymentGrade;

// Force linker to include the integration crate's inventory submissions.
use everruns_integrations_codesandbox as _;

#[test]
fn test_codesandbox_plugin_is_submitted() {
    let plugins: Vec<&IntegrationPlugin> = inventory::iter::<IntegrationPlugin>().collect();
    assert!(
        plugins.iter().any(|p| {
            let cap = (p.factory)();
            cap.id() == "codesandbox"
        }),
        "CodeSandbox IntegrationPlugin should be submitted via inventory"
    );
}

#[test]
fn test_codesandbox_plugin_is_not_experimental() {
    let plugins: Vec<&IntegrationPlugin> = inventory::iter::<IntegrationPlugin>().collect();
    let csb = plugins
        .iter()
        .find(|p| {
            let cap = (p.factory)();
            cap.id() == "codesandbox"
        })
        .expect("CodeSandbox plugin not found");

    assert!(
        !csb.experimental_only,
        "CodeSandbox should NOT be marked experimental_only"
    );
}

#[test]
fn test_codesandbox_registered_in_dev_registry() {
    let registry = CapabilityRegistry::with_builtins_for_grade(DeploymentGrade::Dev);
    assert!(
        registry.has("codesandbox"),
        "CodeSandbox should be in dev registry"
    );
}

#[test]
fn test_codesandbox_registered_in_prod_registry() {
    let registry = CapabilityRegistry::with_builtins_for_grade(DeploymentGrade::Prod);
    assert!(
        registry.has("codesandbox"),
        "CodeSandbox should be in prod registry"
    );
}

#[test]
fn test_codesandbox_capability_metadata() {
    let registry = CapabilityRegistry::with_builtins_for_grade(DeploymentGrade::Dev);
    let cap = registry
        .get("codesandbox")
        .expect("CodeSandbox capability not found");

    assert_eq!(cap.id(), "codesandbox");
    assert_eq!(cap.name(), "CodeSandbox");
    assert_eq!(cap.icon(), Some("cloud"));
    assert_eq!(cap.category(), Some("Execution"));
    assert_eq!(cap.dependencies(), vec!["session_storage"]);
    assert_eq!(cap.tools().len(), 9);
}

#[test]
fn test_dev_registry_count_with_integration() {
    let registry = CapabilityRegistry::with_builtins_for_grade(DeploymentGrade::Dev);
    // 17 core + integration plugins (codesandbox, docker_container when linked)
    // This test only links codesandbox, so expect at least 18
    assert!(registry.len() >= 18);
}
