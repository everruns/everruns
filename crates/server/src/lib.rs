// Everruns Control Plane Library
// Decision: Shared library for binaries (API server, CLI tools)
// Decision: Pluggable auth backend for SaaS wrapper repos
// Decision: Server entrypoint extracted into run() for SaaS binary reuse
// Decision: App builder pattern for composable server configurations

// Force-link integration crates so inventory::submit! registrations are included
extern crate everruns_integrations_brave_search;
extern crate everruns_integrations_codesandbox;
extern crate everruns_integrations_daytona;
extern crate everruns_integrations_docker;

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

// Internal gRPC service for worker communication
pub mod grpc_service;

// Event retention background job
pub mod event_retention;

// Service seeding (default agents, providers, models)
pub mod seed;

// Session schedule poller
pub mod session_scheduler;

// Server entrypoint (reusable by SaaS binary)
pub mod server;
pub use server::{ServerConfig, run};

// App builder for composable server configurations
pub mod app_builder;
pub use app_builder::{ServerAppBuilder, ServerContext};
