//! Integration test: verify Docker Container plugin registers via inventory.

use everruns_core::capabilities::{CapabilityRegistry, IntegrationPlugin};
use everruns_core::deployment::DeploymentGrade;

// Force linker to include the integration crate's inventory submissions.
use everruns_integrations_docker as _;

#[test]
fn test_docker_plugin_is_submitted() {
    let plugins: Vec<&IntegrationPlugin> = inventory::iter::<IntegrationPlugin>().collect();
    assert!(
        plugins.iter().any(|p| {
            let cap = (p.factory)();
            cap.id() == "docker_container"
        }),
        "Docker Container IntegrationPlugin should be submitted via inventory"
    );
}

#[test]
fn test_docker_plugin_is_experimental() {
    let plugins: Vec<&IntegrationPlugin> = inventory::iter::<IntegrationPlugin>().collect();
    let docker = plugins
        .iter()
        .find(|p| {
            let cap = (p.factory)();
            cap.id() == "docker_container"
        })
        .expect("Docker Container plugin not found");

    assert!(
        docker.experimental_only,
        "Docker Container should be marked experimental_only"
    );
}

#[test]
fn test_docker_registered_in_dev_registry() {
    let registry = CapabilityRegistry::with_builtins_for_grade(DeploymentGrade::Dev);
    assert!(
        registry.has("docker_container"),
        "Docker Container should be in dev registry"
    );
}

#[test]
fn test_docker_not_registered_in_prod_registry() {
    let registry = CapabilityRegistry::with_builtins_for_grade(DeploymentGrade::Prod);
    assert!(
        !registry.has("docker_container"),
        "Docker Container should NOT be in prod registry"
    );
}

#[test]
fn test_docker_capability_metadata() {
    let registry = CapabilityRegistry::with_builtins_for_grade(DeploymentGrade::Dev);
    let cap = registry
        .get("docker_container")
        .expect("Docker Container capability not found");

    assert_eq!(cap.id(), "docker_container");
    assert_eq!(cap.name(), "[Experimental] Docker Container");
    assert_eq!(cap.icon(), Some("container"));
    assert_eq!(cap.category(), Some("Development"));
    assert_eq!(cap.tools().len(), 5);
}
