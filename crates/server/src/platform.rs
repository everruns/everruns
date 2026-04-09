//! Default OSS platform definition helpers.
//!
//! The default OSS platform stays centralized here so server startup, org
//! initialization, and docs can all point to the same preset. Inventory-based
//! integration discovery is intentionally confined to this module; embedders
//! can start from the OSS preset or construct a `PlatformDefinition` manually
//! without depending on inventory registration.

use everruns_core::connection_provider::{ConnectionProviderPlugin, ConnectionProviderRegistry};
use everruns_core::deployment::DeploymentGrade;
use everruns_core::{BuiltInHarnessDefinition, CapabilityRegistry, PlatformDefinition};

/// Build the default OSS `PlatformDefinition` for the current deployment grade.
pub fn oss_platform_definition() -> PlatformDefinition {
    oss_platform_definition_for_grade(DeploymentGrade::from_env())
}

/// Build the default OSS `PlatformDefinition` for an explicit deployment grade.
pub fn oss_platform_definition_for_grade(grade: DeploymentGrade) -> PlatformDefinition {
    let capability_registry = CapabilityRegistry::with_builtins_for_grade(grade);
    let driver_registry = everruns_worker::create_driver_registry();
    let connection_providers = oss_connection_provider_registry_for_grade(grade);

    PlatformDefinition::builder()
        .capability_registry(capability_registry)
        .driver_registry(driver_registry)
        .connection_providers(connection_providers)
        .built_in_harnesses(oss_built_in_harnesses())
        .build()
}

/// Build the default OSS connection-provider registry.
pub fn oss_connection_provider_registry() -> ConnectionProviderRegistry {
    oss_connection_provider_registry_for_grade(DeploymentGrade::from_env())
}

/// Build the default OSS connection-provider registry for an explicit grade.
pub fn oss_connection_provider_registry_for_grade(
    grade: DeploymentGrade,
) -> ConnectionProviderRegistry {
    let mut registry = ConnectionProviderRegistry::new();

    for plugin in inventory::iter::<ConnectionProviderPlugin> {
        if plugin.experimental_only && !grade.experimental_features_enabled() {
            continue;
        }
        registry.register_boxed((plugin.factory)());
    }

    registry
}

/// Built-in harness templates for the default OSS platform.
pub fn oss_built_in_harnesses() -> Vec<BuiltInHarnessDefinition> {
    crate::harnesses::built_in_harnesses()
}
