// Observability Integration for Everruns
//
// This crate provides extensible observability integrations for the agent loop.
// Key design decisions:
// - Uses event subscription pattern to keep agent-loop decoupled from observability
// - Supports multiple backends via the ObservabilityBackend trait
// - Langfuse integration via OpenTelemetry OTLP export
// - Feature-flagged backends to minimize dependencies

pub mod backend;
pub mod config;
pub mod emitter;

#[cfg(feature = "langfuse")]
pub mod langfuse;

// Re-exports
pub use backend::{ObservabilityBackend, ObservabilityEvent};
pub use config::ObservabilityConfig;
pub use emitter::ObservableEventEmitter;

#[cfg(feature = "langfuse")]
pub use langfuse::LangfuseBackend;
