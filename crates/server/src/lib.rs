// Everruns Control Plane Library
// Decision: Shared library for binaries (API server, CLI tools)
// Decision: Pluggable auth backend for SaaS wrapper repos
// Decision: Server entrypoint extracted into run() for SaaS binary reuse

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

// Internal gRPC service for worker communication
pub mod grpc_service;

// Service seeding (default agents, providers, models)
pub mod seed;

// Server entrypoint (reusable by SaaS binary)
pub mod server;
pub use server::{ServerConfig, run};
