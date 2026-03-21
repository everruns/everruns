// Everruns Control Plane Library
// Decision: Shared library for binaries (API server, CLI tools)
// Decision: Pluggable auth backend for SaaS wrapper repos
// Decision: App builder pattern (ServerAppBuilder) for composable server configurations

// Force-link integration crates so inventory::submit! registrations are included
extern crate everruns_integrations_brave_search;
extern crate everruns_integrations_browserless;
extern crate everruns_integrations_daytona;
extern crate everruns_integrations_docker;
extern crate everruns_integrations_duckduckgo;

// API routes and types (shared for OpenAPI generation)
pub mod api;

// Authentication module
pub mod auth;
pub use auth::{AuthBackend, BuiltinAuthBackend};

// Shared application errors
pub mod errors;

// Services layer
pub mod services;
pub use services::CapabilityService;
pub use services::EventService;

// Storage layer
pub mod storage;

// OpenAPI spec generation
pub mod openapi;
pub mod platform;
pub use platform::{
    oss_built_in_harnesses, oss_connection_provider_registry, oss_platform_definition,
    oss_platform_definition_for_grade,
};

// Direct worker adapters for in-process task worker
pub mod direct_worker_adapters;
pub mod execution_metadata;
pub use direct_worker_adapters::DirectWorkerAdapters;

// Task notification broadcaster for push-based notifications
pub mod task_notifications;
pub use task_notifications::TaskNotificationBroadcaster;

// Event notification broadcaster for push-based SSE delivery
pub mod event_notifications;
pub use event_notifications::EventNotificationBroadcaster;

// Notification broadcaster for push-based user inbox delivery
pub mod notification_notifications;
pub use notification_notifications::NotificationNotificationBroadcaster;

// Internal gRPC service for worker communication
pub mod grpc_service;

// Event retention background job
pub mod event_retention;

// Organization initialization (built-in harnesses, reconciliation)
pub mod org_init;

// Service seeding (default agents, providers, models)
pub mod seed;

// Session schedule poller
pub mod leased_resource_scheduler;
pub mod session_scheduler;

// Background sweep: time out sessions stuck in waiting_for_tool_results
pub mod tool_result_timeout;

// Server configuration and router helpers
pub mod server;
pub use server::ServerConfig;

// Valkey (Redis-compatible) client for distributed rate limiting
pub mod valkey;

// Slack delivery dispatcher for event-driven message posting
pub mod slack_delivery;

// App builder for composable server configurations
pub mod app_builder;
pub use app_builder::{ServerAppBuilder, ServerContext};

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
