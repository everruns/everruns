// Cross-cutting infrastructure modules.
//
// Domain-owned business logic lives under `crate::domains::*`. The remaining
// top-level `services` modules are infra adapters, validators, listeners, or
// other shared helpers that do not define a separate user-facing service layer.

#[cfg(test)]
mod budget_tests;
pub mod capability;
pub(crate) mod capability_validation;
pub mod eval_runner;
pub mod event;
pub mod leased_resource;
pub mod llm_resolver;
pub mod model_sync;
pub mod principal;
pub mod scoped_mcp;
pub mod usage_tracking;
pub mod virtual_mount_registry;

pub use capability::CapabilityService;
pub use event::EventService;
pub use leased_resource::LeasedResourceService;
pub use llm_resolver::{LlmResolverService, ResolvedModel};
pub use model_sync::{ModelSyncService, SyncResult};
pub use principal::{PrincipalService, row_to_principal};
pub use usage_tracking::UsageTrackingListener;
