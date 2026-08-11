//! Hosted platform-management capabilities (EVE-839).
//!
//! `PlatformCapability` (catalog-backed `platform_*` surface) and
//! `PlatformManagementCapability` (org-scoped CRUD compatibility tools) are
//! hosted-only: they require a `PlatformStore` and are registered by the
//! server/worker host composition, never by the portable `everruns-core`
//! default registry. They were carved out of `everruns-core` so the portable
//! runtime carries no `PlatformStore` seam.

pub mod platform;
pub mod platform_management;
pub mod util;

pub use platform::{
    DISCOVER_DESCRIPTION as PLATFORM_DISCOVER_DESCRIPTION,
    EXECUTE_DESCRIPTION as PLATFORM_EXECUTE_DESCRIPTION, PLATFORM_CAPABILITY_ID,
    PlatformCapability, QUERY_DESCRIPTION as PLATFORM_QUERY_DESCRIPTION, discover_input_schema,
    execute_input_schema, query_input_schema,
};
pub use platform_management::{
    ManageAgentsTool, ManageHarnessesTool, ManageSessionsTool, PLATFORM_MANAGEMENT_CAPABILITY_ID,
    PlatformManagementCapability, ReadAgentsTool, ReadCapabilitiesTool, ReadHarnessesTool,
    ReadSessionsTool, SessionReadMessagesTool, SessionReadResponseTool, SessionSendMessageTool,
};

/// Register the hosted platform-management capabilities on a registry (EVE-839).
///
/// Server/worker call this after composing the neutral core preset, portable
/// policy bundle, and environment integrations so hosted deployments keep the
/// `platform` and `platform_management` capabilities that were previously
/// registered inside core.
pub fn register_platform_capabilities(
    registry: &mut everruns_core::capabilities::CapabilityRegistry,
) {
    registry.register(PlatformCapability);
    registry.register(PlatformManagementCapability);
}

/// Register environment-backed capabilities compiled into the hosted product.
#[cfg(feature = "environment-capabilities")]
pub fn register_environment_capabilities(
    registry: &mut everruns_core::capabilities::CapabilityRegistry,
) {
    registry.register(everruns_integrations_filesystem::FileSystemCapability);
    registry.register(everruns_integrations_bashkit::BashkitShellCapability);
    registry.register(everruns_integrations_web_fetch::WebFetchCapability::from_env());
    registry.register(everruns_integrations_openrouter_workspace::ModelScoutCapability);
    registry.register(everruns_integrations_openrouter_workspace::OpenRouterWorkspaceCapability);

    #[cfg(feature = "lua")]
    if everruns_core::InternalFeatureFlags::from_env().lua {
        registry.register(everruns_integrations_lua::LuaCapability);
        registry.register(everruns_integrations_lua::LuaCodeModeCapability);
    }
}

#[cfg(not(feature = "environment-capabilities"))]
fn register_environment_capabilities(
    _registry: &mut everruns_core::capabilities::CapabilityRegistry,
) {
}

/// Grade-selected neutral core preset, optional portable policy bundle,
/// environment integrations, and hosted platform capabilities. This is the
/// registry server/worker use for catalog, validation, and execution (EVE-839).
pub fn hosted_capability_registry_for_grade(
    grade: everruns_core::DeploymentGrade,
) -> everruns_core::capabilities::CapabilityRegistry {
    let mut registry =
        everruns_core::capabilities::CapabilityRegistry::with_builtins_for_grade(grade);
    #[cfg(feature = "portable-builtins")]
    everruns_builtins::register_portable_capabilities(&mut registry)
        .expect("core and portable built-in catalogs must not collide");
    register_environment_capabilities(&mut registry);
    register_platform_capabilities(&mut registry);
    registry
}

/// [`hosted_capability_registry_for_grade`] with the grade taken from the
/// environment (mirrors `CapabilityRegistry::with_builtins`).
pub fn hosted_capability_registry() -> everruns_core::capabilities::CapabilityRegistry {
    let mut registry = everruns_core::capabilities::CapabilityRegistry::with_builtins();
    #[cfg(feature = "portable-builtins")]
    everruns_builtins::register_portable_capabilities(&mut registry)
        .expect("core and portable built-in catalogs must not collide");
    register_environment_capabilities(&mut registry);
    register_platform_capabilities(&mut registry);
    registry
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn hosted_registry_always_contains_platform_capabilities() {
        let registry = hosted_capability_registry_for_grade(everruns_core::DeploymentGrade::Prod);
        assert!(registry.has(PLATFORM_CAPABILITY_ID));
        assert!(registry.has(PLATFORM_MANAGEMENT_CAPABILITY_ID));
    }

    #[cfg(feature = "portable-builtins")]
    #[test]
    fn hosted_registry_contains_full_portable_policy_catalog() {
        let registry = hosted_capability_registry_for_grade(everruns_core::DeploymentGrade::Prod);
        for capability_id in [
            "current_time",
            "compaction",
            "tool_call_repair",
            "usage_limit_auto_continue",
        ] {
            assert!(
                registry.has(capability_id),
                "hosted product registry is missing `{capability_id}`"
            );
        }
    }

    #[cfg(feature = "environment-capabilities")]
    #[test]
    fn hosted_registry_composes_product_environment_capabilities() {
        let registry = hosted_capability_registry_for_grade(everruns_core::DeploymentGrade::Prod);
        for capability_id in [
            "session_file_system",
            "bashkit_shell",
            "web_fetch",
            "model_scout",
            "openrouter_workspace",
        ] {
            assert!(
                registry.has(capability_id),
                "hosted product registry is missing `{capability_id}`"
            );
        }
        assert_eq!(registry.canonical_id("virtual_bash"), Some("bashkit_shell"));
    }

    #[cfg(not(feature = "environment-capabilities"))]
    #[test]
    fn platform_default_does_not_link_environment_capabilities() {
        let registry = hosted_capability_registry_for_grade(everruns_core::DeploymentGrade::Prod);
        for capability_id in [
            "session_file_system",
            "bashkit_shell",
            "web_fetch",
            "model_scout",
            "openrouter_workspace",
        ] {
            assert!(!registry.has(capability_id));
        }
    }
}
