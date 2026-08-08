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
    EXECUTE_DESCRIPTION as PLATFORM_EXECUTE_DESCRIPTION, PLATFORM_CAPABILITY_ID, PlatformCapability,
    QUERY_DESCRIPTION as PLATFORM_QUERY_DESCRIPTION, discover_input_schema, execute_input_schema,
    query_input_schema,
};
pub use platform_management::{
    ManageAgentsTool, ManageHarnessesTool, ManageSessionsTool, PLATFORM_MANAGEMENT_CAPABILITY_ID,
    PlatformManagementCapability, ReadAgentsTool, ReadCapabilitiesTool, ReadHarnessesTool,
    ReadSessionsTool, SessionReadMessagesTool, SessionReadResponseTool, SessionSendMessageTool,
};

/// Register the hosted platform-management capabilities on a registry (EVE-839).
///
/// Server/worker call this after building the portable `everruns-core` builtins
/// so hosted deployments keep the `platform` and `platform_management`
/// capabilities that were previously registered inside core.
pub fn register_platform_capabilities(
    registry: &mut everruns_core::capabilities::CapabilityRegistry,
) {
    registry.register(PlatformCapability);
    registry.register(PlatformManagementCapability);
}

/// Portable `everruns-core` builtins plus the hosted platform capabilities,
/// grade-selected. The hosted registry server/worker use for catalog,
/// validation, and execution (EVE-839).
pub fn hosted_capability_registry_for_grade(
    grade: everruns_core::DeploymentGrade,
) -> everruns_core::capabilities::CapabilityRegistry {
    let mut registry = everruns_core::capabilities::CapabilityRegistry::with_builtins_for_grade(grade);
    register_platform_capabilities(&mut registry);
    registry
}

/// [`hosted_capability_registry_for_grade`] with the grade taken from the
/// environment (mirrors `CapabilityRegistry::with_builtins`).
pub fn hosted_capability_registry() -> everruns_core::capabilities::CapabilityRegistry {
    let mut registry = everruns_core::capabilities::CapabilityRegistry::with_builtins();
    register_platform_capabilities(&mut registry);
    registry
}
