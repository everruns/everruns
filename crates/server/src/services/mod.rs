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
