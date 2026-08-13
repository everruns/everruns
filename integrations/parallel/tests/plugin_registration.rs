//! Integration test: verify Parallel plugin registers via inventory.

use everruns_core::capabilities::{CapabilityRegistry, IntegrationPlugin};
use everruns_core::deployment::DeploymentGrade;
use everruns_platform::connector::ConnectorPlugin;

use everruns_integrations_parallel as _;

fn registry_for_grade(grade: DeploymentGrade) -> CapabilityRegistry {
    let decisions = everruns_core::ExecutionFeatureDecisions::from_env(grade);
    let mut registry = CapabilityRegistry::new();
    registry.register_inventory_plugins(|plugin| {
        (!plugin.experimental_only || grade.experimental_features_enabled())
            && plugin
                .feature_flag
                .is_none_or(|flag| decisions.is_enabled(flag))
    });
    registry
}

#[test]
fn parallel_plugin_is_submitted() {
    let plugins: Vec<&IntegrationPlugin> = inventory::iter::<IntegrationPlugin>().collect();
    assert!(
        plugins.iter().any(|p| {
            let cap = (p.factory)();
            cap.id() == "parallel_search"
        }),
        "Parallel IntegrationPlugin should be submitted via inventory"
    );
}

#[test]
fn parallel_plugin_is_experimental() {
    let plugins: Vec<&IntegrationPlugin> = inventory::iter::<IntegrationPlugin>().collect();
    let plugin = plugins
        .iter()
        .find(|p| {
            let cap = (p.factory)();
            cap.id() == "parallel_search"
        })
        .expect("Parallel plugin not found");

    assert!(plugin.experimental_only);
}

#[test]
fn parallel_registered_in_dev_registry() {
    let registry = registry_for_grade(DeploymentGrade::Dev);
    assert!(registry.has("parallel_search"));
}

#[test]
fn parallel_not_registered_in_prod_registry() {
    let registry = registry_for_grade(DeploymentGrade::Prod);
    assert!(!registry.has("parallel_search"));
}

// Env-var-mutating tests must not run in parallel with each other.
static ENV_LOCK: std::sync::Mutex<()> = std::sync::Mutex::new(());

/// Restore an env var to a previously captured value, or remove it if it was unset.
fn restore_env(key: &str, prev: Option<String>) {
    match prev {
        Some(value) => unsafe { std::env::set_var(key, value) },
        None => unsafe { std::env::remove_var(key) },
    }
}

#[test]
fn payments_plugin_is_flag_gated_not_experimental() {
    let plugins: Vec<&IntegrationPlugin> = inventory::iter::<IntegrationPlugin>().collect();
    let plugin = plugins
        .iter()
        .find(|p| (p.factory)().id() == "parallel")
        .expect("Parallel payments plugin not found");

    // Gated by the machine_payments flag rather than the experimental grade, so it
    // can be enabled deliberately in any environment but never registers by default.
    assert!(!plugin.experimental_only);
    assert_eq!(plugin.feature_flag, Some("machine_payments"));
}

#[test]
fn payments_capability_disabled_by_default() {
    let _lock = ENV_LOCK.lock().unwrap_or_else(|e| e.into_inner());
    let prev = std::env::var("FEATURE_MACHINE_PAYMENTS").ok();
    unsafe { std::env::remove_var("FEATURE_MACHINE_PAYMENTS") };
    // Off by default even in dev: spend is irreversible.
    let registry = registry_for_grade(DeploymentGrade::Dev);
    assert!(!registry.has("parallel"));
    restore_env("FEATURE_MACHINE_PAYMENTS", prev);
}

#[test]
fn payments_capability_enabled_by_flag() {
    let _lock = ENV_LOCK.lock().unwrap_or_else(|e| e.into_inner());
    let prev = std::env::var("FEATURE_MACHINE_PAYMENTS").ok();
    unsafe { std::env::set_var("FEATURE_MACHINE_PAYMENTS", "true") };
    // Deliberate enablement works in any grade, including prod.
    let registry = registry_for_grade(DeploymentGrade::Prod);
    assert!(registry.has("parallel"));
    restore_env("FEATURE_MACHINE_PAYMENTS", prev);
}

#[test]
fn connection_provider_is_submitted() {
    let plugins: Vec<&ConnectorPlugin> = inventory::iter::<ConnectorPlugin>().collect();
    assert!(
        plugins.iter().any(|p| {
            let provider = (p.factory)();
            provider.provider_id() == "parallel"
        }),
        "Parallel ConnectorPlugin should be submitted via inventory"
    );
}

#[test]
fn connection_provider_has_api_key_form() {
    let plugins: Vec<&ConnectorPlugin> = inventory::iter::<ConnectorPlugin>().collect();
    let plugin = plugins
        .iter()
        .find(|p| {
            let provider = (p.factory)();
            provider.provider_id() == "parallel"
        })
        .expect("Parallel connection plugin not found");

    let provider = (plugin.factory)();
    let schema = provider.form_schema().expect("should have form schema");
    assert_eq!(schema.fields.len(), 1);
    assert_eq!(schema.fields[0].name, "api_key");
}
