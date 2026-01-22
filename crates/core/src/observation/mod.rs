// Observation Module
//
// This module contains observability backends that listen to events and generate
// telemetry data. Each backend implements the EventListener trait.
//
// Available backends:
// - `otel`: OpenTelemetry spans following gen-ai semantic conventions
//
// Utilities:
// - `openai_format`: Convert internal messages to OpenAI-compatible format

pub mod openai_format;
pub mod otel;

// Re-exports
pub use openai_format::{
    convert_content_to_openai_format, convert_message_to_openai_format,
    convert_messages_to_openai_format, convert_tool_calls_to_openai_format,
};
pub use otel::OtelEventListener;
