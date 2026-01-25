// Everruns Control Plane Library
// Decision: Shared library for binaries (API server, CLI tools)

// API routes and types (shared for OpenAPI generation)
pub mod api;

// Authentication module
pub mod auth;

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
