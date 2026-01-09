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
