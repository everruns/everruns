//! `WorkerService` RPC handlers, grouped by the surface they serve.
//!
//! One gRPC trait impl cannot be split across modules, so `worker_service_impl`
//! keeps the `impl WorkerService for WorkerServiceImpl` block and does nothing but
//! forward each RPC to an inherent method defined here. Handlers live next to the
//! other handlers for their domain rather than in one 5000-line file.

pub(crate) mod support;

mod artifacts;
mod commands;
mod connections;
mod credentials;
mod durable;
mod events;
mod files;
mod leases;
mod messages;
mod notifications;
mod platform_agents;
mod platform_harnesses;
mod platform_meta;
mod platform_sessions;
mod policy;
mod resilience;
mod resources;
mod schedules;
mod sessions;
mod sqldb;
mod storage;
mod tasks;
