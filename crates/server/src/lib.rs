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

// Services layer
pub mod services;
pub use services::CapabilityService;
pub use services::EventService;

// Storage layer
pub mod storage;

// OpenAPI spec generation
pub mod openapi;

// Direct worker adapters for in-process task worker
pub mod direct_worker_adapters;
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

// Service seeding (default agents, providers, models)
pub mod seed;

// Session schedule poller
pub mod session_scheduler;

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
