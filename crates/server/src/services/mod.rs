// Cross-cutting infrastructure modules.
//
// Domain-owned business logic lives under `crate::domains::*`. The modules
// that remain here are infra adapters, validators, listeners, or shared
// helpers with no single owning domain:
//
// - `capability` — capability registry threaded through `Ctx`; used by every
//   domain.
// - `event` — event persistence + fanout listener; called by domains that
//   emit events.
// - `llm_resolver` — spans `llm_providers` + `llm_models` + params; no single
//   owner.
// - `model_sync` — background provider-model sync listener.
// - `principal` — resolves users + agent identities; shared.
// - `usage_tracking` — cross-domain event listener feeding budgets.
//
// Anything with a clear single owner belongs under `domains/<owner>/`. See
// `specs/domains.md` for the "shared services" rule.
//
// BACKWARDS COMPATIBILITY: the `services` module is `pub`, so downstream
// wrapper crates may have imported the now-moved modules from here. The
// `pub use ... as <old_name>` lines below re-export them under their old
// paths with `#[deprecated]` so existing imports still compile with a
// compiler warning. Drop these in the next breaking release.

pub mod capability;
pub mod event;
pub mod llm_resolver;
pub mod model_sync;
pub mod principal;
pub mod usage_tracking;

pub use capability::CapabilityService;
pub use event::EventService;
pub use llm_resolver::{LlmResolverService, ResolvedModel};
pub use model_sync::{ModelSyncService, SyncResult};
pub use principal::{PrincipalService, row_to_principal};
pub use usage_tracking::UsageTrackingListener;

// --- Deprecated re-exports of moved modules ---
// Each module moved to its single owning domain. Import from the new path;
// these aliases will be removed in a future release.

#[deprecated(
    since = "0.8.19",
    note = "moved to `crate::domains::evals::runner`; update your imports"
)]
pub use crate::domains::evals::runner as eval_runner;

#[deprecated(
    since = "0.8.19",
    note = "moved to `crate::domains::session_files::virtual_mount_registry`; update your imports"
)]
pub use crate::domains::session_files::virtual_mount_registry;

#[deprecated(
    since = "0.8.19",
    note = "moved to `crate::domains::mcp_servers::scoped_mcp`; update your imports"
)]
pub use crate::domains::mcp_servers::scoped_mcp;

#[deprecated(
    since = "0.8.19",
    note = "moved to `crate::domains::session_resources::leased_resource`; update your imports"
)]
pub use crate::domains::session_resources::leased_resource;

#[deprecated(
    since = "0.8.19",
    note = "moved to `crate::domains::capabilities::validation`; update your imports"
)]
pub use crate::domains::capabilities::validation as capability_validation;

#[deprecated(
    since = "0.8.19",
    note = "LeasedResourceService moved to `crate::domains::session_resources`"
)]
#[allow(deprecated)]
pub use leased_resource::LeasedResourceService;
