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
