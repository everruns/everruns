// Everruns Observability Exporters
//
// Event-listener backends that translate the agentic loop's events into
// external observability systems. These were moved out of `everruns-core`
// (EVE-651) so the core crate stays free of exporter-specific HTTP/telemetry
// dependencies and so a misconfigured exporter can never panic core's init
// path.
//
// Backends:
// - `braintrust`: Braintrust tracing and logging (always-on `reqwest`).
// - `otel`: OpenTelemetry spans following gen-AI semantic conventions. Emits
//   plain `tracing` spans; the OTLP exporter wiring lives in
//   `everruns_core::telemetry` behind core's `telemetry` feature.
//
// All backends implement `everruns_core::EventListener`.

pub mod braintrust;
pub mod otel;

pub use braintrust::{BraintrustConfig, BraintrustListener};
pub use otel::OtelEventListener;
