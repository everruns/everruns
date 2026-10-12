// Everruns Control Plane Library
// Decision: Shared library for binaries (API server, CLI tools)
// Decision: Pluggable auth backend for SaaS wrapper repos
// Decision: App builder pattern (ServerAppBuilder) for composable server configurations
// Decision: `result_large_err` allowed crate-wide — ErrorResponse carries the
//   full RFC 9457 Problem Details payload (title/detail/code/allowed_actions/...)
//   and is only ever moved once per request, so the lint's stack-size concern
//   doesn't reflect real overhead.
#![allow(clippy::result_large_err)]

// Shared low-level security primitives (constant-time comparison, etc.)
pub mod security;

// Deterministic credential-format detection for text a person typed.
pub mod credential_shape;

// Layer-neutral shared DTOs (error body, list wrapper, pagination).
pub mod common_dto;

// API routes and types (shared for OpenAPI generation)
pub mod api;

// HTTP middleware (request ID, etc.)
pub mod middleware;

// Authentication module
pub mod auth;
pub use auth::{AuthBackend, BuiltinAuthBackend};

// Shared application errors
pub mod errors;

// Domain modules (feature-oriented: commands + queries + types)
pub mod domains;

// Persistence/API entities belong to the control plane.
pub mod records;

// Services layer
pub mod services;
pub use services::CapabilityService;
pub use services::EventService;

// Storage layer
pub mod storage;

// OpenAPI spec generation
mod oauth_client;
pub mod openapi;
pub mod platform;
pub use platform::{
    oss_built_in_harnesses, oss_connector_registry, oss_host_composition,
    oss_host_composition_for_grade,
};
pub mod harness_chain;
pub mod harnesses;

pub mod execution_metadata;
mod kernel_imports;
pub mod knowledge_store;

// ATIF trajectory interchange (knowledge/evaluation/atif-adoption.md)
pub mod atif;
pub use worker_link::direct_worker_adapters::DirectWorkerAdapters;
pub mod max_iterations;
pub mod metrics_names;
pub mod resource_links;

// Push delivery: event fan-out and task/event/notification broadcasters
pub mod live_updates;
pub use live_updates::event_delivery::EventDelivery;
pub use live_updates::event_notifications::EventNotificationBroadcaster;
pub use live_updates::notification_notifications::NotificationNotificationBroadcaster;
pub use live_updates::task_notifications::{TaskBroadcaster, TaskNotificationBroadcaster};

// Server<->worker link: internal gRPC service and in-process direct adapters
pub mod worker_link;

// Event listeners with no single owning domain: run summaries, turn latency,
// coordination thread turns
pub mod listeners;

// Background sweeps, retention/GC jobs, durable reaping, task supervision
pub mod background;

// Startup and bootstrap: storage bring-up, seeding, org initialization
pub mod setup;
// kept for saas; remove after adoption
pub use setup::org_init;

pub(crate) mod platform_chat_agent;

// Guided agent templates (agent examples with a setup path)
pub(crate) mod agent_templates;

// Server-owned session-scoped SQLite implementation.
pub mod session_sqldb;

// The server's turn requests on the `TurnBackend` entry point
pub mod turns;

// `--health-check` probe for distroless container healthchecks
pub mod health_probe;

// Server configuration and router helpers
pub mod server;
pub use server::ServerConfig;

// Valkey (Redis-compatible) client for distributed rate limiting
pub mod valkey;

// Per-agent GitHub Apps: manifest creation, installation, token minting
pub mod github_apps;

// Channel-specific plumbing (Slack: delivery, actions, events, install)
pub mod channels;

// App builder for composable server configurations
pub mod app_builder;
mod security_headers;

#[cfg(test)]
mod docs_catalog;
pub use app_builder::{ServerAppBuilder, ServerContext};

// Org creation policy extension point (EVE-607) — wrappers gate org creation
// before any DB write without forking the OSS handler. See `knowledge/foundations/embedding.md`.
pub use api::organizations::{OrgCreateContext, OrgCreatePolicy, OrgCreateRejection};
pub use setup::org_init::{
    OrgInitContext, OrgInitializer, OrgInitializerError, run_org_initializers,
};

#[cfg(test)]
mod tests {
    use std::collections::BTreeMap;
    use std::fs;
    use std::path::Path;

    #[test]
    fn migration_versions_are_unique() {
        let migrations_dir = Path::new(env!("CARGO_MANIFEST_DIR")).join("migrations");
        let mut files_by_version: BTreeMap<String, Vec<String>> = BTreeMap::new();

        for entry in fs::read_dir(&migrations_dir).expect("Failed to read migrations directory") {
            let path = entry.expect("Failed to read migration entry").path();
            if path.extension().and_then(|ext| ext.to_str()) != Some("sql") {
                continue;
            }

            let file_name = path
                .file_name()
                .and_then(|name| name.to_str())
                .expect("Migration file name must be valid UTF-8")
                .to_string();

            let (version, _) = file_name
                .split_once('_')
                .expect("Migration file names must start with '<version>_'");

            files_by_version
                .entry(version.to_string())
                .or_default()
                .push(file_name);
        }

        let duplicates: Vec<String> = files_by_version
            .into_iter()
            .filter_map(|(version, mut files)| {
                if files.len() <= 1 {
                    return None;
                }

                files.sort();
                Some(format!("{version}: {}", files.join(", ")))
            })
            .collect();

        assert!(
            duplicates.is_empty(),
            "Migration versions must be unique. Duplicates:\n{}",
            duplicates.join("\n")
        );
    }
}
