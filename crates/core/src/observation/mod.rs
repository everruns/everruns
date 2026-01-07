// Observation Module
//
// This module contains observability backends that listen to events and generate
// telemetry data. Each backend implements the EventListener trait.
//
// Available backends:
// - `otel`: OpenTelemetry spans following gen-ai semantic conventions

pub mod otel;

// Re-exports
pub use otel::OtelEventListener;
