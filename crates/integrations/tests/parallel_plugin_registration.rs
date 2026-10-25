#![cfg_attr(test, allow(clippy::unwrap_used, clippy::expect_used))]
//! Integration test: verify Parallel plugin is published by the crate catalog.

use everruns_contracts::runtime::capabilities::CapabilityRegistry;
use everruns_contracts::runtime::deployment::DeploymentGrade;

use everruns_integrations::parallel::CAPABILITY_PLUGINS;

fn registry_for_grade(grade: DeploymentGrade) -> CapabilityRegistry {
    let mut registry = CapabilityRegistry::new();
    registry.register_plugins(CAPABILITY_PLUGINS.iter(), |plugin| {
        plugin.feature_flag.is_none_or(|flag| {
            everruns_contracts::runtime::feature_flag_available(flag, &[], grade)
        })
    });
    registry
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
    unsafe { std::env::set_var("FEATURE_MACHINE_PAYMENTS", "prod") };
    // Deliberate enablement works in any grade, including prod.
    let registry = registry_for_grade(DeploymentGrade::Prod);
    assert!(registry.has("parallel"));
    restore_env("FEATURE_MACHINE_PAYMENTS", prev);
}
