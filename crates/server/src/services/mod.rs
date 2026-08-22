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
// - `provider_resolver` — spans `providers` + `models` + params; no single
//   owner.
// - `model_sync` — background provider-model sync listener.
// - `principal` — resolves users + agent identities; shared.
// - `usage_tracking` — cross-domain event listener feeding budgets.
//
// Anything with a clear single owner belongs under `domains/<owner>/`. See
// `knowledge/foundations/domains.md` for the "shared services" rule.

pub mod capability;
pub mod event;
pub mod generation_reconciler;
pub mod model_sync;
pub mod openrouter_generation;
pub mod org_feature_flags;
pub mod platform_command_surface;
pub mod principal;
pub mod provider_resolver;
pub mod run_summary;
pub mod usage_tracking;

pub use capability::CapabilityService;
pub use event::EventService;
pub use generation_reconciler::GenerationReconcilerService;
pub use model_sync::{ModelSyncService, SyncResult};
pub use principal::{PrincipalService, row_to_principal};
pub use provider_resolver::{ProviderResolverService, ResolvedModel};
pub use run_summary::RunSummaryService;
pub use usage_tracking::UsageTrackingListener;
