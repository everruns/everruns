//! Default OSS platform definition helpers.
//!
//! The default OSS platform stays centralized here so server startup, org
//! initialization, and docs can all point to the same preset. Inventory-based
//! integration discovery is intentionally confined to this module; embedders
//! can start from the OSS preset or construct a `PlatformDefinition` manually
//! without depending on inventory registration.

use everruns_core::connector::{ConnectorPlugin, ConnectorRegistry};
use everruns_core::deployment::DeploymentGrade;
use everruns_core::{
    BuiltInHarnessDefinition, CapabilityRegistry, DirectEgressService, PlatformDefinition,
    SystemEmailConfig, SystemUtilityLlmConfig,
};
use std::sync::Arc;

/// Build the default OSS `PlatformDefinition` for the current deployment grade.
pub fn oss_platform_definition() -> PlatformDefinition {
    oss_platform_definition_for_grade(DeploymentGrade::from_env())
}

/// Build the default OSS `PlatformDefinition` for an explicit deployment grade.
pub fn oss_platform_definition_for_grade(grade: DeploymentGrade) -> PlatformDefinition {
    let capability_registry = CapabilityRegistry::with_builtins_for_grade(grade);
    let driver_registry = everruns_worker::create_driver_registry();
    let connectors = oss_connector_registry_for_grade(grade);
    // Resolve the optional host-wide system allowlist from the environment
    // (EVERRUNS_SYSTEM_ALLOWLIST_ENABLED). Disabled by default.
    let egress_service = Arc::new(DirectEgressService::from_env());
    let email_sender = SystemEmailConfig::from_env()
        .expect("Invalid system email configuration")
        .into_sender_with_egress(egress_service.clone());
    let utility_llm_service = SystemUtilityLlmConfig::from_env().into_service();

    PlatformDefinition::builder()
        .capability_registry(capability_registry)
        .driver_registry(driver_registry)
        .connectors(connectors)
        .built_in_harnesses(oss_built_in_harnesses())
        .egress_service(egress_service)
        .email_sender(email_sender)
        .utility_llm_service(utility_llm_service)
        .session_file_system_factory(Arc::new(
            crate::domains::session_files::StorageSessionFileSystemFactory,
        ))
        .build()
}

/// Build the default OSS connector registry.
pub fn oss_connector_registry() -> ConnectorRegistry {
    oss_connector_registry_for_grade(DeploymentGrade::from_env())
}

/// Build the default OSS connector registry for an explicit grade.
pub fn oss_connector_registry_for_grade(grade: DeploymentGrade) -> ConnectorRegistry {
    let mut registry = ConnectorRegistry::new();

    for plugin in inventory::iter::<ConnectorPlugin> {
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
