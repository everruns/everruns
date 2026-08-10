// Everruns Observability
//
// Runtime/exporter implementations behind the neutral observability contracts
// in `everruns-core`. Core owns only the `EventListener` trait, event types,
// and the vendor-neutral gen-AI span conventions; this crate owns everything
// that talks to a vendor or wires up a subscriber (EVE-651, EVE-876):
//
// - `telemetry`: OpenTelemetry initialization — OTLP exporter wiring,
//   tracing-subscriber layers, `TelemetryConfig`/`TelemetryGuard`.
// - `composite`: `CompositeEventListener` fan-out with panic isolation.
// - `braintrust`: Braintrust tracing and logging (always-on `reqwest`).
// - `otel`: OpenTelemetry spans following gen-AI semantic conventions. Emits
//   plain `tracing` spans; the OTLP exporter wiring lives in `telemetry`.
//
// All listener backends implement `everruns_core::EventListener`.

pub mod braintrust;
pub mod composite;
pub mod otel;
pub mod telemetry;

pub use braintrust::{BraintrustConfig, BraintrustListener};
pub use composite::CompositeEventListener;
pub use otel::OtelEventListener;
pub use telemetry::{TelemetryConfig, TelemetryGuard, init_telemetry};
