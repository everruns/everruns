// Cross-cutting infrastructure modules.
//
// Domain-owned business logic lives under `crate::domains::*`, event listeners
// with no single owning domain under `crate::listeners`, and the worker command
// surface under `crate::worker_link`. The modules that remain here are infra
// adapters, validators, or shared helpers with no single owning domain:
//
// - `capability` — capability registry threaded through `Ctx`; used by every
//   domain.
// - `event` — event persistence + fanout listener; called by domains that
//   emit events.
// - `provider_resolver` — spans `providers` + `models` + params; no single
//   owner.
// - `org_feature_flags` — rollout-grade policy read by API, domains and
//   channels alike.
// - `standard_webhooks` — signing shared by outbound MCP Events
//   (`domains::mcp_servers::events`) and inbound MCP event triggers
//   (`domains::agent_triggers`).
// - `command_catalog` — transport-neutral domain-command catalog behind MCP,
//   the Platform capability and the worker shell.
//
// Anything with a clear single owner belongs under `domains/<owner>/`. See
// `knowledge/foundations/domains.md` for the "shared services" rule.

pub mod capability;
pub(crate) mod command_catalog;
pub mod event;
pub mod org_feature_flags;
pub mod provider_resolver;
pub mod standard_webhooks;

pub use capability::CapabilityService;
pub use event::EventService;
pub use provider_resolver::{ProviderResolverService, ResolvedModel};

// kept for saas; remove after adoption
pub use crate::domains::models::sync as model_sync;
