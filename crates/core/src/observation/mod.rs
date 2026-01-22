// Observation Module
//
// This module contains observability backends that listen to events and generate
// telemetry data. Each backend implements the EventListener trait.
//
// Available backends:
// - `braintrust`: Braintrust tracing and logging
// - `otel`: OpenTelemetry spans following gen-ai semantic conventions
//
// OpenAI format conversion is handled via methods on domain types:
// - Message::to_openai_format()
// - ContentPart::to_openai_format()
// - ToolCall::to_openai_format()

pub mod braintrust;
pub mod otel;

// Re-exports
pub use braintrust::{BraintrustConfig, BraintrustListener};
pub use otel::OtelEventListener;
